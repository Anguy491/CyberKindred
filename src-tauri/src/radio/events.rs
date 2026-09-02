use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Runtime};
use uuid::Uuid;

use crate::ipc::{ApiError, EventEnvelope, InternalReason, ProcessSequence};

pub const PROGRAM_STATE_EVENT: &str = "cyberkindred://v1/program/state";
pub const PROGRAM_SEGMENT_EVENT: &str = "cyberkindred://v1/program/segment";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramEventState {
    Planning,
    Running,
    Paused,
    Stopping,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramSegmentEventState {
    Queued,
    Playing,
    Completed,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramStateEvent {
    #[serde(flatten)]
    pub envelope: EventEnvelope,
    pub program_id: Uuid,
    pub state: ProgramEventState,
    pub safe_message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramSegmentEvent {
    #[serde(flatten)]
    pub envelope: EventEnvelope,
    pub program_id: Uuid,
    pub segment_id: Uuid,
    pub state: ProgramSegmentEventState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RadioEvent {
    ProgramState(ProgramStateEvent),
    ProgramSegment(ProgramSegmentEvent),
}

/// At-most-once event transport. Callers never retry failed emission.
pub trait RadioEventSink: Send + Sync + 'static {
    /// Attempts a single public delivery.
    ///
    /// # Errors
    ///
    /// Returns a redacted transport failure without changing persisted state.
    fn publish(&self, event: RadioEvent) -> Result<(), ApiError>;
}

pub struct TauriRadioEventSink<R: Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: Runtime> TauriRadioEventSink<R> {
    #[must_use]
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> RadioEventSink for TauriRadioEventSink<R> {
    fn publish(&self, event: RadioEvent) -> Result<(), ApiError> {
        let result = match event {
            RadioEvent::ProgramState(event) => self.app.emit(PROGRAM_STATE_EVENT, event),
            RadioEvent::ProgramSegment(event) => self.app.emit(PROGRAM_SEGMENT_EVENT, event),
        };
        result.map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))
    }
}

pub trait RadioClock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;

    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }

    fn now_rfc3339(&self) -> String {
        self.now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRadioClock;

impl RadioClock for SystemRadioClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone)]
pub(crate) struct RadioEventPublisher {
    sink: Arc<dyn RadioEventSink>,
    sequence: Arc<ProcessSequence>,
    clock: Arc<dyn RadioClock>,
}

impl RadioEventPublisher {
    pub(crate) fn new(
        sink: Arc<dyn RadioEventSink>,
        sequence: Arc<ProcessSequence>,
        clock: Arc<dyn RadioClock>,
    ) -> Self {
        Self {
            sink,
            sequence,
            clock,
        }
    }

    pub(crate) fn state(
        &self,
        program_id: Uuid,
        state: ProgramEventState,
        safe_message: Option<&'static str>,
    ) {
        let Some(envelope) = self.envelope() else {
            return;
        };
        let _ = self
            .sink
            .publish(RadioEvent::ProgramState(ProgramStateEvent {
                envelope,
                program_id,
                state,
                safe_message: safe_message.map(str::to_owned),
            }));
    }

    pub(crate) fn segment(
        &self,
        program_id: Uuid,
        segment_id: Uuid,
        state: ProgramSegmentEventState,
    ) {
        let Some(envelope) = self.envelope() else {
            return;
        };
        let _ = self
            .sink
            .publish(RadioEvent::ProgramSegment(ProgramSegmentEvent {
                envelope,
                program_id,
                segment_id,
                state,
            }));
    }

    fn envelope(&self) -> Option<EventEnvelope> {
        self.sequence.next().ok().map(|sequence| EventEnvelope {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            sequence,
            occurred_at: self.clock.now_rfc3339(),
        })
    }
}
