use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex as TransitionLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use tauri::{Emitter, Runtime};
use tokio::sync::Mutex;

use crate::{
    ipc::{ApiError, EventEnvelope, InternalReason, ProcessSequence},
    playback::PlaybackService,
    providers::ProviderService,
    radio::RadioService,
    scanner::ScannerService,
    schedule::SchedulerService,
    storage::{Storage, StorageError, StorageReason},
    understanding::UnderstandingService,
    weather::WeatherService,
};
#[cfg(windows)]
use cyberkindred_windows_power_observer::{PowerEvent, PowerObserver};

const SUSPEND_DEADLINE: Duration = Duration::from_secs(2);
const SUSPEND_QUIESCE_DEADLINE: Duration = Duration::from_millis(1_500);
const GAP_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RESUME_GAP_THRESHOLD: Duration = Duration::from_secs(2);
pub(crate) const APP_RESUMED_EVENT: &str = "cyberkindred://v1/app/resumed";

type RecoveryFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct ResumeHint {
    slept_at: Option<String>,
    occurred_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppResumedEvent {
    #[serde(flatten)]
    envelope: EventEnvelope,
    slept_at: Option<String>,
}

pub(crate) trait RecoveryEventSink: Send + Sync {
    fn resumed(&self, slept_at: Option<String>, occurred_at: String) -> Result<(), ApiError>;
}

pub(crate) struct TauriRecoveryEventSink<R: Runtime> {
    app: tauri::AppHandle<R>,
    sequence: Arc<ProcessSequence>,
}

impl<R: Runtime> TauriRecoveryEventSink<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>, sequence: Arc<ProcessSequence>) -> Self {
        Self { app, sequence }
    }
}

impl<R: Runtime> RecoveryEventSink for TauriRecoveryEventSink<R> {
    fn resumed(&self, slept_at: Option<String>, occurred_at: String) -> Result<(), ApiError> {
        let mut envelope = EventEnvelope::now(self.sequence.next()?);
        envelope.occurred_at = occurred_at;
        envelope.validate()?;
        self.app
            .emit(APP_RESUMED_EVENT, AppResumedEvent { envelope, slept_at })
            .map_err(|_| ApiError::unexpected())
    }
}

trait RecoveryBackend: Send + Sync {
    fn begin_suspend(&self);
    fn suspend(&self) -> RecoveryFuture<'_, Result<(), ApiError>>;
    fn reconcile_resume(&self) -> RecoveryFuture<'_, Result<ResumeHint, ApiError>>;
    fn finish_resume(&self, hint: ResumeHint);
}

struct ApplicationRecoveryBackend {
    storage: Arc<Storage>,
    playback: PlaybackService,
    radio: RadioService,
    provider: Arc<ProviderService>,
    understanding: Arc<UnderstandingService>,
    weather: Arc<WeatherService>,
    scheduler: Arc<SchedulerService>,
    scanner: Arc<ScannerService>,
    events: Arc<dyn RecoveryEventSink>,
}

impl RecoveryBackend for ApplicationRecoveryBackend {
    fn begin_suspend(&self) {
        // Close every paid or sound-capable admission boundary synchronously
        // while Windows is still delivering PBT_APMSUSPEND. The bounded async
        // phase below performs durable cancellation, silence, and checkpoint.
        self.playback.begin_suspend();
        self.radio.begin_suspend();
        self.provider.begin_suspend();
        self.understanding.begin_suspend();
        self.weather.begin_suspend();
        let _ = self.scanner.begin_suspend();
    }

