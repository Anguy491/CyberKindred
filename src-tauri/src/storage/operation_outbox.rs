//! Durable, non-content lifecycle records for accepted voice-preview operations.

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};
use crate::ipc::{ApiError, InternalReason};

const OPERATION_AGGREGATE: &str = "voice_preview_operation";
const ACCEPTED_REVISION: i64 = 0;
const TERMINAL_REVISION: i64 = 1;
const ACCEPTED_EVENT: &str = "cyberkindred://internal/v1/operation/accepted";
const COMPLETED_EVENT: &str = "cyberkindred://v1/operation/completed";
const FAILED_EVENT: &str = "cyberkindred://v1/operation/failed";
const CANCELLED_EVENT: &str = "cyberkindred://v1/operation/cancelled";
pub const MAX_VOICE_PREVIEW_OUTBOX_BATCH: u32 = 100;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredOperationKind {
    VoicePreview,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredOperationState {
    Accepted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredAcceptedPayload {
    schema_version: String,
    accepted_at: String,
    operation_id: Uuid,
    kind: StoredOperationKind,
    state: StoredOperationState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCompletedPayload {
    schema_version: String,
    occurred_at: String,
    operation_id: Uuid,
    kind: StoredOperationKind,
    output_label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredFailedPayload {
    schema_version: String,
    occurred_at: String,
    operation_id: Uuid,
    error: ApiError,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCancelledPayload {
    schema_version: String,
    occurred_at: String,
    operation_id: Uuid,
    kind: StoredOperationKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OperationAcceptedRecord {
    outbox_id: Uuid,
    operation_id: Uuid,
    accepted_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationTerminalKind {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationTerminalRecord {
    pub outbox_id: Uuid,
    pub operation_id: Uuid,
    pub occurred_at: String,
    kind: OperationTerminalKind,
    pub error: Option<ApiError>,
    pub delivered_at_ms: Option<i64>,
}

impl OperationTerminalRecord {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.kind == OperationTerminalKind::Cancelled
    }
}

enum OperationOutboxState {
    Accepted(OperationAcceptedRecord),
    Terminal(OperationTerminalRecord),
}

type OutboxRow = (String, String, i64, String, String, i64, Option<i64>);

impl Repository {
    pub(crate) async fn load_active_voice_preview_ids(&self) -> Result<Vec<Uuid>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT aggregate_id FROM outbox_events WHERE aggregate_type = ? AND event_type = ? AND aggregate_revision = ? AND delivered_at_ms IS NULL ORDER BY created_at_ms, id",
        )
        .bind(OPERATION_AGGREGATE)
        .bind(ACCEPTED_EVENT)
        .bind(ACCEPTED_REVISION)
        .fetch_all(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        values
            .into_iter()
            .map(|value| Uuid::parse_str(&value).map_err(|_| integrity_error()))
            .collect()
    }

    /// Persists the minimal accepted state before API-009 returns or spawns work.
    ///
    /// # Errors
    ///
    /// Returns a stable validation, integrity, conflict, or write error. No
    /// provider call or audio task should start when this write fails.
    pub async fn persist_voice_preview_accepted(
        &self,
        operation_id: Uuid,
        accepted_at: String,
    ) -> Result<Uuid, StorageError> {
        validate_operation_id(operation_id).map_err(|_| write_error())?;
        let created_at_ms = parse_timestamp(&accepted_at).map_err(|_| write_error())?;
        let payload = serde_json::to_string(&StoredAcceptedPayload {
            schema_version: "1.0.0".to_owned(),
            accepted_at: accepted_at.clone(),
            operation_id,
            kind: StoredOperationKind::VoicePreview,
            state: StoredOperationState::Accepted,
        })
        .map_err(|_| write_error())?;
        let outbox_id = Uuid::now_v7();
        let inserted = sqlx::query(
            "INSERT INTO outbox_events(id, aggregate_type, aggregate_id, aggregate_revision, event_type, payload_json, created_at_ms, delivered_at_ms) SELECT ?, ?, ?, ?, ?, ?, ?, NULL WHERE NOT EXISTS (SELECT 1 FROM outbox_events WHERE aggregate_type = ? AND aggregate_id = ?)",
        )
        .bind(outbox_id.to_string())
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(ACCEPTED_REVISION)
        .bind(ACCEPTED_EVENT)
        .bind(payload)
        .bind(created_at_ms)
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        if inserted.rows_affected() == 1 {
            return Ok(outbox_id);
        }
        match self.load_voice_preview_state(operation_id).await? {
            Some(OperationOutboxState::Accepted(existing))
                if existing.accepted_at == accepted_at =>
            {
                Ok(existing.outbox_id)
            }
            Some(_) => Err(StorageError::new(StorageReason::RevisionConflict)),
            None => Err(integrity_error()),
        }
    }

    /// Atomically converts the accepted row to its one authoritative terminal.
    ///
    /// Completed and failed are mutually exclusive. An identical retry returns
    /// the existing terminal; a different terminal returns a revision conflict.
    ///
    /// # Errors
    ///
    /// Returns not-found if acceptance was never persisted, or a stable
    /// validation, integrity, conflict, or write error.
    pub async fn persist_voice_preview_terminal(
        &self,
        operation_id: Uuid,
        occurred_at: String,
        error: Option<ApiError>,
    ) -> Result<OperationTerminalRecord, StorageError> {
        validate_operation_id(operation_id).map_err(|_| write_error())?;
        let created_at_ms = parse_timestamp(&occurred_at).map_err(|_| write_error())?;
        let event_type = if error.is_some() {
            FAILED_EVENT
        } else {
            COMPLETED_EVENT
        };
        let terminal_kind = if error.is_some() {
            OperationTerminalKind::Failed
        } else {
            OperationTerminalKind::Completed
        };
        let payload = encode_terminal_payload(operation_id, &occurred_at, error.as_ref())?;
        let updated = sqlx::query(
            "UPDATE outbox_events SET aggregate_revision = ?, event_type = ?, payload_json = ?, created_at_ms = ?, delivered_at_ms = NULL WHERE aggregate_type = ? AND aggregate_id = ? AND event_type = ? AND aggregate_revision = ? AND delivered_at_ms IS NULL AND NOT EXISTS (SELECT 1 FROM outbox_events AS terminal WHERE terminal.aggregate_type = ? AND terminal.aggregate_id = ? AND terminal.event_type IN (?, ?, ?))",
        )
        .bind(TERMINAL_REVISION)
        .bind(event_type)
        .bind(payload)
        .bind(created_at_ms)
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(ACCEPTED_EVENT)
        .bind(ACCEPTED_REVISION)
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(COMPLETED_EVENT)
        .bind(FAILED_EVENT)
        .bind(CANCELLED_EVENT)
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() == 1 {
            let Some(OperationOutboxState::Terminal(record)) =
                self.load_voice_preview_state(operation_id).await?
            else {
                return Err(integrity_error());
            };
            return Ok(record);
        }

        match self.load_voice_preview_state(operation_id).await? {
            Some(OperationOutboxState::Terminal(existing))
                if existing.kind == terminal_kind
                    && existing.occurred_at == occurred_at
                    && existing.error == error =>
            {
                Ok(existing)
            }
            Some(_) => Err(StorageError::new(StorageReason::RevisionConflict)),
            None => Err(StorageError::new(StorageReason::EntityNotFound)),
        }
    }

    /// Atomically changes an accepted preview to the EVT-011 terminal.
    ///
    /// A new cancellation wins only while the accepted row is authoritative.
    /// Any existing terminal is returned as `AlreadyTerminal`, including a
    /// prior cancellation issued with a different API-038 request ID.
    ///
    /// # Errors
    ///
    /// Returns not-found when the operation was never accepted, or a stable
    /// validation, integrity, or storage failure.
    pub async fn cancel_voice_preview(
        &self,
        operation_id: Uuid,
        occurred_at: String,
    ) -> Result<(bool, OperationTerminalRecord), StorageError> {
        validate_operation_id(operation_id).map_err(|_| write_error())?;
        let created_at_ms = parse_timestamp(&occurred_at).map_err(|_| write_error())?;
        let payload = serde_json::to_string(&StoredCancelledPayload {
            schema_version: "1.0.0".to_owned(),
            occurred_at,
            operation_id,
            kind: StoredOperationKind::VoicePreview,
        })
        .map_err(|_| write_error())?;
        let updated = sqlx::query(
            "UPDATE outbox_events SET aggregate_revision = ?, event_type = ?, payload_json = ?, created_at_ms = ?, delivered_at_ms = NULL WHERE aggregate_type = ? AND aggregate_id = ? AND event_type = ? AND aggregate_revision = ? AND delivered_at_ms IS NULL AND NOT EXISTS (SELECT 1 FROM outbox_events AS terminal WHERE terminal.aggregate_type = ? AND terminal.aggregate_id = ? AND terminal.event_type IN (?, ?, ?))",
        )
        .bind(TERMINAL_REVISION)
        .bind(CANCELLED_EVENT)
        .bind(payload)
        .bind(created_at_ms)
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(ACCEPTED_EVENT)
        .bind(ACCEPTED_REVISION)
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(COMPLETED_EVENT)
        .bind(FAILED_EVENT)
        .bind(CANCELLED_EVENT)
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        let Some(OperationOutboxState::Terminal(record)) =
            self.load_voice_preview_state(operation_id).await?
        else {
            return if updated.rows_affected() == 0 {
                Err(StorageError::new(StorageReason::EntityNotFound))
            } else {
                Err(integrity_error())
            };
        };
        if updated.rows_affected() == 1 {
            Ok((true, record))
        } else {
            Ok((false, record))
        }
    }

    /// Converts one bounded batch of abandoned accepted operations to fixed,
    /// safe `ERR-1601` failed terminals for startup recovery.
    ///
    /// # Errors
    ///
    /// Returns a validation, read, integrity, conflict, or write error. Each
    /// accepted-to-failed transition is one atomic SQLite statement.
    pub async fn recover_abandoned_voice_previews(
        &self,
        limit: u32,
        recovered_at: &str,
    ) -> Result<u32, StorageError> {
        validate_batch_limit(limit)?;
        parse_timestamp(recovered_at).map_err(|_| write_error())?;
        let rows: Vec<OutboxRow> = sqlx::query_as(
            "SELECT id, aggregate_id, aggregate_revision, event_type, payload_json, created_at_ms, delivered_at_ms FROM outbox_events WHERE aggregate_type = ? AND event_type = ? ORDER BY created_at_ms ASC, id ASC LIMIT ?",
        )
        .bind(OPERATION_AGGREGATE)
        .bind(ACCEPTED_EVENT)
        .bind(i64::from(limit))
        .fetch_all(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        let mut recovered = 0_u32;
        for row in rows {
            let OperationOutboxState::Accepted(accepted) = decode_row(row)? else {
                return Err(integrity_error());
            };
            self.persist_voice_preview_terminal(
                accepted.operation_id,
                recovered_at.to_owned(),
                Some(ApiError::from_reason(InternalReason::UnexpectedInternal)),
            )
            .await?;
            recovered = recovered.checked_add(1).ok_or_else(write_error)?;
        }
        Ok(recovered)
    }

    /// Reads one bounded terminal batch for startup-only delivery.
    ///
    /// # Errors
    ///
    /// Returns a validation/read error or fails closed on a malformed row.
    pub async fn load_pending_voice_preview_terminals(
        &self,
        limit: u32,
    ) -> Result<Vec<OperationTerminalRecord>, StorageError> {
        validate_batch_limit(limit)?;
        let rows: Vec<OutboxRow> = sqlx::query_as(
            "SELECT id, aggregate_id, aggregate_revision, event_type, payload_json, created_at_ms, delivered_at_ms FROM outbox_events WHERE aggregate_type = ? AND delivered_at_ms IS NULL AND event_type IN (?, ?, ?) ORDER BY created_at_ms ASC, id ASC LIMIT ?",
        )
        .bind(OPERATION_AGGREGATE)
        .bind(COMPLETED_EVENT)
        .bind(FAILED_EVENT)
        .bind(CANCELLED_EVENT)
        .bind(i64::from(limit))
        .fetch_all(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        rows.into_iter()
            .map(|row| match decode_row(row)? {
                OperationOutboxState::Terminal(record) => Ok(record),
                OperationOutboxState::Accepted(_) => Err(integrity_error()),
            })
            .collect()
    }

    /// Counts all undelivered accepted or terminal voice-preview rows.
    ///
    /// # Errors
    ///
    /// Returns `storage_read_failed` if SQLite cannot read the bounded count.
    pub async fn count_voice_preview_recovery_work(&self) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox_events WHERE aggregate_type = ? AND delivered_at_ms IS NULL",
        )
        .bind(OPERATION_AGGREGATE)
        .fetch_one(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        u64::try_from(count).map_err(|_| integrity_error())
    }

    /// Conditionally marks one emitted terminal delivered.
    ///
    /// # Errors
    ///
    /// Returns a validation, not-found, or write error without changing a
    /// pending row on failure.
    pub async fn mark_voice_preview_terminal_delivered(
        &self,
        outbox_id: Uuid,
        operation_id: Uuid,
        delivered_at_ms: i64,
    ) -> Result<bool, StorageError> {
        validate_v7_uuid(outbox_id).map_err(|_| write_error())?;
        validate_operation_id(operation_id).map_err(|_| write_error())?;
        if delivered_at_ms < 0 {
            return Err(write_error());
        }
        let updated = sqlx::query(
            "UPDATE outbox_events SET delivered_at_ms = ? WHERE id = ? AND aggregate_type = ? AND aggregate_id = ? AND event_type IN (?, ?, ?) AND delivered_at_ms IS NULL AND created_at_ms <= ?",
        )
        .bind(delivered_at_ms)
        .bind(outbox_id.to_string())
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(COMPLETED_EVENT)
        .bind(FAILED_EVENT)
        .bind(CANCELLED_EVENT)
        .bind(delivered_at_ms)
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() == 1 {
            return Ok(true);
        }
        match self.load_voice_preview_state(operation_id).await? {
            Some(OperationOutboxState::Terminal(record))
                if record.outbox_id == outbox_id && record.delivered_at_ms.is_some() =>
            {
                Ok(false)
            }
            Some(_) => Err(write_error()),
            None => Err(StorageError::new(StorageReason::EntityNotFound)),
        }
    }

    async fn load_voice_preview_state(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<OperationOutboxState>, StorageError> {
        let rows: Vec<OutboxRow> = sqlx::query_as(
            "SELECT id, aggregate_id, aggregate_revision, event_type, payload_json, created_at_ms, delivered_at_ms FROM outbox_events WHERE aggregate_type = ? AND aggregate_id = ?",
        )
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .fetch_all(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        match rows.len() {
            0 => Ok(None),
            1 => rows.into_iter().next().map(decode_row).transpose(),
            _ => Err(integrity_error()),
        }
    }
}

fn encode_terminal_payload(
    operation_id: Uuid,
    occurred_at: &str,
    error: Option<&ApiError>,
) -> Result<String, StorageError> {
    match error {
        None => serde_json::to_string(&StoredCompletedPayload {
            schema_version: "1.0.0".to_owned(),
            occurred_at: occurred_at.to_owned(),
            operation_id,
            kind: StoredOperationKind::VoicePreview,
            output_label: None,
        })
        .map_err(|_| write_error()),
        Some(error) => {
            validate_terminal_error(error).map_err(|_| write_error())?;
            serde_json::to_string(&StoredFailedPayload {
                schema_version: "1.0.0".to_owned(),
                occurred_at: occurred_at.to_owned(),
                operation_id,
                error: error.clone(),
            })
            .map_err(|_| write_error())
        }
    }
}

fn decode_row(row: OutboxRow) -> Result<OperationOutboxState, StorageError> {
    let (outbox_id, aggregate_id, revision, event_type, payload, created_at, delivered_at) = row;
    let outbox_id = parse_uuid(&outbox_id)?;
    validate_v7_uuid(outbox_id)?;
    let aggregate_id = parse_uuid(&aggregate_id)?;
    validate_operation_id(aggregate_id)?;
    if created_at < 0 || delivered_at.is_some_and(|value| value < created_at) {
        return Err(integrity_error());
    }
    match event_type.as_str() {
        ACCEPTED_EVENT => {
            let decoded: StoredAcceptedPayload =
                serde_json::from_str(&payload).map_err(|_| integrity_error())?;
            if revision != ACCEPTED_REVISION
                || delivered_at.is_some()
                || decoded.schema_version != "1.0.0"
                || decoded.kind != StoredOperationKind::VoicePreview
                || decoded.state != StoredOperationState::Accepted
                || decoded.operation_id != aggregate_id
                || parse_timestamp(&decoded.accepted_at)? != created_at
            {
                return Err(integrity_error());
            }
            Ok(OperationOutboxState::Accepted(OperationAcceptedRecord {
                outbox_id,
                operation_id: aggregate_id,
                accepted_at: decoded.accepted_at,
            }))
        }
        COMPLETED_EVENT => {
            let decoded: StoredCompletedPayload =
                serde_json::from_str(&payload).map_err(|_| integrity_error())?;
            if revision != TERMINAL_REVISION
                || decoded.schema_version != "1.0.0"
                || decoded.kind != StoredOperationKind::VoicePreview
                || decoded.output_label.is_some()
                || decoded.operation_id != aggregate_id
                || parse_timestamp(&decoded.occurred_at)? != created_at
            {
                return Err(integrity_error());
            }
            Ok(OperationOutboxState::Terminal(OperationTerminalRecord {
                outbox_id,
                operation_id: aggregate_id,
                occurred_at: decoded.occurred_at,
                kind: OperationTerminalKind::Completed,
                error: None,
                delivered_at_ms: delivered_at,
            }))
        }
        FAILED_EVENT => {
            let decoded: StoredFailedPayload =
                serde_json::from_str(&payload).map_err(|_| integrity_error())?;
            validate_terminal_error(&decoded.error)?;
            if revision != TERMINAL_REVISION
                || decoded.schema_version != "1.0.0"
                || decoded.operation_id != aggregate_id
                || parse_timestamp(&decoded.occurred_at)? != created_at
            {
                return Err(integrity_error());
            }
            Ok(OperationOutboxState::Terminal(OperationTerminalRecord {
                outbox_id,
                operation_id: aggregate_id,
                occurred_at: decoded.occurred_at,
                kind: OperationTerminalKind::Failed,
                error: Some(decoded.error),
                delivered_at_ms: delivered_at,
            }))
        }
        CANCELLED_EVENT => {
            let decoded: StoredCancelledPayload =
                serde_json::from_str(&payload).map_err(|_| integrity_error())?;
            if revision != TERMINAL_REVISION
                || decoded.schema_version != "1.0.0"
                || decoded.kind != StoredOperationKind::VoicePreview
                || decoded.operation_id != aggregate_id
                || parse_timestamp(&decoded.occurred_at)? != created_at
            {
                return Err(integrity_error());
            }
            Ok(OperationOutboxState::Terminal(OperationTerminalRecord {
                outbox_id,
                operation_id: aggregate_id,
                occurred_at: decoded.occurred_at,
                kind: OperationTerminalKind::Cancelled,
                error: None,
                delivered_at_ms: delivered_at,
            }))
        }
        _ => Err(integrity_error()),
    }
}

fn validate_terminal_error(error: &ApiError) -> Result<(), StorageError> {
    if error.schema_version != "1.0.0"
        || error.safe_message.chars().count() == 0
        || error.safe_message.chars().count() > 300
        || parse_uuid(&error.correlation_id)?.is_nil()
    {
        return Err(integrity_error());
    }
    let details = error.details.as_ref().ok_or_else(integrity_error)?;
    let reason = details.reason.as_deref().ok_or_else(integrity_error)?;
    let expected_reason = match reason {
        "provider_authentication" => InternalReason::ProviderAuthentication,
        "provider_rate_limit" => InternalReason::ProviderRateLimit,
        "provider_timeout" => InternalReason::ProviderTimeout,
        "provider_unavailable" => InternalReason::ProviderUnavailable,
        "network_unavailable" => InternalReason::NetworkUnavailable,
        "provider_invalid_response" => InternalReason::ProviderInvalidResponse,
        "provider_output_policy_violation" => InternalReason::ProviderOutputPolicyViolation,
        "storage_read_failed" => InternalReason::StorageReadFailed,
        "storage_write_failed" => InternalReason::StorageWriteFailed,
        "storage_integrity_failed" => InternalReason::StorageIntegrityFailed,
        "unexpected_internal" => InternalReason::UnexpectedInternal,
        _ => return Err(integrity_error()),
    };
    let expected = ApiError::from_reason(expected_reason);
    if error.error_id != expected.error_id
        || error.safe_message != expected.safe_message
        || error.retryable != expected.retryable
        || details.field.is_some()
        || details.current_revision.is_some()
        || details.capability.is_some()
        || details.operation_id.is_some()
        || error
            .retry_after_ms
            .is_some_and(|value| value > 9_007_199_254_740_991)
        || (error.retry_after_ms.is_some() && reason != "provider_rate_limit")
    {
        return Err(integrity_error());
    }
    Ok(())
}

fn validate_batch_limit(limit: u32) -> Result<(), StorageError> {
    if (1..=MAX_VOICE_PREVIEW_OUTBOX_BATCH).contains(&limit) {
        Ok(())
    } else {
        Err(StorageError::new(StorageReason::InvalidSetting))
    }
}

fn parse_timestamp(value: &str) -> Result<i64, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .map_err(|_| integrity_error())
        .and_then(|timestamp| {
            if timestamp < 0 {
                Err(integrity_error())
            } else {
                Ok(timestamp)
            }
        })
}

fn validate_operation_id(value: Uuid) -> Result<(), StorageError> {
    validate_v7_uuid(value)
}

fn validate_v7_uuid(value: Uuid) -> Result<(), StorageError> {
    if value.is_nil() || value.get_version_num() != 7 {
        Err(integrity_error())
    } else {
        Ok(())
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|_| integrity_error())
}

const fn write_error() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

const fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    const ACCEPTED_AT: &str = "2026-09-02T03:04:04.000Z";
    const TERMINAL_AT: &str = "2026-09-02T03:04:05.000Z";
    const TERMINAL_AT_MS: i64 = 1_788_318_245_000;

    async fn fixture() -> (tempfile::TempDir, Storage, Repository) {
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
            .persist_voice_preview_accepted(operation_id, ACCEPTED_AT.to_owned())
            .await
            .expect("accepted row");
    }

    #[tokio::test]
    async fn accepted_is_unique_and_terminal_requires_acceptance() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        let first = repository
            .persist_voice_preview_accepted(operation_id, ACCEPTED_AT.to_owned())
            .await
            .expect("first accept");
        let retry = repository
            .persist_voice_preview_accepted(operation_id, ACCEPTED_AT.to_owned())
            .await
            .expect("idempotent accept");
        assert_eq!(first, retry);
        let missing = repository
            .persist_voice_preview_terminal(Uuid::now_v7(), TERMINAL_AT.to_owned(), None)
            .await
            .expect_err("terminal without accept");
        assert_eq!(missing.reason(), StorageReason::EntityNotFound);
        storage.close().await;
    }

    #[tokio::test]
    async fn concurrent_terminal_transition_is_atomic_and_unique() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let completed_repository = repository.clone();
        let failed_repository = repository.clone();
        let (completed, failed) = tokio::join!(
            completed_repository.persist_voice_preview_terminal(
                operation_id,
                TERMINAL_AT.to_owned(),
                None,
            ),
            failed_repository.persist_voice_preview_terminal(
                operation_id,
                TERMINAL_AT.to_owned(),
                Some(ApiError::from_reason(InternalReason::ProviderTimeout)),
            ),
        );
        assert_eq!(u8::from(completed.is_ok()) + u8::from(failed.is_ok()), 1);
        let conflict = completed
            .err()
            .or_else(|| failed.err())
            .expect("one conflict");
        assert_eq!(conflict.reason(), StorageReason::RevisionConflict);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox_events WHERE aggregate_type = ? AND aggregate_id = ?",
        )
        .bind(OPERATION_AGGREGATE)
        .bind(operation_id.to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("one authority");
        assert_eq!(count, 1);
        storage.close().await;
    }

    #[tokio::test]
    async fn cancellation_is_one_atomic_terminal_and_later_cancel_is_already_terminal() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let (won, cancelled) = repository
            .cancel_voice_preview(operation_id, TERMINAL_AT.to_owned())
            .await
            .expect("first cancellation");
        assert!(won);
        assert!(cancelled.is_cancelled());
        assert!(cancelled.error.is_none());
        let (won_again, same) = repository
            .cancel_voice_preview(operation_id, TERMINAL_AT.to_owned())
            .await
            .expect("second cancellation observes terminal");
        assert!(!won_again);
        assert_eq!(same, cancelled);
        let pending = repository
            .load_pending_voice_preview_terminals(10)
            .await
            .expect("cancelled terminal is replayable");
        assert_eq!(pending, vec![cancelled]);
        storage.close().await;
    }

    #[tokio::test]
    async fn cancellation_and_completion_race_has_exactly_one_authority() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let completion_repository = repository.clone();
        let cancellation_repository = repository.clone();
        let (completion, cancellation) = tokio::join!(
            completion_repository.persist_voice_preview_terminal(
                operation_id,
                TERMINAL_AT.to_owned(),
                None,
            ),
            cancellation_repository.cancel_voice_preview(operation_id, TERMINAL_AT.to_owned(),),
        );
        let cancellation_won = cancellation.as_ref().is_ok_and(|(won, _)| *won);
        assert_eq!(completion.is_ok(), !cancellation_won);
        let pending = repository
            .load_pending_voice_preview_terminals(10)
            .await
            .expect("one replayable terminal");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].is_cancelled(), cancellation_won);
        storage.close().await;
    }

    #[tokio::test]
    async fn terminal_retry_is_idempotent_and_different_terminal_conflicts() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let first = repository
            .persist_voice_preview_terminal(operation_id, TERMINAL_AT.to_owned(), None)
            .await
            .expect("completed terminal");
        let retry = repository
            .persist_voice_preview_terminal(operation_id, TERMINAL_AT.to_owned(), None)
            .await
            .expect("same terminal");
        assert_eq!(first, retry);
        let conflict = repository
            .persist_voice_preview_terminal(
                operation_id,
                TERMINAL_AT.to_owned(),
                Some(ApiError::from_reason(InternalReason::ProviderTimeout)),
            )
            .await
            .expect_err("different terminal");
        assert_eq!(conflict.reason(), StorageReason::RevisionConflict);
        storage.close().await;
    }

    #[tokio::test]
    async fn mark_failure_leaves_terminal_pending() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let terminal = repository
            .persist_voice_preview_terminal(operation_id, TERMINAL_AT.to_owned(), None)
            .await
            .expect("terminal");
        assert!(
            repository
                .mark_voice_preview_terminal_delivered(
                    terminal.outbox_id,
                    Uuid::now_v7(),
                    TERMINAL_AT_MS,
                )
                .await
                .is_err()
        );
        assert_eq!(
            repository
                .count_voice_preview_recovery_work()
                .await
                .expect("one pending terminal"),
            1
        );
        assert!(
            repository
                .mark_voice_preview_terminal_delivered(
                    terminal.outbox_id,
                    operation_id,
                    TERMINAL_AT_MS,
                )
                .await
                .expect("mark delivered")
        );
        assert_eq!(
            repository
                .count_voice_preview_recovery_work()
                .await
                .expect("no recovery work"),
            0
        );
        storage.close().await;
    }

    #[tokio::test]
    async fn unsafe_terminal_error_is_rejected_without_canary_persistence() {
        const SECRET_CANARY: &str = "sk-outbox-secret-canary";
        const PATH_CANARY: &str = "C:\\\\Users\\\\Canary\\\\private.mp3";
        let (temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        let mut error = ApiError::from_reason(InternalReason::ProviderUnavailable);
        error.safe_message = format!("{SECRET_CANARY} {PATH_CANARY}");
        assert!(
            repository
                .persist_voice_preview_terminal(operation_id, TERMINAL_AT.to_owned(), Some(error),)
                .await
                .is_err()
        );
        storage.passive_checkpoint().await.expect("checkpoint");
        for entry in std::fs::read_dir(temp.path().join("data")).expect("data directory") {
            let path = entry.expect("data entry").path();
            if path.is_file() {
                let bytes = std::fs::read(path).expect("database artifact");
                for canary in [SECRET_CANARY, PATH_CANARY] {
                    assert!(
                        !bytes
                            .windows(canary.len())
                            .any(|window| window == canary.as_bytes()),
                        "forbidden terminal canary must not enter storage"
                    );
                }
            }
        }
        storage.close().await;
    }

    #[tokio::test]
    async fn abandoned_accept_is_converted_to_safe_failed_terminal() {
        let (_temp, storage, repository) = fixture().await;
        let operation_id = Uuid::now_v7();
        accept(&repository, operation_id).await;
        assert_eq!(
            repository
                .recover_abandoned_voice_previews(100, TERMINAL_AT)
                .await
                .expect("one abandoned accept recovered"),
            1
        );
        let pending = repository
            .load_pending_voice_preview_terminals(100)
            .await
            .expect("recovered terminal");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation_id, operation_id);
        assert_eq!(
            pending[0].error.as_ref().map(|error| error.error_id),
            Some(crate::ipc::ErrorId::UnexpectedInternal)
        );
        storage.close().await;
    }
}
