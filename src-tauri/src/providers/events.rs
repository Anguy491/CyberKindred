//! Strict Tauri event adapter for terminal voice-preview operations.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{Clock, VoicePreviewEventSink, VoicePreviewTerminal};
use crate::{
    ipc::{ApiError, EventEnvelope, InternalReason, ProcessSequence},
    storage::{OperationTerminalRecord, Repository, StorageError, StorageReason},
};

pub const OPERATION_COMPLETED_EVENT: &str = "cyberkindred://v1/operation/completed";
pub const OPERATION_FAILED_EVENT: &str = "cyberkindred://v1/operation/failed";
const STARTUP_RECOVERY_BATCH: u32 = 100;
const MAX_STARTUP_RECOVERY_OPERATIONS: u64 = 1_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CompletedOperationKind {
    VoicePreview,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationCompletedPayload {
    schema_version: String,
    sequence: u64,
    occurred_at: String,
    operation_id: Uuid,
    kind: CompletedOperationKind,
    output_label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationFailedPayload {
    schema_version: String,
    sequence: u64,
    occurred_at: String,
    operation_id: Uuid,
    error: ApiError,
}

enum PreviewTerminalEvent {
    Completed(OperationCompletedPayload),
    Failed(OperationFailedPayload),
}

fn map_terminal_event(
    sequence: &ProcessSequence,
    terminal: VoicePreviewTerminal,
) -> Result<PreviewTerminalEvent, ApiError> {
    let mut envelope = EventEnvelope::now(sequence.next()?);
    envelope.occurred_at = terminal.occurred_at;
    envelope.validate()?;

    Ok(match terminal.error {
        None => PreviewTerminalEvent::Completed(OperationCompletedPayload {
            schema_version: envelope.schema_version,
            sequence: envelope.sequence,
            occurred_at: envelope.occurred_at,
            operation_id: terminal.operation_id,
            kind: CompletedOperationKind::VoicePreview,
            output_label: None,
        }),
        Some(error) => PreviewTerminalEvent::Failed(OperationFailedPayload {
            schema_version: envelope.schema_version,
            sequence: envelope.sequence,
            occurred_at: envelope.occurred_at,
            operation_id: terminal.operation_id,
            error,
        }),
    })
}

/// Publishes exact EVT-008/EVT-009 payloads through the process-local Tauri bus.
///
/// The shared sequence must also be used by every other public event adapter so
/// subscribers can detect gaps across event kinds.
pub struct TauriVoicePreviewEventSink<R: Runtime> {
    app_handle: AppHandle<R>,
    sequence: Arc<ProcessSequence>,
}

impl<R: Runtime> TauriVoicePreviewEventSink<R> {
    #[must_use]
    pub fn new(app_handle: AppHandle<R>, sequence: Arc<ProcessSequence>) -> Self {
        Self {
            app_handle,
            sequence,
        }
    }
}

impl<R: Runtime> VoicePreviewEventSink for TauriVoicePreviewEventSink<R> {
    fn publish(&self, terminal: VoicePreviewTerminal) -> Result<(), ApiError> {
        let event = map_terminal_event(self.sequence.as_ref(), terminal)?;

        match event {
            PreviewTerminalEvent::Completed(payload) => self
                .app_handle
                .emit(OPERATION_COMPLETED_EVENT, payload)
                .map_err(|_| ApiError::unexpected()),
            PreviewTerminalEvent::Failed(payload) => self
                .app_handle
                .emit(OPERATION_FAILED_EVENT, payload)
                .map_err(|_| ApiError::unexpected()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StartupDrainReport {
    pub recovered_accepted: u32,
    pub loaded: u32,
    pub delivered: u32,
}

#[derive(Clone)]
enum StartupRecoveryState {
    Pending,
    Complete(StartupDrainReport),
    Failed(ApiError),
}

/// Process-scoped gate that resolves the durable outbox before this process
/// accepts its first new voice-preview operation.
///
/// The command that trips this gate is necessarily invoked by a loaded
/// frontend, so the event listeners can be installed before recovery begins.
/// Successful and failed results are cached for the lifetime of the process;
/// this prevents a mark-delivered failure from causing another emit in the same
/// frontend lifecycle. Across a process crash between emit and mark, delivery
/// is intentionally at-least-once; the database terminal remains exactly one.
pub struct StartupVoicePreviewOutboxRecovery {
    repository: Repository,
    event_sink: Arc<dyn VoicePreviewEventSink>,
    clock: Arc<dyn Clock>,
    state: Mutex<StartupRecoveryState>,
}

impl StartupVoicePreviewOutboxRecovery {
    #[must_use]
    pub fn new(
        repository: Repository,
        event_sink: Arc<dyn VoicePreviewEventSink>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            event_sink,
            clock,
            state: Mutex::new(StartupRecoveryState::Pending),
        }
    }

    /// Completes the process's single bounded recovery attempt.
    ///
    /// # Errors
    ///
    /// Returns the same stable, redacted error after a failed attempt. The
    /// caller must not accept a new preview while recovery is unresolved.
    pub async fn ensure_recovered(&self) -> Result<StartupDrainReport, ApiError> {
        let mut state = self.state.lock().await;
        match &*state {
            StartupRecoveryState::Complete(report) => return Ok(*report),
            StartupRecoveryState::Failed(error) => return Err(error.clone()),
            StartupRecoveryState::Pending => {}
        }

        let result = self.recover_all().await;
        *state = match &result {
            Ok(report) => StartupRecoveryState::Complete(*report),
            Err(error) => StartupRecoveryState::Failed(error.clone()),
        };
        result
    }

    async fn recover_all(&self) -> Result<StartupDrainReport, ApiError> {
        let initial_work = self
            .repository
            .count_voice_preview_recovery_work()
            .await
            .map_err(|error| map_outbox_storage_error(&error))?;
        if initial_work > MAX_STARTUP_RECOVERY_OPERATIONS {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }

        let recovered_at = self.clock.now_rfc3339();
        let mut recovered_accepted = 0_u32;
        loop {
            let recovered = self
                .repository
                .recover_abandoned_voice_previews(STARTUP_RECOVERY_BATCH, &recovered_at)
                .await
                .map_err(|error| map_outbox_storage_error(&error))?;
            recovered_accepted = recovered_accepted
                .checked_add(recovered)
                .ok_or_else(ApiError::unexpected)?;
            if recovered == 0 {
                break;
            }
        }

        let mut loaded = 0_u32;
        let mut delivered = 0_u32;
        loop {
            let pending = self
                .repository
                .load_pending_voice_preview_terminals(STARTUP_RECOVERY_BATCH)
                .await
                .map_err(|error| map_outbox_storage_error(&error))?;
            if pending.is_empty() {
                break;
            }
            loaded = loaded
                .checked_add(u32::try_from(pending.len()).map_err(|_| ApiError::unexpected())?)
                .ok_or_else(ApiError::unexpected)?;
            for record in pending {
                // The sink assigns the current process sequence at emit time;
                // no sequence is stored and replayed across restarts.
                self.event_sink.publish(record_to_terminal(&record))?;
                let marked = self
                    .repository
                    .mark_voice_preview_terminal_delivered(
                        record.outbox_id,
                        record.operation_id,
                        self.clock.now_ms(),
                    )
                    .await
                    .map_err(|error| map_outbox_storage_error(&error))?;
                if !marked {
                    return Err(ApiError::from_reason(InternalReason::RevisionConflict));
                }
                delivered = delivered.checked_add(1).ok_or_else(ApiError::unexpected)?;
            }
        }

        let remaining = self
            .repository
            .count_voice_preview_recovery_work()
            .await
            .map_err(|error| map_outbox_storage_error(&error))?;
        if remaining != 0 {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        Ok(StartupDrainReport {
            recovered_accepted,
            loaded,
            delivered,
        })
    }
}

fn record_to_terminal(record: &OperationTerminalRecord) -> VoicePreviewTerminal {
    VoicePreviewTerminal {
        operation_id: record.operation_id,
        occurred_at: record.occurred_at.clone(),
        error: record.error.clone(),
    }
}

fn map_outbox_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::InvalidSetting => InternalReason::RequestInvalid,
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::StorageWriteFailed => InternalReason::StorageWriteFailed,
    };
    ApiError::from_reason(reason)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    };

    use chrono::{DateTime, Utc};
    use serde_json::{Value, json};

    use super::*;
    use crate::ipc::{ErrorId, InternalReason};
    use crate::storage::{AppPaths, Storage};

    const OCCURRED_AT: &str = "2026-09-02T00:00:00.000Z";

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            DateTime::parse_from_rfc3339("2026-09-02T03:04:05.000Z")
                .expect("fixed clock")
                .with_timezone(&Utc)
        }
    }

    #[test]
    fn voice_preview_completed_payload_is_exact_evt_008() {
        let operation_id = Uuid::parse_str("018f1f64-4ca0-7a2a-8e91-e89c389b3a31")
            .expect("fixed UUID must be valid");
        let event = map_terminal_event(
            &ProcessSequence::default(),
            VoicePreviewTerminal {
                operation_id,
                occurred_at: OCCURRED_AT.to_owned(),
                error: None,
            },
        )
        .expect("fixed terminal must map");
        let PreviewTerminalEvent::Completed(payload) = event else {
            panic!("successful preview must map to EVT-008");
        };

        let actual = serde_json::to_value(&payload).expect("payload must serialize");
        assert_eq!(
            actual,
            json!({
                "schemaVersion": "1.0.0",
                "sequence": 1,
                "occurredAt": OCCURRED_AT,
                "operationId": operation_id,
                "kind": "voice_preview",
                "outputLabel": null
            })
        );
    }

    #[test]
    fn voice_preview_failure_payload_is_exact_evt_009_and_redacted() {
        let operation_id = Uuid::parse_str("018f1f64-4ca0-7a2a-8e91-e89c389b3a32")
            .expect("fixed UUID must be valid");
        let error = ApiError::from_reason(InternalReason::ProviderAuthentication);
        let event = map_terminal_event(
            &ProcessSequence::default(),
            VoicePreviewTerminal {
                operation_id,
                occurred_at: OCCURRED_AT.to_owned(),
                error: Some(error),
            },
        )
        .expect("fixed terminal must map");
        let PreviewTerminalEvent::Failed(payload) = event else {
            panic!("failed preview must map to EVT-009");
        };

        let actual = serde_json::to_value(&payload).expect("payload must serialize");
        assert_eq!(actual["schemaVersion"], "1.0.0");
        assert_eq!(actual["sequence"], 1);
        assert_eq!(actual["occurredAt"], OCCURRED_AT);
        assert_eq!(actual["operationId"], operation_id.to_string());
        assert_eq!(actual["error"]["errorId"], "ERR-1301");
        assert_eq!(
            actual["error"]["details"]["reason"],
            "provider_authentication"
        );
        assert_eq!(actual.as_object().map(serde_json::Map::len), Some(5));

        let serialized = serde_json::to_string(&actual).expect("value must serialize");
        assert!(!serialized.contains("sk-canary"));
        assert!(!serialized.contains("C:\\\\Users\\\\canary"));
        assert_eq!(payload.error.error_id, ErrorId::ProviderAuthentication);
    }

    #[test]
    fn preview_events_share_the_process_sequence_and_reject_invalid_time() {
        let sequence = ProcessSequence::default();
        let first = map_terminal_event(
            &sequence,
            VoicePreviewTerminal {
                operation_id: Uuid::now_v7(),
                occurred_at: OCCURRED_AT.to_owned(),
                error: None,
            },
        )
        .expect("first event must map");
        let second = map_terminal_event(
            &sequence,
            VoicePreviewTerminal {
                operation_id: Uuid::now_v7(),
                occurred_at: OCCURRED_AT.to_owned(),
                error: Some(ApiError::unexpected()),
            },
        )
        .expect("second event must map");
        let invalid = map_terminal_event(
            &sequence,
            VoicePreviewTerminal {
                operation_id: Uuid::now_v7(),
                occurred_at: "not-a-time".to_owned(),
                error: None,
            },
        );

        let PreviewTerminalEvent::Completed(first) = first else {
            panic!("first event must be completed");
        };
        let PreviewTerminalEvent::Failed(second) = second else {
            panic!("second event must be failed");
        };
        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert!(invalid.is_err());
    }

    #[test]
    fn evt_009_payload_rejects_unknown_fields_when_deserialized() {
        let error = ApiError::from_reason(InternalReason::ProviderUnavailable);
        let mut value = json!({
            "schemaVersion": "1.0.0",
            "sequence": 1,
            "occurredAt": OCCURRED_AT,
            "operationId": Uuid::now_v7(),
            "error": error
        });
        value
            .as_object_mut()
            .expect("payload fixture must be an object")
            .insert("secret".to_owned(), Value::String("sk-canary".to_owned()));

        assert!(serde_json::from_value::<OperationFailedPayload>(value).is_err());
    }

    #[derive(Default)]
    struct RecordingSink {
        events: StdMutex<Vec<VoicePreviewTerminal>>,
        fail: AtomicBool,
    }

    impl VoicePreviewEventSink for RecordingSink {
        fn publish(&self, terminal: VoicePreviewTerminal) -> Result<(), ApiError> {
            if self.fail.load(Ordering::Acquire) {
                return Err(ApiError::unexpected());
            }
            self.events
                .lock()
                .map_err(|_| ApiError::unexpected())?
                .push(terminal);
            Ok(())
        }
    }

    async fn outbox_fixture() -> (tempfile::TempDir, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped paths");
        let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
        let repository = storage.repository();
        (temp, storage, repository)
    }

    async fn accept(repository: &Repository, operation_id: Uuid) {
        repository
            .persist_voice_preview_accepted(operation_id, OCCURRED_AT.to_owned())
            .await
            .expect("accepted operation");
    }

    #[tokio::test]
    async fn startup_recovery_converts_abandoned_accept_to_safe_failure() {
        let (_temp, storage, repository) = outbox_fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let sink = Arc::new(RecordingSink::default());
        let recovery = StartupVoicePreviewOutboxRecovery::new(
            repository.clone(),
            sink.clone() as Arc<dyn VoicePreviewEventSink>,
            Arc::new(FixedClock),
        );
        let report = recovery.ensure_recovered().await.expect("startup recovery");
        assert_eq!(
            report,
            StartupDrainReport {
                recovered_accepted: 1,
                loaded: 1,
                delivered: 1,
            }
        );
        {
            let events = sink.events.lock().expect("events");
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].operation_id, operation_id);
            assert_eq!(
                events[0].error.as_ref().map(|error| error.error_id),
                Some(ErrorId::UnexpectedInternal)
            );
        }
        assert_eq!(
            repository
                .count_voice_preview_recovery_work()
                .await
                .expect("no recovery tail"),
            0
        );
        storage.close().await;
    }

    #[tokio::test]
    async fn startup_recovery_caches_success_without_reemitting() {
        let (_temp, storage, repository) = outbox_fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        repository
            .persist_voice_preview_terminal(operation_id, OCCURRED_AT.to_owned(), None)
            .await
            .expect("pending terminal");
        let sink = Arc::new(RecordingSink::default());
        let recovery = StartupVoicePreviewOutboxRecovery::new(
            repository,
            sink.clone() as Arc<dyn VoicePreviewEventSink>,
            Arc::new(FixedClock),
        );

        let first = recovery.ensure_recovered().await.expect("first recovery");
        let second = recovery.ensure_recovered().await.expect("cached recovery");

        assert_eq!(first, second);
        assert_eq!(first.recovered_accepted, 0);
        assert_eq!(first.delivered, 1);
        assert_eq!(sink.events.lock().expect("events").len(), 1);
        storage.close().await;
    }

    #[tokio::test]
    async fn startup_recovery_drains_more_than_one_full_batch_without_tail() {
        let (_temp, storage, repository) = outbox_fixture().await;
        for _ in 0..205 {
            accept(&repository, Uuid::now_v7()).await;
        }
        let sink = Arc::new(RecordingSink::default());
        let recovery = StartupVoicePreviewOutboxRecovery::new(
            repository.clone(),
            sink.clone() as Arc<dyn VoicePreviewEventSink>,
            Arc::new(FixedClock),
        );
        let report = recovery
            .ensure_recovered()
            .await
            .expect("all bounded batches recover");
        assert_eq!(report.recovered_accepted, 205);
        assert_eq!(report.loaded, 205);
        assert_eq!(report.delivered, 205);
        assert_eq!(sink.events.lock().expect("events").len(), 205);
        assert_eq!(
            repository
                .count_voice_preview_recovery_work()
                .await
                .expect("no recovery tail"),
            0
        );
        storage.close().await;
    }

    #[tokio::test]
    async fn startup_emit_failure_stays_pending_and_is_not_retried_in_process() {
        let (_temp, storage, repository) = outbox_fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        repository
            .persist_voice_preview_terminal(operation_id, OCCURRED_AT.to_owned(), None)
            .await
            .expect("pending terminal");
        let sink = Arc::new(RecordingSink::default());
        sink.fail.store(true, Ordering::Release);
        let recovery = StartupVoicePreviewOutboxRecovery::new(
            repository.clone(),
            sink as Arc<dyn VoicePreviewEventSink>,
            Arc::new(FixedClock),
        );
        let first = recovery.ensure_recovered().await.expect_err("emit failure");
        let second = recovery
            .ensure_recovered()
            .await
            .expect_err("cached failure");
        assert_eq!(first.error_id, ErrorId::UnexpectedInternal);
        assert_eq!(second.error_id, first.error_id);
        assert_eq!(
            repository
                .count_voice_preview_recovery_work()
                .await
                .expect("pending after emit failure"),
            1
        );
        storage.close().await;
    }
}