    fn suspend(&self) -> RecoveryFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let observed_at_ms = Utc::now().timestamp_millis();
            let repository = self.storage.repository();
            let (suspend_record, quiesce) = tokio::join!(
                repository.record_power_suspend(observed_at_ms),
                tokio::time::timeout(SUSPEND_QUIESCE_DEADLINE, async {
                    tokio::join!(
                        self.provider.prepare_suspend(),
                        self.understanding.prepare_suspend(),
                        self.weather.prepare_suspend(),
                        self.playback.prepare_suspend(),
                        self.radio.prepare_suspend(),
                        self.scanner.prepare_suspend(),
                    )
                })
            );
            // Checkpoint is attempted even when one cancellation path fails or
            // exhausts its share of the two-second Windows suspend budget.
            let checkpoint = self
                .storage
                .passive_checkpoint()
                .await
                .map_err(|error| map_storage_error(&error));
            let (provider, understanding, (), playback, radio, scanner) =
                quiesce.map_err(|_| ApiError::from_reason(InternalReason::ResourceBusy))?;
            suspend_record.map_err(|error| map_storage_error(&error))?;
            provider?;
            understanding?;
            playback?;
            radio?;
            scanner?;
            checkpoint
        })
    }

    fn reconcile_resume(&self) -> RecoveryFuture<'_, Result<ResumeHint, ApiError>> {
        Box::pin(async move {
            let resumed_at_ms = Utc::now().timestamp_millis();
            self.storage
                .verify_integrity()
                .await
                .map_err(|error| map_storage_error(&error))?;
            self.playback.resume_silent().await?;
            self.scheduler.reconcile_after_resume().await?;
            let slept_at_ms = self
                .storage
                .repository()
                .record_power_resume(resumed_at_ms)
                .await
                .map_err(|error| map_storage_error(&error))?;
            Ok(ResumeHint {
                slept_at: slept_at_ms.map(format_timestamp).transpose()?,
                occurred_at: format_timestamp(resumed_at_ms)?,
            })
        })
    }

    fn finish_resume(&self, hint: ResumeHint) {
        // Restoring admission performs no provider request and starts no
        // sound. Any paid/text/TTS retry still needs a fresh user command.
        self.playback.resume_after_suspend();
        self.weather.resume_after_suspend();
        self.understanding.resume_after_suspend();
        self.provider.resume_after_suspend();
        self.radio.resume_after_suspend();
        self.scanner.resume_after_suspend();

        // EVT-010 is a process-local hint. Reconciliation and admission
        // reopening are authoritative and must not be rolled back when a
        // window is closing or has no active event listener.
        publish_resume_hint(self.events.as_ref(), hint.slept_at, hint.occurred_at);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PowerLifecycleState {
    Active,
    Suspending,
    Suspended,
    Resuming,
}

/// Serializes startup, sleep, and clock-gap recovery. No method can create a
/// provider request or authorize sound; it only cancels, checkpoints, refreshes
/// local/OS state, and restores command admission.
pub(crate) struct RecoveryCoordinator {
    backend: Arc<dyn RecoveryBackend>,
    state: Mutex<PowerLifecycleState>,
    admission_transition: TransitionLock<()>,
    suspend_pending: AtomicBool,
    suspend_deadline: Duration,
}

impl RecoveryCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        storage: Arc<Storage>,
        playback: PlaybackService,
        radio: RadioService,
        provider: Arc<ProviderService>,
        understanding: Arc<UnderstandingService>,
        weather: Arc<WeatherService>,
        scheduler: Arc<SchedulerService>,
        scanner: Arc<ScannerService>,
        events: Arc<dyn RecoveryEventSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend: Arc::new(ApplicationRecoveryBackend {
                storage,
                playback,
                radio,
                provider,
                understanding,
                weather,
                scheduler,
                scanner,
                events,
            }),
            state: Mutex::new(PowerLifecycleState::Active),
            admission_transition: TransitionLock::new(()),
            suspend_pending: AtomicBool::new(false),
            suspend_deadline: SUSPEND_DEADLINE,
        })
    }

    fn begin_suspend(&self) {
        let _transition = self
            .admission_transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.suspend_pending.store(true, Ordering::Release);
        self.backend.begin_suspend();
    }

    async fn prepare_suspend(&self) -> Result<(), ApiError> {
        self.begin_suspend();
        self.finish_prepare_suspend().await
    }

    async fn finish_prepare_suspend(&self) -> Result<(), ApiError> {
        let mut state = self.state.lock().await;
        if !self.suspend_pending.swap(false, Ordering::AcqRel)
            && matches!(
                *state,
                PowerLifecycleState::Suspending | PowerLifecycleState::Suspended
            )
        {
            return Ok(());
        }
        *state = PowerLifecycleState::Suspending;
        let result = tokio::time::timeout(self.suspend_deadline, self.backend.suspend()).await;
        *state = PowerLifecycleState::Suspended;
        match result {
            Ok(result) => result,
            Err(_) => Err(ApiError::from_reason(InternalReason::ResourceBusy)),
        }
    }

    async fn resume(&self) -> Result<(), ApiError> {
        let mut state = self.state.lock().await;
        if matches!(*state, PowerLifecycleState::Active)
            && !self.suspend_pending.load(Ordering::Acquire)
        {
            return Ok(());
        }
        *state = PowerLifecycleState::Resuming;
        let hint = match self.backend.reconcile_resume().await {
            Ok(hint) => hint,
            Err(error) => {
                *state = PowerLifecycleState::Suspended;
                return Err(error);
            }
        };
        let _transition = self
            .admission_transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.suspend_pending.load(Ordering::Acquire) {
            *state = PowerLifecycleState::Suspended;
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        self.backend.finish_resume(hint);
        *state = PowerLifecycleState::Active;
        Ok(())
    }

    /// Handles the conservative clock-gap/non-Windows fallback. Calling
    /// suspend first keeps the fallback fail-closed when no pre-suspend edge
    /// was observed.
    pub(crate) async fn reconcile_after_resume(&self) -> Result<(), ApiError> {
        let suspend = self.prepare_suspend().await;
        let resume = self.resume().await;
        suspend.and(resume)
    }
}

