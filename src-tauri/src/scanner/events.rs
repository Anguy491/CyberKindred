use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use uuid::Uuid;

use crate::ipc::{ApiError, EventEnvelope, ProcessSequence};

pub const LIBRARY_SCAN_EVENT: &str = "cyberkindred://v1/library/scan";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanEventState {
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl ScanEventState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// Exact EVT-005 payload. It is structurally unable to carry a filesystem path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanEvent {
    pub schema_version: String,
    pub sequence: u64,
    pub occurred_at: String,
    pub operation_id: Uuid,
    pub state: ScanEventState,
    pub scanned: u64,
    pub discovered: u64,
    pub failed: u64,
    pub safe_message: Option<String>,
}

impl ScanEvent {
    pub(super) fn new(
        sequence: &ProcessSequence,
        occurred_at: String,
        operation_id: Uuid,
        state: ScanEventState,
        counters: ScanEventCounters,
        safe_message: Option<String>,
    ) -> Result<Self, ApiError> {
        let mut envelope = EventEnvelope::now(sequence.next()?);
        envelope.occurred_at = occurred_at;
        envelope.validate()?;
        Ok(Self {
            schema_version: envelope.schema_version,
            sequence: envelope.sequence,
            occurred_at: envelope.occurred_at,
            operation_id,
            state,
            scanned: counters.scanned,
            discovered: counters.discovered,
            failed: counters.failed,
            safe_message,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ScanEventCounters {
    pub scanned: u64,
    pub discovered: u64,
    pub failed: u64,
}

/// Event transport. Terminal calls may repeat after recovery; consumers dedupe
/// the persisted authoritative terminal by `operationId`.
pub trait ScanEventSink: Send + Sync {
    /// Publishes one path-free EVT-005 projection.
    ///
    /// # Errors
    ///
    /// Returns a safe transport error. A terminal remains pending when this fails.
    fn publish(&self, event: ScanEvent) -> Result<(), ApiError>;
}

pub struct TauriScanEventSink<R: Runtime> {
    app_handle: AppHandle<R>,
}

impl<R: Runtime> TauriScanEventSink<R> {
    #[must_use]
    pub fn new(app_handle: AppHandle<R>) -> Self {
        Self { app_handle }
    }
}

impl<R: Runtime> ScanEventSink for TauriScanEventSink<R> {
    fn publish(&self, event: ScanEvent) -> Result<(), ApiError> {
        self.app_handle
            .emit(LIBRARY_SCAN_EVENT, event)
            .map_err(|_| ApiError::unexpected())
    }
}

pub(super) type SharedScanEventSink = Arc<dyn ScanEventSink>;
