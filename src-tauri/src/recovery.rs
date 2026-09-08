use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

use crate::{
    ipc::{ApiError, InternalReason},
    playback::PlaybackService,
    providers::ProviderService,
    radio::RadioService,
    schedule::SchedulerService,
    storage::{Storage, StorageError, StorageReason},
    understanding::UnderstandingService,
    weather::WeatherService,
};

const SUSPEND_DEADLINE: Duration = Duration::from_secs(2);
const GAP_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RESUME_GAP_THRESHOLD: Duration = Duration::from_secs(2);

type RecoveryFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait RecoveryBackend: Send + Sync {
    fn suspend(&self) -> RecoveryFuture<'_, Result<(), ApiError>>;
    fn resume(&self) -> RecoveryFuture<'_, Result<(), ApiError>>;
}

struct ApplicationRecoveryBackend {
    storage: Arc<Storage>,
    playback: PlaybackService,
    radio: RadioService,
    provider: Arc<ProviderService>,
    understanding: Arc<UnderstandingService>,
    weather: Arc<WeatherService>,
    scheduler: Arc<SchedulerService>,
}

impl RecoveryBackend for ApplicationRecoveryBackend {
    fn suspend(&self) -> RecoveryFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let network_operations = async {
                let (provider, understanding, ()) = tokio::join!(
                    self.provider.prepare_suspend(),
                    self.understanding.prepare_suspend(),
                    self.weather.prepare_suspend(),
                );
                provider?;
                understanding?;
                Ok::<(), ApiError>(())
            };
            let sound_operations = async {
                // Revoking system-session tokens must happen before cancelling
                // a radio speech task, otherwise its completion could resume a
                // stale Apple session during the power boundary.
                self.playback.prepare_suspend().await?;
                self.radio.prepare_suspend().await
            };
            let (network, sound) = tokio::join!(network_operations, sound_operations);
            network?;
            sound?;
            self.storage
                .passive_checkpoint()
                .await
                .map_err(|error| map_storage_error(&error))
        })
    }

    fn resume(&self) -> RecoveryFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            self.storage
                .verify_integrity()
                .await
                .map_err(|error| map_storage_error(&error))?;
            self.playback.resume_silent().await?;
            self.scheduler.reconcile_after_resume().await?;

            // Restoring admission performs no provider request and starts no
            // sound. Any paid/text/TTS retry still needs a fresh user command.
            self.weather.resume_after_suspend();
            self.understanding.resume_after_suspend();
            self.provider.resume_after_suspend();
            self.radio.resume_after_suspend();

            Ok(())
        })
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
            }),
            state: Mutex::new(PowerLifecycleState::Active),
            suspend_deadline: SUSPEND_DEADLINE,
        })
    }

    async fn prepare_suspend(&self) -> Result<(), ApiError> {
        let mut state = self.state.lock().await;
        if matches!(
            *state,
            PowerLifecycleState::Suspending | PowerLifecycleState::Suspended
        ) {
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
        *state = PowerLifecycleState::Resuming;
        let result = self.backend.resume().await;
        *state = if result.is_ok() {
            PowerLifecycleState::Active
        } else {
            PowerLifecycleState::Suspended
        };
        result
    }

    /// Handles both a real event-loop resume and the conservative clock-gap
    /// fallback. Calling suspend first makes this safe even when Windows did
    /// not expose a pre-suspend event through Tauri.
    pub(crate) async fn reconcile_after_resume(&self) -> Result<(), ApiError> {
        let suspend = self.prepare_suspend().await;
        let resume = self.resume().await;
        suspend.and(resume)
    }
}

pub(crate) struct RecoveryRuntime(tauri::async_runtime::JoinHandle<()>);

impl RecoveryRuntime {
    pub(crate) fn start(coordinator: Arc<RecoveryCoordinator>) -> Arc<Self> {
        Arc::new(Self(tauri::async_runtime::spawn(async move {
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
                    let _ = coordinator.reconcile_after_resume().await;
                    last_tick = Instant::now();
                }
            }
        })))
    }

    pub(crate) fn abort(&self) {
        self.0.abort();
    }

    pub(crate) fn is_finished(&self) -> bool {
        match &self.0 {
            tauri::async_runtime::JoinHandle::Tokio(handle) => handle.is_finished(),
        }
    }
}

impl Drop for RecoveryRuntime {
    fn drop(&mut self) {
        self.0.abort();
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::sync::Notify;

    use super::*;
    use crate::ipc::ErrorId;

    #[derive(Default)]
    struct FakeBackend {
        calls: StdMutex<Vec<&'static str>>,
        paid_calls: AtomicUsize,
        audible_calls: AtomicUsize,
        suspend_gate: Option<Arc<Notify>>,
    }

    impl RecoveryBackend for FakeBackend {
        fn suspend(&self) -> RecoveryFuture<'_, Result<(), ApiError>> {
            Box::pin(async move {
                self.calls.lock().expect("calls").push("suspend");
                if let Some(gate) = &self.suspend_gate {
                    gate.notified().await;
                }
                Ok(())
            })
        }

        fn resume(&self) -> RecoveryFuture<'_, Result<(), ApiError>> {
            Box::pin(async move {
                self.calls.lock().expect("calls").push("resume_silent");
                Ok(())
            })
        }
    }

    fn coordinator(backend: Arc<FakeBackend>, deadline: Duration) -> RecoveryCoordinator {
        RecoveryCoordinator {
            backend,
            state: Mutex::new(PowerLifecycleState::Active),
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
            ["suspend", "resume_silent"]
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
            ["suspend", "resume_silent"]
        );
        assert_eq!(backend.paid_calls.load(Ordering::Acquire), 0);
        assert_eq!(backend.audible_calls.load(Ordering::Acquire), 0);
    }
}