#[derive(Debug)]
pub(crate) struct RecoveryRuntimeError;

impl std::fmt::Display for RecoveryRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Windows power lifecycle observer unavailable")
    }
}

impl std::error::Error for RecoveryRuntimeError {}

pub(crate) struct RecoveryRuntime {
    watchdog: tauri::async_runtime::JoinHandle<()>,
    #[cfg(windows)]
    power_observer: TransitionLock<Option<PowerObserver>>,
}

impl RecoveryRuntime {
    pub(crate) fn start(
        coordinator: &Arc<RecoveryCoordinator>,
    ) -> Result<Arc<Self>, RecoveryRuntimeError> {
        #[cfg(windows)]
        let observer_coordinator = Arc::clone(coordinator);
        #[cfg(windows)]
        let power_observer = PowerObserver::start(move |event| match event {
            PowerEvent::Suspend => {
                // Windows waits for WM_POWERBROADCAST handlers before entering
                // sleep. Keep this dedicated window thread inside the approved
                // two-second cleanup budget so the callback cannot return while
                // sound/provider admission remains open.
                observer_coordinator.begin_suspend();
                let _ =
                    tauri::async_runtime::block_on(observer_coordinator.finish_prepare_suspend());
            }
            PowerEvent::Resume => {
                let coordinator = Arc::clone(&observer_coordinator);
                tauri::async_runtime::spawn(async move {
                    let _ = coordinator.resume().await;
                });
            }
        })
        .map_err(|_| RecoveryRuntimeError)?;
        let watchdog_coordinator = Arc::clone(coordinator);
        let watchdog = tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(GAP_POLL_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            let mut last_tick = Instant::now();
            loop {
                interval.tick().await;
                let now = Instant::now();
                let gap = now.saturating_duration_since(last_tick);
                last_tick = now;
                if gap >= RESUME_GAP_THRESHOLD {
                    let _ = watchdog_coordinator.reconcile_after_resume().await;
                    last_tick = Instant::now();
                }
            }
        });
        Ok(Arc::new(Self {
            watchdog,
            #[cfg(windows)]
            power_observer: TransitionLock::new(Some(power_observer)),
        }))
    }

    pub(crate) fn abort(&self) {
        self.watchdog.abort();
        #[cfg(windows)]
        if let Ok(mut observer) = self.power_observer.lock() {
            drop(observer.take());
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        match &self.watchdog {
            tauri::async_runtime::JoinHandle::Tokio(handle) => handle.is_finished(),
        }
    }
}

