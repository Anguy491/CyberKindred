use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use uuid::Uuid;

use crate::{
    contracts::MemoryRecord,
    ipc::{ApiError, EventEnvelope, ProcessSequence},
};

pub const CHAT_MESSAGE_EVENT: &str = "cyberkindred://v1/chat/message";
pub const MEMORY_PROPOSED_EVENT: &str = "cyberkindred://v1/memory/proposed";
pub const OPERATION_CANCELLED_EVENT: &str = "cyberkindred://v1/operation/cancelled";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ChatRole {
    User,
    Assistant,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChatMessagePayload<'a> {
    schema_version: &'a str,
    sequence: u64,
    occurred_at: &'a str,
    operation_id: Uuid,
    program_id: Uuid,
    role: ChatRole,
    text: &'a str,
    #[serde(rename = "final")]
    final_message: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemoryProposedPayload<'a> {
    schema_version: &'a str,
    sequence: u64,
    occurred_at: &'a str,
    memory: &'a MemoryRecord,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelledPayload<'a> {
    schema_version: &'a str,
    sequence: u64,
    occurred_at: &'a str,
    operation_id: Uuid,
    kind: &'static str,
}

pub trait ChatEventSink: Send + Sync {
    fn user_message(
        &self,
        operation_id: Uuid,
        program_id: Uuid,
        text: &str,
        occurred_at: &str,
    ) -> Result<(), ApiError>;
    fn assistant_message(
        &self,
        operation_id: Uuid,
        program_id: Uuid,
        text: &str,
        occurred_at: &str,
    ) -> Result<(), ApiError>;
    fn memory_proposed(&self, memory: &MemoryRecord, occurred_at: &str) -> Result<(), ApiError>;
    fn cancelled(&self, operation_id: Uuid, occurred_at: &str) -> Result<(), ApiError>;
}

pub struct TauriChatEventSink<R: Runtime> {
    app: AppHandle<R>,
    sequence: Arc<ProcessSequence>,
}

impl<R: Runtime> TauriChatEventSink<R> {
    pub fn new(app: AppHandle<R>, sequence: Arc<ProcessSequence>) -> Self {
        Self { app, sequence }
    }

    fn envelope(&self, occurred_at: &str) -> Result<EventEnvelope, ApiError> {
        let envelope = EventEnvelope {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            sequence: self.sequence.next()?,
            occurred_at: occurred_at.to_owned(),
        };
        envelope.validate()?;
        Ok(envelope)
    }
}

impl<R: Runtime> ChatEventSink for TauriChatEventSink<R> {
    fn user_message(
        &self,
        operation_id: Uuid,
        program_id: Uuid,
        text: &str,
        occurred_at: &str,
    ) -> Result<(), ApiError> {
        let envelope = self.envelope(occurred_at)?;
        self.app
            .emit(
                CHAT_MESSAGE_EVENT,
                ChatMessagePayload {
                    schema_version: &envelope.schema_version,
                    sequence: envelope.sequence,
                    occurred_at: &envelope.occurred_at,
                    operation_id,
                    program_id,
                    role: ChatRole::User,
                    text,
                    final_message: false,
                },
            )
            .map_err(|_| ApiError::unexpected())
    }

    fn assistant_message(
        &self,
        operation_id: Uuid,
        program_id: Uuid,
        text: &str,
        occurred_at: &str,
    ) -> Result<(), ApiError> {
        let envelope = self.envelope(occurred_at)?;
        self.app
            .emit(
                CHAT_MESSAGE_EVENT,
                ChatMessagePayload {
                    schema_version: &envelope.schema_version,
                    sequence: envelope.sequence,
                    occurred_at: &envelope.occurred_at,
                    operation_id,
                    program_id,
                    role: ChatRole::Assistant,
                    text,
                    final_message: true,
                },
            )
            .map_err(|_| ApiError::unexpected())
    }

    fn memory_proposed(&self, memory: &MemoryRecord, occurred_at: &str) -> Result<(), ApiError> {
        let envelope = self.envelope(occurred_at)?;
        self.app
            .emit(
                MEMORY_PROPOSED_EVENT,
                MemoryProposedPayload {
                    schema_version: &envelope.schema_version,
                    sequence: envelope.sequence,
                    occurred_at: &envelope.occurred_at,
                    memory,
                },
            )
            .map_err(|_| ApiError::unexpected())
    }

    fn cancelled(&self, operation_id: Uuid, occurred_at: &str) -> Result<(), ApiError> {
        let envelope = self.envelope(occurred_at)?;
        self.app
            .emit(
                OPERATION_CANCELLED_EVENT,
                CancelledPayload {
                    schema_version: &envelope.schema_version,
                    sequence: envelope.sequence,
                    occurred_at: &envelope.occurred_at,
                    operation_id,
                    kind: "chat",
                },
            )
            .map_err(|_| ApiError::unexpected())
    }
}