impl Drop for RecoveryRuntime {
    fn drop(&mut self) {
        self.watchdog.abort();
        #[cfg(windows)]
        if let Ok(observer) = self.power_observer.get_mut() {
            drop(observer.take());
        }
    }
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
        StorageReason::StorageWriteFailed
        | StorageReason::InvalidSetting
        | StorageReason::EntityNotFound
        | StorageReason::RevisionConflict
        | StorageReason::PathDenied
        | StorageReason::PathOutsideScope
        | StorageReason::UnsafeReparsePoint => InternalReason::StorageWriteFailed,
    };
    ApiError::from_reason(reason)
}

fn format_timestamp(value: i64) -> Result<String, ApiError> {
    Utc.timestamp_millis_opt(value)
        .single()
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(ApiError::unexpected)
}

fn publish_resume_hint(
    events: &dyn RecoveryEventSink,
    slept_at: Option<String>,
    occurred_at: String,
) {
    let _ = events.resumed(slept_at, occurred_at);
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::sync::Notify;

    use super::*;
    use crate::ipc::ErrorId;

    struct FailingEventSink;

    impl RecoveryEventSink for FailingEventSink {
        fn resumed(&self, _slept_at: Option<String>, _occurred_at: String) -> Result<(), ApiError> {
            Err(ApiError::unexpected())
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        calls: StdMutex<Vec<&'static str>>,
        paid_calls: AtomicUsize,
        audible_calls: AtomicUsize,
        admission_closed: std::sync::atomic::AtomicBool,
        suspend_gate: Option<Arc<Notify>>,
        resume_gate: Option<Arc<Notify>>,
    }

    impl RecoveryBackend for FakeBackend {
        fn begin_suspend(&self) {
            self.admission_closed.store(true, Ordering::Release);
            self.calls.lock().expect("calls").push("begin_suspend");
        }

        fn suspend(&self) -> RecoveryFuture<'_, Result<(), ApiError>> {
            Box::pin(async move {
                self.calls.lock().expect("calls").push("suspend");
                if let Some(gate) = &self.suspend_gate {
                    gate.notified().await;
                }
                Ok(())
            })
        }

        fn reconcile_resume(&self) -> RecoveryFuture<'_, Result<ResumeHint, ApiError>> {
            Box::pin(async move {
                self.calls.lock().expect("calls").push("resume_silent");
                if let Some(gate) = &self.resume_gate {
                    gate.notified().await;
                }
                Ok(ResumeHint {
                    slept_at: None,
                    occurred_at: "2026-09-08T10:00:00.000Z".to_owned(),
                })
            })
        }

        fn finish_resume(&self, _hint: ResumeHint) {
            self.calls.lock().expect("calls").push("finish_resume");
            self.admission_closed.store(false, Ordering::Release);
        }
    }

    fn coordinator(backend: Arc<FakeBackend>, deadline: Duration) -> RecoveryCoordinator {
        RecoveryCoordinator {
            backend,
            state: Mutex::new(PowerLifecycleState::Active),
            admission_transition: TransitionLock::new(()),
            suspend_pending: AtomicBool::new(false),
            suspend_deadline: deadline,
        }
    }

    #[tokio::test]
    async fn offline_recovery_resume_orders_silence_before_reconciliation_without_paid_calls() {
        let backend = Arc::new(FakeBackend::default());
        let coordinator = coordinator(Arc::clone(&backend), Duration::from_secs(1));
        coordinator
            .reconcile_after_resume()
            .await
            .expect("recovery");
        assert_eq!(
            backend.calls.lock().expect("calls").as_slice(),
            ["begin_suspend", "suspend", "resume_silent", "finish_resume"]
        );
        assert_eq!(backend.paid_calls.load(Ordering::Acquire), 0);
        assert_eq!(backend.audible_calls.load(Ordering::Acquire), 0);
        assert_eq!(*coordinator.state.lock().await, PowerLifecycleState::Active);
    }

    #[tokio::test]
    async fn recovery_suspend_timeout_is_bounded_and_resume_remains_silent() {
        let backend = Arc::new(FakeBackend {
            suspend_gate: Some(Arc::new(Notify::new())),
            ..FakeBackend::default()
        });
        let coordinator = coordinator(Arc::clone(&backend), Duration::from_millis(5));
        let error = coordinator
            .reconcile_after_resume()
            .await
            .expect_err("timeout is surfaced");
        assert_eq!(error.error_id, ErrorId::ResourceBusy);
        assert_eq!(
            backend.calls.lock().expect("calls").as_slice(),
            ["begin_suspend", "suspend", "resume_silent", "finish_resume"]
        );
        assert_eq!(backend.paid_calls.load(Ordering::Acquire), 0);
        assert_eq!(backend.audible_calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn resume_keeps_admission_closed_until_reconciliation_finishes() {
        let resume_gate = Arc::new(Notify::new());
        let backend = Arc::new(FakeBackend {
            resume_gate: Some(Arc::clone(&resume_gate)),
            ..FakeBackend::default()
        });
        let coordinator = Arc::new(coordinator(Arc::clone(&backend), Duration::from_secs(1)));
        coordinator.prepare_suspend().await.expect("suspend");
        assert!(backend.admission_closed.load(Ordering::Acquire));

        let resume = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.resume().await })
        };
        tokio::task::yield_now().await;
        assert!(backend.admission_closed.load(Ordering::Acquire));
        resume_gate.notify_one();
        resume.await.expect("resume task").expect("resume");
        assert!(!backend.admission_closed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn rapid_suspend_during_resume_never_reopens_admission() {
        let resume_gate = Arc::new(Notify::new());
        let backend = Arc::new(FakeBackend {
            resume_gate: Some(Arc::clone(&resume_gate)),
            ..FakeBackend::default()
        });
        let coordinator = Arc::new(coordinator(Arc::clone(&backend), Duration::from_secs(1)));
        coordinator
            .prepare_suspend()
            .await
            .expect("initial suspend");

        let resume = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.resume().await })
        };
        tokio::task::yield_now().await;
        coordinator.begin_suspend();
        assert!(backend.admission_closed.load(Ordering::Acquire));
        resume_gate.notify_one();
        let error = resume
            .await
            .expect("resume task")
            .expect_err("pending suspend aborts resume");
        assert_eq!(error.error_id, ErrorId::ResourceBusy);
        assert!(backend.admission_closed.load(Ordering::Acquire));
        assert!(
            !backend
                .calls
                .lock()
                .expect("calls")
                .contains(&"finish_resume")
        );

        coordinator
            .finish_prepare_suspend()
            .await
            .expect("pending suspend quiesces");
        assert!(backend.admission_closed.load(Ordering::Acquire));
        assert_eq!(
            *coordinator.state.lock().await,
            PowerLifecycleState::Suspended
        );
    }

    #[test]
    fn resume_event_payload_matches_evt_010_and_transport_failure_is_best_effort() {
        let payload = AppResumedEvent {
            envelope: EventEnvelope {
                schema_version: "1.0.0".to_owned(),
                sequence: 7,
                occurred_at: "2026-09-08T10:00:00.000Z".to_owned(),
            },
            slept_at: Some("2026-09-08T09:59:00.000Z".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(payload).expect("serialize"),
            serde_json::json!({
                "schemaVersion": "1.0.0",
                "sequence": 7,
                "occurredAt": "2026-09-08T10:00:00.000Z",
                "sleptAt": "2026-09-08T09:59:00.000Z"
            })
        );
        publish_resume_hint(
            &FailingEventSink,
            None,
            "2026-09-08T10:00:00.000Z".to_owned(),
        );
    }
}
