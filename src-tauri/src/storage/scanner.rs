//! Transactional persistence for authorized local-library scans.

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction, sqlite::SqliteRow};
use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};

const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_SCAN_BATCH: usize = 500;
const MAX_ROOTS_PER_OPERATION: usize = 256;
const MAX_OUTBOX_BATCH: u32 = 100;
const MAX_PATH_CHARS: usize = 32_767;
const MAX_FILE_IDENTITY_CHARS: usize = 256;
const MAX_TAG_CHARS: usize = 1_000;
const MAX_GENRES: usize = 100;
const MAX_GENRE_CHARS: usize = 200;
const SCAN_AGGREGATE: &str = "library_scan_operation";
const SCAN_EVENT: &str = "cyberkindred://v1/library/scan";
const TERMINAL_REVISION: i64 = 1;

/// An accepted operation and its Rust-only authorized roots.
///
/// This type deliberately implements neither `Debug` nor serialization because
/// it contains canonical local paths.
#[derive(Clone)]
pub(crate) struct AuthorizedScanOperation {
    operation_id: Uuid,
    roots: Vec<AuthorizedScanRoot>,
}

impl AuthorizedScanOperation {
    #[must_use]
    pub(crate) const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    #[must_use]
    pub(crate) fn roots(&self) -> &[AuthorizedScanRoot] {
        &self.roots
    }
}

/// One authorized root/job binding. Canonical paths never cross IPC.
#[derive(Clone)]
pub(crate) struct AuthorizedScanRoot {
    job_id: Uuid,
    canonical_path: PathBuf,
}

impl AuthorizedScanRoot {
    #[must_use]
    pub(crate) const fn job_id(&self) -> Uuid {
        self.job_id
    }

    #[must_use]
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanTrackAvailability {
    Available,
    Corrupt,
    Unsupported,
}

impl ScanTrackAvailability {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Corrupt => "corrupt",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One scanner-produced local fact. It deliberately has no `Debug` or serde
/// implementation because `relative_path` is storage-only path information.
pub(crate) struct ScanTrackRecord {
    pub(crate) relative_path: PathBuf,
    pub(crate) file_identity: Option<String>,
    pub(crate) availability: ScanTrackAvailability,
    pub(crate) format: String,
    pub(crate) file_size_bytes: u64,
    pub(crate) modified_at_ms: i64,
    pub(crate) duration_ms: u64,
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) album_artist: Option<String>,
    pub(crate) track_number: Option<u32>,
    pub(crate) disc_number: Option<u32>,
    pub(crate) year: Option<i32>,
    pub(crate) genres: Vec<String>,
    pub(crate) metadata_confidence: f64,
    pub(crate) embedded_cover_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScanCounters {
    pub(crate) files_seen: u64,
    pub(crate) tracks_indexed: u64,
    pub(crate) errors_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanOperationState {
    Accepted,
    Running,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl ScanOperationState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanTerminalState {
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl ScanTerminalState {
    const fn operation_state(self) -> ScanOperationState {
        match self {
            Self::Completed => ScanOperationState::Completed,
            Self::Cancelled => ScanOperationState::Cancelled,
            Self::Failed => ScanOperationState::Failed,
            Self::Interrupted => ScanOperationState::Interrupted,
        }
    }

    const fn job_status(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    const fn error_code(self) -> Option<&'static str> {
        match self {
            Self::Failed => Some("scan_failed"),
            Self::Interrupted => Some("interrupted"),
            Self::Completed | Self::Cancelled => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScanProgress {
    pub(crate) operation_id: Uuid,
    pub(crate) state: ScanOperationState,
    pub(crate) counters: ScanCounters,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScanTerminalRecord {
    pub(crate) outbox_id: Uuid,
    pub(crate) operation_id: Uuid,
    pub(crate) state: ScanTerminalState,
    pub(crate) counters: ScanCounters,
    pub(crate) occurred_at_ms: i64,
    pub(crate) delivered_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanCancelOutcome {
    Cancelled(ScanTerminalRecord),
    AlreadyTerminal(ScanTerminalRecord),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredScanTerminal {
    schema_version: String,
    operation_id: Uuid,
    state: StoredScanTerminalState,
    files_seen: u64,
    tracks_indexed: u64,
    errors_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredScanTerminalState {
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl From<ScanTerminalState> for StoredScanTerminalState {
    fn from(value: ScanTerminalState) -> Self {
        match value {
            ScanTerminalState::Completed => Self::Completed,
            ScanTerminalState::Cancelled => Self::Cancelled,
            ScanTerminalState::Failed => Self::Failed,
            ScanTerminalState::Interrupted => Self::Interrupted,
        }
    }
}

impl From<StoredScanTerminalState> for ScanTerminalState {
    fn from(value: StoredScanTerminalState) -> Self {
        match value {
            StoredScanTerminalState::Completed => Self::Completed,
            StoredScanTerminalState::Cancelled => Self::Cancelled,
            StoredScanTerminalState::Failed => Self::Failed,
            StoredScanTerminalState::Interrupted => Self::Interrupted,
        }
    }
}

struct ValidatedTrack<'a> {
    record: &'a ScanTrackRecord,
    relative_path: String,
    relative_path_key: String,
    genres_json: String,
    file_size_bytes: i64,
    duration_ms: i64,
}

struct StoredRoot {
    root_id: Uuid,
    enabled: bool,
    canonical_path: PathBuf,
}

impl Repository {
    /// Atomically accepts one API operation over validated, currently authorized roots.
    pub(crate) async fn accept_scan_operation(
        &self,
        operation_id: Uuid,
        requested_root_ids: &[Uuid],
        accepted_at_ms: i64,
        app_version: &str,
    ) -> Result<AuthorizedScanOperation, StorageError> {
        validate_v7(operation_id)?;
        validate_time(accepted_at_ms)?;
        validate_app_version(app_version)?;
        let requested = validate_requested_roots(requested_root_ids)?;
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let stored_roots = load_and_validate_roots(&mut transaction).await?;
        let selected = select_roots(stored_roots, &requested)?;
        if selected.is_empty() {
            return Err(StorageError::new(StorageReason::EntityNotFound));
        }
        if selected.len() > MAX_ROOTS_PER_OPERATION {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        for root in &selected {
            let active: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM scan_jobs WHERE root_id = ? AND status IN ('queued', 'running'))",
            )
            .bind(root.root_id.to_string())
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| read_error())?;
            if active != 0 {
                return Err(StorageError::new(StorageReason::ResourceBusy));
            }
        }
        sqlx::query(
            "INSERT INTO scan_operations(id, status, app_version) VALUES(?, 'accepted', ?)",
        )
        .bind(operation_id.to_string())
        .bind(app_version)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;

        let mut roots = Vec::with_capacity(selected.len());
        for root in selected {
            let job_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO scan_jobs(id, root_id, status, app_version) VALUES(?, ?, 'queued', ?)",
            )
            .bind(job_id.to_string())
            .bind(root.root_id.to_string())
            .bind(app_version)
            .execute(&mut *transaction)
            .await
            .map_err(|_| write_error())?;
            sqlx::query(
                "INSERT INTO scan_operation_roots(operation_id, root_id, scan_job_id) VALUES(?, ?, ?)",
            )
            .bind(operation_id.to_string())
            .bind(root.root_id.to_string())
            .bind(job_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|_| write_error())?;
            roots.push(AuthorizedScanRoot {
                job_id,
                canonical_path: root.canonical_path,
            });
        }
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(AuthorizedScanOperation {
            operation_id,
            roots,
        })
    }

    /// Moves an accepted operation and all root jobs to running in one transaction.
    pub(crate) async fn mark_scan_operation_running(
        &self,
        operation_id: Uuid,
        started_at_ms: i64,
    ) -> Result<ScanProgress, StorageError> {
        validate_v7(operation_id)?;
        validate_time(started_at_ms)?;
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let current = load_progress(&mut transaction, operation_id).await?;
        match current.state {
            ScanOperationState::Accepted => {
                let operation = sqlx::query(
                    "UPDATE scan_operations SET status = 'running', started_at_ms = ? WHERE id = ? AND status = 'accepted'",
                )
                .bind(started_at_ms)
                .bind(operation_id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(|_| write_error())?;
                let jobs = sqlx::query(
                    "UPDATE scan_jobs SET status = 'running', started_at_ms = ? WHERE id IN (SELECT scan_job_id FROM scan_operation_roots WHERE operation_id = ?) AND status = 'queued'",
                )
                .bind(started_at_ms)
                .bind(operation_id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(|_| write_error())?;
                let expected_jobs: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM scan_operation_roots WHERE operation_id = ?",
                )
                .bind(operation_id.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| read_error())?;
                if operation.rows_affected() != 1
                    || jobs.rows_affected()
                        != u64::try_from(expected_jobs).map_err(|_| integrity_error())?
                    || expected_jobs == 0
                {
                    return Err(integrity_error());
                }
            }
            ScanOperationState::Running => {}
            _ => return Err(StorageError::new(StorageReason::RevisionConflict)),
        }
        let progress = load_progress(&mut transaction, operation_id).await?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(progress)
    }

    /// Transactionally upserts one bounded batch and advances monotonic counters.
    pub(crate) async fn upsert_scan_batch(
        &self,
        operation_id: Uuid,
        job_id: Uuid,
        records: &[ScanTrackRecord],
        job_counters: ScanCounters,
        updated_at_ms: i64,
    ) -> Result<ScanProgress, StorageError> {
        validate_v7(operation_id)?;
        validate_v7(job_id)?;
        validate_time(updated_at_ms)?;
        validate_counters(job_counters)?;
        if records.len() > MAX_SCAN_BATCH {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        let validated = validate_track_batch(records)?;
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        require_running_job(&mut transaction, operation_id, job_id, job_counters).await?;
        let root_id_text: String = sqlx::query_scalar(
            "SELECT root_id FROM scan_operation_roots WHERE operation_id = ? AND scan_job_id = ?",
        )
        .bind(operation_id.to_string())
        .bind(job_id.to_string())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| integrity_error())?;
        let root_id = parse_v7(&root_id_text)?;
        for record in validated {
            upsert_track(&mut transaction, root_id, job_id, &record, updated_at_ms).await?;
        }
        let job_update = sqlx::query(
            "UPDATE scan_jobs SET files_seen = ?, tracks_indexed = ?, errors_count = ? WHERE id = ? AND status = 'running'",
        )
        .bind(to_i64(job_counters.files_seen)?)
        .bind(to_i64(job_counters.tracks_indexed)?)
        .bind(to_i64(job_counters.errors_count)?)
        .bind(job_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        if job_update.rows_affected() != 1 {
            return Err(integrity_error());
        }
        let aggregate = aggregate_job_counters(&mut transaction, operation_id).await?;
        let updated = sqlx::query(
            "UPDATE scan_operations SET files_seen = ?, tracks_indexed = ?, errors_count = ? WHERE id = ? AND status = 'running'",
        )
        .bind(to_i64(aggregate.files_seen)?)
        .bind(to_i64(aggregate.tracks_indexed)?)
        .bind(to_i64(aggregate.errors_count)?)
        .bind(operation_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() != 1 {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(ScanProgress {
            operation_id,
            state: ScanOperationState::Running,
            counters: aggregate,
        })
    }

    /// Reads the current authoritative coarse progress.
    pub(crate) async fn load_scan_progress(
        &self,
        operation_id: Uuid,
    ) -> Result<ScanProgress, StorageError> {
        validate_v7(operation_id)?;
        let mut transaction = self.writer.begin().await.map_err(|_| read_error())?;
        let progress = load_progress(&mut transaction, operation_id).await?;
        transaction.commit().await.map_err(|_| read_error())?;
        Ok(progress)
    }

    /// Atomically terminalizes the operation, every root job, and its outbox record.
    pub(crate) async fn finish_scan_operation(
        &self,
        operation_id: Uuid,
        terminal: ScanTerminalState,
        successful_job_ids: &[Uuid],
        finished_at_ms: i64,
    ) -> Result<ScanTerminalRecord, StorageError> {
        let (record, changed) = self
            .terminalize_scan_operation(operation_id, terminal, successful_job_ids, finished_at_ms)
            .await?;
        if !changed && record.state != terminal {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        Ok(record)
    }

    /// Idempotently cancels one operation; already terminal operations are unchanged.
    pub(crate) async fn cancel_scan_operation(
        &self,
        operation_id: Uuid,
        successful_job_ids: &[Uuid],
        finished_at_ms: i64,
    ) -> Result<ScanCancelOutcome, StorageError> {
        let (record, changed) = self
            .terminalize_scan_operation(
                operation_id,
                ScanTerminalState::Cancelled,
                successful_job_ids,
                finished_at_ms,
            )
            .await?;
        Ok(if changed {
            ScanCancelOutcome::Cancelled(record)
        } else {
            ScanCancelOutcome::AlreadyTerminal(record)
        })
    }

    /// Converts a bounded startup batch of accepted/running operations to interrupted.
    pub(crate) async fn recover_interrupted_scan_operations(
        &self,
        limit: u32,
        recovered_at_ms: i64,
    ) -> Result<Vec<ScanTerminalRecord>, StorageError> {
        validate_outbox_limit(limit)?;
        validate_time(recovered_at_ms)?;
        let operation_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM scan_operations WHERE status IN ('accepted', 'running') ORDER BY id LIMIT ?",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let mut recovered = Vec::with_capacity(operation_ids.len());
        for operation_id in operation_ids {
            let operation_id = parse_v7(&operation_id)?;
            let (record, changed) = self
                .terminalize_scan_operation(
                    operation_id,
                    ScanTerminalState::Interrupted,
                    &[],
                    recovered_at_ms,
                )
                .await?;
            if !changed {
                return Err(integrity_error());
            }
            recovered.push(record);
        }
        Ok(recovered)
    }

    /// Reads a bounded strict terminal batch for startup/recovery delivery.
    pub(crate) async fn load_pending_scan_terminals(
        &self,
        limit: u32,
    ) -> Result<Vec<ScanTerminalRecord>, StorageError> {
        validate_outbox_limit(limit)?;
        let rows: Vec<SqliteRow> = sqlx::query(
            "SELECT e.id, e.aggregate_id, e.aggregate_revision, e.event_type, e.payload_json, e.created_at_ms, e.delivered_at_ms, s.status, s.files_seen, s.tracks_indexed, s.errors_count, s.finished_at_ms FROM outbox_events e JOIN scan_operations s ON s.id = e.aggregate_id WHERE e.aggregate_type = ? AND e.event_type = ? AND e.delivered_at_ms IS NULL ORDER BY e.created_at_ms, e.id LIMIT ?",
        )
        .bind(SCAN_AGGREGATE)
        .bind(SCAN_EVENT)
        .bind(i64::from(limit))
        .fetch_all(&self.writer)
        .await
        .map_err(|_| read_error())?;
        rows.iter().map(decode_terminal_row).collect()
    }

    /// Conditionally marks one scan terminal delivered; retention remains generic.
    pub(crate) async fn mark_scan_terminal_delivered(
        &self,
        outbox_id: Uuid,
        operation_id: Uuid,
        delivered_at_ms: i64,
    ) -> Result<bool, StorageError> {
        validate_v7(outbox_id)?;
        validate_v7(operation_id)?;
        validate_time(delivered_at_ms)?;
        let updated = sqlx::query(
            "UPDATE outbox_events SET delivered_at_ms = ? WHERE id = ? AND aggregate_type = ? AND aggregate_id = ? AND aggregate_revision = ? AND event_type = ? AND delivered_at_ms IS NULL AND created_at_ms <= ?",
        )
        .bind(delivered_at_ms)
        .bind(outbox_id.to_string())
        .bind(SCAN_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(TERMINAL_REVISION)
        .bind(SCAN_EVENT)
        .bind(delivered_at_ms)
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() == 1 {
            return Ok(true);
        }
        let row: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT delivered_at_ms FROM outbox_events WHERE id = ? AND aggregate_type = ? AND aggregate_id = ? AND aggregate_revision = ? AND event_type = ?",
        )
        .bind(outbox_id.to_string())
        .bind(SCAN_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(TERMINAL_REVISION)
        .bind(SCAN_EVENT)
        .fetch_optional(&self.writer)
        .await
        .map_err(|_| read_error())?;
        match row {
            Some(Some(_)) => Ok(false),
            Some(None) => Err(write_error()),
            None => Err(StorageError::new(StorageReason::EntityNotFound)),
        }
    }

    async fn terminalize_scan_operation(
        &self,
        operation_id: Uuid,
        terminal: ScanTerminalState,
        successful_job_ids: &[Uuid],
        finished_at_ms: i64,
    ) -> Result<(ScanTerminalRecord, bool), StorageError> {
        validate_v7(operation_id)?;
        validate_time(finished_at_ms)?;
        let successful_jobs = validate_requested_roots(successful_job_ids)?;
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let progress = load_progress(&mut transaction, operation_id).await?;
        if progress.state.is_terminal() {
            let record = load_terminal_for_operation(&mut transaction, operation_id).await?;
            transaction.commit().await.map_err(|_| read_error())?;
            return Ok((record, false));
        }
        let jobs: Vec<(String, String)> = sqlx::query_as(
            "SELECT scan_job_id, root_id FROM scan_operation_roots WHERE operation_id = ? ORDER BY scan_job_id",
        )
        .bind(operation_id.to_string())
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| read_error())?;
        if jobs.is_empty() {
            return Err(integrity_error());
        }
        let known_jobs: HashSet<Uuid> = jobs
            .iter()
            .map(|(job_id, _)| parse_v7(job_id))
            .collect::<Result<_, _>>()?;
        if !successful_jobs.is_subset(&known_jobs)
            || (terminal == ScanTerminalState::Completed && successful_jobs != known_jobs)
        {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        terminalize_scan_jobs(
            &mut transaction,
            &jobs,
            &successful_jobs,
            terminal,
            finished_at_ms,
        )
        .await?;
        let counters = aggregate_job_counters(&mut transaction, operation_id).await?;
        let operation_state = terminal.operation_state();
        let updated = sqlx::query(
            "UPDATE scan_operations SET status = ?, files_seen = ?, tracks_indexed = ?, errors_count = ?, finished_at_ms = ?, error_code = ? WHERE id = ? AND status IN ('accepted', 'running')",
        )
        .bind(operation_state.as_str())
        .bind(to_i64(counters.files_seen)?)
        .bind(to_i64(counters.tracks_indexed)?)
        .bind(to_i64(counters.errors_count)?)
        .bind(finished_at_ms)
        .bind(terminal.error_code())
        .bind(operation_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() != 1 {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let outbox_id = Uuid::now_v7();
        let payload = serde_json::to_string(&StoredScanTerminal {
            schema_version: "1.0.0".to_owned(),
            operation_id,
            state: terminal.into(),
            files_seen: counters.files_seen,
            tracks_indexed: counters.tracks_indexed,
            errors_count: counters.errors_count,
        })
        .map_err(|_| write_error())?;
        sqlx::query(
            "INSERT INTO outbox_events(id, aggregate_type, aggregate_id, aggregate_revision, event_type, payload_json, created_at_ms, delivered_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(outbox_id.to_string())
        .bind(SCAN_AGGREGATE)
        .bind(operation_id.to_string())
        .bind(TERMINAL_REVISION)
        .bind(SCAN_EVENT)
        .bind(payload)
        .bind(finished_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok((
            ScanTerminalRecord {
                outbox_id,
                operation_id,
                state: terminal,
                counters,
                occurred_at_ms: finished_at_ms,
                delivered_at_ms: None,
            },
            true,
        ))
    }
}

async fn load_and_validate_roots(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<StoredRoot>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, canonical_path, path_key, display_name, enabled FROM library_roots ORDER BY created_at_ms, id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| read_error())?;
    rows.iter().map(validate_root_row).collect()
}

async fn terminalize_scan_jobs(
    transaction: &mut Transaction<'_, Sqlite>,
    jobs: &[(String, String)],
    successful_jobs: &HashSet<Uuid>,
    terminal: ScanTerminalState,
    finished_at_ms: i64,
) -> Result<(), StorageError> {
    for (job_id_text, root_id_text) in jobs {
        let job_id = parse_v7(job_id_text)?;
        let successful = successful_jobs.contains(&job_id);
        let job_status = if successful {
            "completed"
        } else {
            terminal.job_status()
        };
        if successful {
            sqlx::query(
                "UPDATE tracks SET availability = 'missing', updated_at_ms = ? WHERE root_id = ? AND (last_seen_scan_id IS NULL OR last_seen_scan_id != ?) AND availability != 'missing'",
            )
            .bind(finished_at_ms)
            .bind(root_id_text)
            .bind(job_id_text)
            .execute(&mut **transaction)
            .await
            .map_err(|_| write_error())?;
            sqlx::query("UPDATE library_roots SET last_scan_at_ms = ? WHERE id = ?")
                .bind(finished_at_ms)
                .bind(root_id_text)
                .execute(&mut **transaction)
                .await
                .map_err(|_| write_error())?;
        }
        let updated = sqlx::query(
            "UPDATE scan_jobs SET status = ?, finished_at_ms = ?, error_code = ? WHERE id = ? AND status IN ('queued', 'running')",
        )
        .bind(job_status)
        .bind(finished_at_ms)
        .bind(if successful { None } else { terminal.error_code() })
        .bind(job_id_text)
        .execute(&mut **transaction)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() != 1 {
            return Err(integrity_error());
        }
    }
    Ok(())
}

fn validate_root_row(row: &SqliteRow) -> Result<StoredRoot, StorageError> {
    let root_id_text: String = row.try_get("id").map_err(|_| integrity_error())?;
    let root_id = parse_v7(&root_id_text)?;
    if root_id.to_string() != root_id_text {
        return Err(integrity_error());
    }
    let canonical_text: String = row
        .try_get("canonical_path")
        .map_err(|_| integrity_error())?;
    let stored_key: String = row.try_get("path_key").map_err(|_| integrity_error())?;
    let display_name: String = row.try_get("display_name").map_err(|_| integrity_error())?;
    let enabled: i64 = row.try_get("enabled").map_err(|_| integrity_error())?;
    if enabled != 0 && enabled != 1 {
        return Err(integrity_error());
    }
    let canonical_path = PathBuf::from(&canonical_text);
    if !canonical_path.is_absolute()
        || canonical_text.is_empty()
        || canonical_text.chars().count() > MAX_PATH_CHARS
        || path_key(&canonical_text)? != stored_key
        || display_name_for_path(&canonical_path)? != display_name
    {
        return Err(integrity_error());
    }
    if enabled == 1 && !authorized_root_is_current(&canonical_path, &stored_key) {
        return Err(StorageError::new(StorageReason::PathDenied));
    }
    Ok(StoredRoot {
        root_id,
        enabled: enabled == 1,
        canonical_path,
    })
}

fn select_roots(
    roots: Vec<StoredRoot>,
    requested: &HashSet<Uuid>,
) -> Result<Vec<StoredRoot>, StorageError> {
    if requested.is_empty() {
        return Ok(roots.into_iter().filter(|root| root.enabled).collect());
    }
    let mut selected = Vec::with_capacity(requested.len());
    for root in roots {
        if requested.contains(&root.root_id) {
            if !root.enabled {
                return Err(StorageError::new(StorageReason::EntityNotFound));
            }
            selected.push(root);
        }
    }
    if selected.len() != requested.len() {
        return Err(StorageError::new(StorageReason::EntityNotFound));
    }
    Ok(selected)
}

fn validate_requested_roots(values: &[Uuid]) -> Result<HashSet<Uuid>, StorageError> {
    let mut result = HashSet::with_capacity(values.len());
    for value in values {
        validate_v7(*value)?;
        if !result.insert(*value) {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
    }
    Ok(result)
}

async fn load_progress(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: Uuid,
) -> Result<ScanProgress, StorageError> {
    let row = sqlx::query(
        "SELECT status, files_seen, tracks_indexed, errors_count, started_at_ms, finished_at_ms, error_code FROM scan_operations WHERE id = ?",
    )
    .bind(operation_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| read_error())?
    .ok_or_else(|| StorageError::new(StorageReason::EntityNotFound))?;
    let state = parse_operation_state(
        &row.try_get::<String, _>("status")
            .map_err(|_| integrity_error())?,
    )?;
    let counters = ScanCounters {
        files_seen: parse_counter(&row, "files_seen")?,
        tracks_indexed: parse_counter(&row, "tracks_indexed")?,
        errors_count: parse_counter(&row, "errors_count")?,
    };
    validate_counters(counters)?;
    let started: Option<i64> = row
        .try_get("started_at_ms")
        .map_err(|_| integrity_error())?;
    let finished: Option<i64> = row
        .try_get("finished_at_ms")
        .map_err(|_| integrity_error())?;
    let error_code: Option<String> = row.try_get("error_code").map_err(|_| integrity_error())?;
    if started.is_some_and(|value| value < 0)
        || finished.is_some_and(|value| value < 0)
        || !valid_operation_shape(state, started, finished, error_code.as_deref())
    {
        return Err(integrity_error());
    }
    Ok(ScanProgress {
        operation_id,
        state,
        counters,
    })
}

fn valid_operation_shape(
    state: ScanOperationState,
    started: Option<i64>,
    finished: Option<i64>,
    error_code: Option<&str>,
) -> bool {
    match state {
        ScanOperationState::Accepted => {
            started.is_none() && finished.is_none() && error_code.is_none()
        }
        ScanOperationState::Running => {
            started.is_some() && finished.is_none() && error_code.is_none()
        }
        ScanOperationState::Completed => {
            started.is_some() && finished.is_some() && error_code.is_none()
        }
        ScanOperationState::Cancelled => finished.is_some() && error_code.is_none(),
        ScanOperationState::Failed => {
            finished.is_some() && matches!(error_code, Some("scan_failed"))
        }
        ScanOperationState::Interrupted => {
            finished.is_some() && matches!(error_code, Some("interrupted"))
        }
    }
}

fn parse_operation_state(value: &str) -> Result<ScanOperationState, StorageError> {
    match value {
        "accepted" => Ok(ScanOperationState::Accepted),
        "running" => Ok(ScanOperationState::Running),
        "completed" => Ok(ScanOperationState::Completed),
        "cancelled" => Ok(ScanOperationState::Cancelled),
        "failed" => Ok(ScanOperationState::Failed),
        "interrupted" => Ok(ScanOperationState::Interrupted),
        _ => Err(integrity_error()),
    }
}

async fn require_running_job(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: Uuid,
    job_id: Uuid,
    next: ScanCounters,
) -> Result<(), StorageError> {
    let progress = load_progress(transaction, operation_id).await?;
    if progress.state != ScanOperationState::Running {
        return Err(StorageError::new(StorageReason::RevisionConflict));
    }
    let row = sqlx::query(
        "SELECT j.status, j.files_seen, j.tracks_indexed, j.errors_count FROM scan_jobs j JOIN scan_operation_roots r ON r.scan_job_id = j.id WHERE r.operation_id = ? AND r.scan_job_id = ?",
    )
    .bind(operation_id.to_string())
    .bind(job_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| read_error())?
    .ok_or_else(|| StorageError::new(StorageReason::EntityNotFound))?;
    let status: String = row.try_get("status").map_err(|_| integrity_error())?;
    let current = ScanCounters {
        files_seen: parse_counter(&row, "files_seen")?,
        tracks_indexed: parse_counter(&row, "tracks_indexed")?,
        errors_count: parse_counter(&row, "errors_count")?,
    };
    if status != "running"
        || next.files_seen < current.files_seen
        || next.tracks_indexed < current.tracks_indexed
        || next.errors_count < current.errors_count
    {
        return Err(StorageError::new(StorageReason::RevisionConflict));
    }
    Ok(())
}

async fn aggregate_job_counters(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: Uuid,
) -> Result<ScanCounters, StorageError> {
    let row = sqlx::query(
        "SELECT COALESCE(SUM(j.files_seen), 0) AS files_seen, COALESCE(SUM(j.tracks_indexed), 0) AS tracks_indexed, COALESCE(SUM(j.errors_count), 0) AS errors_count FROM scan_jobs j JOIN scan_operation_roots r ON r.scan_job_id = j.id WHERE r.operation_id = ?",
    )
    .bind(operation_id.to_string())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| read_error())?;
    let counters = ScanCounters {
        files_seen: parse_counter(&row, "files_seen")?,
        tracks_indexed: parse_counter(&row, "tracks_indexed")?,
        errors_count: parse_counter(&row, "errors_count")?,
    };
    validate_counters(counters)?;
    Ok(counters)
}

fn parse_counter(row: &SqliteRow, column: &str) -> Result<u64, StorageError> {
    let value: i64 = row.try_get(column).map_err(|_| integrity_error())?;
    u64::try_from(value).map_err(|_| integrity_error())
}

fn validate_track_batch(
    records: &[ScanTrackRecord],
) -> Result<Vec<ValidatedTrack<'_>>, StorageError> {
    let mut paths = HashSet::with_capacity(records.len());
    let mut validated = Vec::with_capacity(records.len());
    for record in records {
        let relative_path = normalize_relative_path(&record.relative_path)?;
        let relative_path_key = relative_path.to_lowercase();
        if !paths.insert(relative_path_key.clone()) {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        if let Some(identity) = &record.file_identity
            && (identity.is_empty()
                || identity.chars().count() > MAX_FILE_IDENTITY_CHARS
                || identity.chars().any(char::is_control))
        {
            return Err(StorageError::new(StorageReason::InvalidSetting));
        }
        validate_track_record(record)?;
        let genres_json = serde_json::to_string(&record.genres).map_err(|_| write_error())?;
        validated.push(ValidatedTrack {
            record,
            relative_path,
            relative_path_key,
            genres_json,
            file_size_bytes: i64::try_from(record.file_size_bytes)
                .map_err(|_| StorageError::new(StorageReason::InvalidSetting))?,
            duration_ms: i64::try_from(record.duration_ms)
                .map_err(|_| StorageError::new(StorageReason::InvalidSetting))?,
        });
    }
    Ok(validated)
}

fn validate_track_record(record: &ScanTrackRecord) -> Result<(), StorageError> {
    if record.file_size_bytes > MAX_JS_SAFE_INTEGER
        || record.duration_ms > MAX_JS_SAFE_INTEGER
        || record.modified_at_ms < 0
        || record.format.is_empty()
        || record.format.len() > 32
        || !record
            .format
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'_')
        || !record.metadata_confidence.is_finite()
        || !(0.0..=1.0).contains(&record.metadata_confidence)
        || record
            .track_number
            .is_some_and(|value| value == 0 || value > 10_000)
        || record
            .disc_number
            .is_some_and(|value| value == 0 || value > 10_000)
        || record
            .year
            .is_some_and(|value| !(0..=9_999).contains(&value))
        || record.genres.len() > MAX_GENRES
        || record.genres.iter().any(|value| {
            value.is_empty() || value.chars().count() > MAX_GENRE_CHARS || value.contains('\0')
        })
        || [
            record.title.as_deref(),
            record.artist.as_deref(),
            record.album.as_deref(),
            record.album_artist.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.chars().count() > MAX_TAG_CHARS || value.contains('\0'))
        || record.embedded_cover_hash.as_deref().is_some_and(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(StorageError::new(StorageReason::InvalidSetting));
    }
    Ok(())
}

fn normalize_relative_path(path: &Path) -> Result<String, StorageError> {
    if path.is_absolute() {
        return Err(StorageError::new(StorageReason::PathOutsideScope));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(value) = component else {
            return Err(StorageError::new(StorageReason::PathOutsideScope));
        };
        let value = value
            .to_str()
            .filter(|value| {
                !value.is_empty() && !value.contains(':') && !value.chars().any(char::is_control)
            })
            .ok_or_else(|| StorageError::new(StorageReason::PathOutsideScope))?;
        parts.push(value);
    }
    let normalized = parts.join("/");
    if normalized.is_empty() || normalized.chars().count() > MAX_PATH_CHARS {
        return Err(StorageError::new(StorageReason::PathOutsideScope));
    }
    Ok(normalized)
}

async fn upsert_track(
    transaction: &mut Transaction<'_, Sqlite>,
    root_id: Uuid,
    job_id: Uuid,
    validated: &ValidatedTrack<'_>,
    updated_at_ms: i64,
) -> Result<Uuid, StorageError> {
    let identity_matches: Vec<(String, Option<String>)> = if let Some(identity) =
        &validated.record.file_identity
    {
        sqlx::query_as(
            "SELECT id, last_seen_scan_id FROM tracks WHERE root_id = ? AND file_identity = ? ORDER BY id LIMIT 2",
        )
        .bind(root_id.to_string())
        .bind(identity)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|_| read_error())?
    } else {
        Vec::new()
    };
    let mut validated_identity_matches = Vec::with_capacity(identity_matches.len());
    for (track_id, last_seen_scan_id) in identity_matches {
        let track_id = parse_v7(&track_id)?;
        let last_seen_scan_id = last_seen_scan_id.as_deref().map(parse_v7).transpose()?;
        validated_identity_matches.push((track_id, last_seen_scan_id));
    }
    let path_match: Option<String> =
        sqlx::query_scalar("SELECT id FROM tracks WHERE root_id = ? AND relative_path_key = ?")
            .bind(root_id.to_string())
            .bind(&validated.relative_path_key)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| read_error())?;

    let existing_id = if let Some(path_id) = path_match {
        Some(parse_v7(&path_id)?)
    } else if validated_identity_matches.len() == 1
        && validated_identity_matches[0].1 != Some(job_id)
    {
        Some(validated_identity_matches[0].0)
    } else {
        None
    };
    let track_id = existing_id.unwrap_or_else(Uuid::now_v7);
    if existing_id.is_some() {
        let updated = bind_track_update(
            sqlx::query(
                "UPDATE tracks SET relative_path = ?, relative_path_key = ?, file_identity = ?, availability = ?, format = ?, file_size_bytes = ?, modified_at_ms = ?, duration_ms = ?, last_seen_scan_id = ?, title = ?, artist = ?, album = ?, album_artist = ?, track_number = ?, disc_number = ?, year = ?, genre_json = ?, metadata_confidence = ?, embedded_cover_hash = ?, updated_at_ms = ? WHERE id = ? AND root_id = ?",
            ),
            validated,
            job_id,
            updated_at_ms,
        )
        .bind(track_id.to_string())
        .bind(root_id.to_string())
        .execute(&mut **transaction)
        .await
        .map_err(|_| write_error())?;
        if updated.rows_affected() != 1 {
            return Err(integrity_error());
        }
    } else {
        bind_track_insert(
            sqlx::query(
                "INSERT INTO tracks(id, root_id, relative_path, relative_path_key, file_identity, availability, format, file_size_bytes, modified_at_ms, duration_ms, last_seen_scan_id, title, artist, album, album_artist, track_number, disc_number, year, genre_json, metadata_confidence, embedded_cover_hash, created_at_ms, updated_at_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            ),
            track_id,
            root_id,
            validated,
            job_id,
            updated_at_ms,
        )
        .execute(&mut **transaction)
        .await
        .map_err(|_| write_error())?;
    }
    Ok(track_id)
}

fn bind_track_update<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments>,
    validated: &'q ValidatedTrack<'q>,
    job_id: Uuid,
    updated_at_ms: i64,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments> {
    bind_track_values(query, validated, job_id, updated_at_ms)
}

fn bind_track_insert<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments>,
    track_id: Uuid,
    root_id: Uuid,
    validated: &'q ValidatedTrack<'q>,
    job_id: Uuid,
    updated_at_ms: i64,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments> {
    bind_track_values(
        query.bind(track_id.to_string()).bind(root_id.to_string()),
        validated,
        job_id,
        updated_at_ms,
    )
    .bind(updated_at_ms)
}

fn bind_track_values<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments>,
    validated: &'q ValidatedTrack<'q>,
    job_id: Uuid,
    updated_at_ms: i64,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments> {
    let record = validated.record;
    query
        .bind(&validated.relative_path)
        .bind(&validated.relative_path_key)
        .bind(&record.file_identity)
        .bind(record.availability.as_str())
        .bind(&record.format)
        .bind(validated.file_size_bytes)
        .bind(record.modified_at_ms)
        .bind(validated.duration_ms)
        .bind(job_id.to_string())
        .bind(&record.title)
        .bind(&record.artist)
        .bind(&record.album)
        .bind(&record.album_artist)
        .bind(record.track_number.map(i64::from))
        .bind(record.disc_number.map(i64::from))
        .bind(record.year.map(i64::from))
        .bind(&validated.genres_json)
        .bind(record.metadata_confidence)
        .bind(&record.embedded_cover_hash)
        .bind(updated_at_ms)
}

async fn load_terminal_for_operation(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: Uuid,
) -> Result<ScanTerminalRecord, StorageError> {
    let rows = sqlx::query(
        "SELECT e.id, e.aggregate_id, e.aggregate_revision, e.event_type, e.payload_json, e.created_at_ms, e.delivered_at_ms, s.status, s.files_seen, s.tracks_indexed, s.errors_count, s.finished_at_ms FROM outbox_events e JOIN scan_operations s ON s.id = e.aggregate_id WHERE e.aggregate_type = ? AND e.aggregate_id = ? AND e.event_type = ?",
    )
    .bind(SCAN_AGGREGATE)
    .bind(operation_id.to_string())
    .bind(SCAN_EVENT)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| read_error())?;
    if rows.len() != 1 {
        return Err(integrity_error());
    }
    decode_terminal_row(&rows[0])
}

fn decode_terminal_row(row: &SqliteRow) -> Result<ScanTerminalRecord, StorageError> {
    let outbox_id = parse_v7(
        &row.try_get::<String, _>("id")
            .map_err(|_| integrity_error())?,
    )?;
    let operation_id = parse_v7(
        &row.try_get::<String, _>("aggregate_id")
            .map_err(|_| integrity_error())?,
    )?;
    let revision: i64 = row
        .try_get("aggregate_revision")
        .map_err(|_| integrity_error())?;
    let event_type: String = row.try_get("event_type").map_err(|_| integrity_error())?;
    let occurred_at_ms: i64 = row
        .try_get("created_at_ms")
        .map_err(|_| integrity_error())?;
    let delivered_at_ms: Option<i64> = row
        .try_get("delivered_at_ms")
        .map_err(|_| integrity_error())?;
    let finished_at_ms: Option<i64> = row
        .try_get("finished_at_ms")
        .map_err(|_| integrity_error())?;
    let payload: StoredScanTerminal = serde_json::from_str(
        &row.try_get::<String, _>("payload_json")
            .map_err(|_| integrity_error())?,
    )
    .map_err(|_| integrity_error())?;
    let state = ScanTerminalState::from(payload.state);
    let database_state = parse_operation_state(
        &row.try_get::<String, _>("status")
            .map_err(|_| integrity_error())?,
    )?;
    let counters = ScanCounters {
        files_seen: parse_counter(row, "files_seen")?,
        tracks_indexed: parse_counter(row, "tracks_indexed")?,
        errors_count: parse_counter(row, "errors_count")?,
    };
    validate_counters(counters)?;
    if revision != TERMINAL_REVISION
        || event_type != SCAN_EVENT
        || occurred_at_ms < 0
        || delivered_at_ms.is_some_and(|value| value < occurred_at_ms)
        || finished_at_ms != Some(occurred_at_ms)
        || payload.schema_version != "1.0.0"
        || payload.operation_id != operation_id
        || state.operation_state() != database_state
        || payload.files_seen != counters.files_seen
        || payload.tracks_indexed != counters.tracks_indexed
        || payload.errors_count != counters.errors_count
    {
        return Err(integrity_error());
    }
    Ok(ScanTerminalRecord {
        outbox_id,
        operation_id,
        state,
        counters,
        occurred_at_ms,
        delivered_at_ms,
    })
}

fn validate_counters(counters: ScanCounters) -> Result<(), StorageError> {
    if counters.files_seen > MAX_JS_SAFE_INTEGER
        || counters.tracks_indexed > MAX_JS_SAFE_INTEGER
        || counters.errors_count > MAX_JS_SAFE_INTEGER
        || counters.tracks_indexed > counters.files_seen
        || counters.errors_count > counters.files_seen
    {
        return Err(StorageError::new(StorageReason::InvalidSetting));
    }
    Ok(())
}

fn parse_v7(value: &str) -> Result<Uuid, StorageError> {
    let id = Uuid::parse_str(value).map_err(|_| integrity_error())?;
    if id.is_nil() || id.get_version_num() != 7 || id.to_string() != value {
        Err(integrity_error())
    } else {
        Ok(id)
    }
}

fn validate_v7(value: Uuid) -> Result<(), StorageError> {
    if value.is_nil() || value.get_version_num() != 7 {
        Err(StorageError::new(StorageReason::InvalidSetting))
    } else {
        Ok(())
    }
}

fn validate_time(value: i64) -> Result<(), StorageError> {
    if value < 0 {
        Err(StorageError::new(StorageReason::InvalidSetting))
    } else {
        Ok(())
    }
}

fn validate_app_version(value: &str) -> Result<(), StorageError> {
    if value.is_empty() || value.len() > 100 || value.chars().any(char::is_control) {
        Err(StorageError::new(StorageReason::InvalidSetting))
    } else {
        Ok(())
    }
}

fn validate_outbox_limit(value: u32) -> Result<(), StorageError> {
    if (1..=MAX_OUTBOX_BATCH).contains(&value) {
        Ok(())
    } else {
        Err(StorageError::new(StorageReason::InvalidSetting))
    }
}

fn to_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| integrity_error())
}

fn display_name_for_path(path: &Path) -> Result<String, StorageError> {
    let display_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Local library");
    if display_name.is_empty()
        || display_name.chars().count() > 255
        || display_name.chars().any(char::is_control)
        || display_name.contains(['/', '\\'])
    {
        return Err(integrity_error());
    }
    Ok(display_name.to_owned())
}

fn path_key(value: &str) -> Result<String, StorageError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(integrity_error());
    }
    let normalized = value.replace('\\', "/").to_lowercase();
    let trimmed = normalized.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    })
}

fn authorized_root_is_current(path: &Path, stored_key: &str) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || is_reparse_or_symlink(&metadata) {
        return false;
    }
    let Ok(current) = std::fs::canonicalize(path) else {
        return false;
    };
    if !current.is_absolute() {
        return false;
    }
    let Ok(current_metadata) = std::fs::symlink_metadata(&current) else {
        return false;
    };
    current_metadata.is_dir()
        && !is_reparse_or_symlink(&current_metadata)
        && current
            .to_str()
            .and_then(|value| path_key(value).ok())
            .is_some_and(|key| key == stored_key)
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

const fn read_error() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
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
    use std::collections::BTreeMap;

    async fn fixture() -> (tempfile::TempDir, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped app paths");
        let storage = Storage::open(&paths, "0.3.0").await.expect("storage");
        let repository = storage.repository();
        (temp, storage, repository)
    }

    async fn add_root(repository: &Repository, path: &Path, now_ms: i64) -> Uuid {
        std::fs::create_dir_all(path).expect("library root");
        repository
            .add_library_root(path, now_ms)
            .await
            .expect("authorize root")
            .root
            .root_id
    }

    async fn accept_and_start(
        repository: &Repository,
        root_id: Uuid,
        accepted_at_ms: i64,
    ) -> AuthorizedScanOperation {
        let operation = repository
            .accept_scan_operation(Uuid::now_v7(), &[root_id], accepted_at_ms, "0.3.0")
            .await
            .expect("accept scan");
        repository
            .mark_scan_operation_running(operation.operation_id(), accepted_at_ms + 1)
            .await
            .expect("start scan");
        operation
    }

    fn track(
        relative_path: &str,
        file_identity: Option<&str>,
        availability: ScanTrackAvailability,
    ) -> ScanTrackRecord {
        ScanTrackRecord {
            relative_path: PathBuf::from(relative_path),
            file_identity: file_identity.map(ToOwned::to_owned),
            availability,
            format: "mp3".to_owned(),
            file_size_bytes: 64,
            modified_at_ms: 50,
            duration_ms: 1_000,
            title: Some("Fixture".to_owned()),
            artist: None,
            album: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            year: None,
            genres: vec!["Test".to_owned()],
            metadata_confidence: if availability == ScanTrackAvailability::Available {
                1.0
            } else {
                0.0
            },
            embedded_cover_hash: None,
        }
    }

    async fn track_rows(
        repository: &Repository,
        root_id: Uuid,
    ) -> BTreeMap<String, (String, Option<String>, String)> {
        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, relative_path, file_identity, availability FROM tracks WHERE root_id = ? ORDER BY relative_path",
        )
        .bind(root_id.to_string())
        .fetch_all(&repository.writer)
        .await
        .expect("track rows");
        rows.into_iter()
            .map(|(id, path, identity, availability)| (path, (id, identity, availability)))
            .collect()
    }

    #[tokio::test]
    async fn empty_root_selection_is_authorized_and_active_roots_are_busy() {
        let (temp, storage, repository) = fixture().await;
        let first_root = add_root(&repository, &temp.path().join("First"), 10).await;
        let second_root = add_root(&repository, &temp.path().join("Second"), 20).await;
        let operation_id = Uuid::now_v7();
        let operation = repository
            .accept_scan_operation(operation_id, &[], 30, "0.3.0")
            .await
            .expect("accept all enabled roots");
        assert_eq!(operation.operation_id(), operation_id);
        assert_eq!(operation.roots().len(), 2);
        assert_ne!(first_root, second_root);
        let canonical_temp = std::fs::canonicalize(temp.path()).expect("canonical temporary root");
        assert!(operation.roots().iter().all(|root| {
            root.canonical_path().is_absolute()
                && root.canonical_path().starts_with(&canonical_temp)
        }));

        let Err(busy) = repository
            .accept_scan_operation(Uuid::now_v7(), &[first_root], 40, "0.3.0")
            .await
        else {
            panic!("active root must reject a second operation");
        };
        assert_eq!(busy.reason(), StorageReason::ResourceBusy);
        let operation_count: i64 = sqlx::query_scalar("SELECT count(*) FROM scan_operations")
            .fetch_one(&repository.writer)
            .await
            .expect("operation count");
        assert_eq!(operation_count, 1);
        drop(operation);
        drop(repository);
        storage.close().await;
    }

    #[tokio::test]
    async fn track_upsert_preserves_safe_matches_without_conflating_identity_collisions() {
        let (temp, storage, repository) = fixture().await;
        let root_id = add_root(&repository, &temp.path().join("Music"), 10).await;
        let first = accept_and_start(&repository, root_id, 20).await;
        let first_job = first.roots()[0].job_id();
        let first_records = vec![
            track(
                "old/a.mp3",
                Some("stable-a"),
                ScanTrackAvailability::Available,
            ),
            track("b.mp3", Some("collision"), ScanTrackAvailability::Corrupt),
            track(
                "c.mp3",
                Some("collision"),
                ScanTrackAvailability::Unsupported,
            ),
            track(
                "old-only.mp3",
                Some("old-only"),
                ScanTrackAvailability::Available,
            ),
        ];
        repository
            .upsert_scan_batch(
                first.operation_id(),
                first_job,
                &first_records,
                ScanCounters {
                    files_seen: 4,
                    tracks_indexed: 4,
                    errors_count: 2,
                },
                30,
            )
            .await
            .expect("persist first batch");
        let before = track_rows(&repository, root_id).await;
        assert_eq!(before["b.mp3"].2, "corrupt");
        assert_eq!(before["c.mp3"].2, "unsupported");
        repository
            .finish_scan_operation(
                first.operation_id(),
                ScanTerminalState::Completed,
                &[first_job],
                40,
            )
            .await
            .expect("finish first scan");

        let second = accept_and_start(&repository, root_id, 50).await;
        let second_job = second.roots()[0].job_id();
        let second_records = vec![
            track("d.mp3", Some("collision"), ScanTrackAvailability::Available),
            track(
                "moved/a.mp3",
                Some("stable-a"),
                ScanTrackAvailability::Available,
            ),
            track(
                "b.mp3",
                Some("replacement-b"),
                ScanTrackAvailability::Available,
            ),
        ];
        repository
            .upsert_scan_batch(
                second.operation_id(),
                second_job,
                &second_records,
                ScanCounters {
                    files_seen: 3,
                    tracks_indexed: 3,
                    errors_count: 0,
                },
                60,
            )
            .await
            .expect("persist second batch");
        repository
            .finish_scan_operation(
                second.operation_id(),
                ScanTerminalState::Completed,
                &[second_job],
                70,
            )
            .await
            .expect("finish second scan");

        let after = track_rows(&repository, root_id).await;
        assert_eq!(after["moved/a.mp3"].0, before["old/a.mp3"].0);
        assert_eq!(after["b.mp3"].0, before["b.mp3"].0);
        assert_eq!(after["b.mp3"].1.as_deref(), Some("replacement-b"));
        assert_ne!(after["d.mp3"].0, before["b.mp3"].0);
        assert_ne!(after["d.mp3"].0, before["c.mp3"].0);
        assert_eq!(after["c.mp3"].2, "missing");
        assert_eq!(after["old-only.mp3"].2, "missing");
        drop(repository);
        storage.close().await;
    }

    #[tokio::test]
    async fn cancellation_is_idempotent_and_does_not_mark_unfinished_roots_missing() {
        let (temp, storage, repository) = fixture().await;
        let root_id = add_root(&repository, &temp.path().join("Music"), 10).await;
        let seed = accept_and_start(&repository, root_id, 20).await;
        let seed_job = seed.roots()[0].job_id();
        repository
            .upsert_scan_batch(
                seed.operation_id(),
                seed_job,
                &[track(
                    "keep.mp3",
                    Some("keep"),
                    ScanTrackAvailability::Available,
                )],
                ScanCounters {
                    files_seen: 1,
                    tracks_indexed: 1,
                    errors_count: 0,
                },
                30,
            )
            .await
            .expect("seed track");
        repository
            .finish_scan_operation(
                seed.operation_id(),
                ScanTerminalState::Completed,
                &[seed_job],
                40,
            )
            .await
            .expect("seed finish");

        let cancelled = accept_and_start(&repository, root_id, 50).await;
        let first_outcome = repository
            .cancel_scan_operation(cancelled.operation_id(), &[], 60)
            .await
            .expect("cancel scan");
        let ScanCancelOutcome::Cancelled(first_terminal) = first_outcome else {
            panic!("first cancellation must transition");
        };
        let second_outcome = repository
            .cancel_scan_operation(cancelled.operation_id(), &[], 70)
            .await
            .expect("repeat cancellation");
        let ScanCancelOutcome::AlreadyTerminal(second_terminal) = second_outcome else {
            panic!("second cancellation must be idempotent");
        };
        assert_eq!(second_terminal, first_terminal);
        assert_eq!(
            track_rows(&repository, root_id).await["keep.mp3"].2,
            "available"
        );
        let terminal_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox_events WHERE aggregate_type = ? AND aggregate_id = ?",
        )
        .bind(SCAN_AGGREGATE)
        .bind(cancelled.operation_id().to_string())
        .fetch_one(&repository.writer)
        .await
        .expect("terminal count");
        assert_eq!(terminal_count, 1);
        drop(repository);
        storage.close().await;
    }

    #[tokio::test]
    async fn terminal_outbox_is_atomic_path_free_and_startup_recovery_is_replayable() {
        let (temp, storage, repository) = fixture().await;
        let first_path = temp.path().join("Sensitive-Canary-First");
        let second_path = temp.path().join("Sensitive-Canary-Second");
        let first_root = add_root(&repository, &first_path, 10).await;
        let second_root = add_root(&repository, &second_path, 20).await;
        let accepted = repository
            .accept_scan_operation(Uuid::now_v7(), &[first_root], 30, "0.3.0")
            .await
            .expect("accepted operation");
        let running = accept_and_start(&repository, second_root, 40).await;

        sqlx::query(
            "CREATE TRIGGER fixture_fail_scan_outbox BEFORE INSERT ON outbox_events WHEN NEW.aggregate_type = 'library_scan_operation' BEGIN SELECT RAISE(ABORT, 'fixture'); END",
        )
        .execute(&repository.writer)
        .await
        .expect("failure trigger");
        let failed_write = repository
            .finish_scan_operation(running.operation_id(), ScanTerminalState::Failed, &[], 50)
            .await
            .expect_err("outbox failure must roll back terminal state");
        assert_eq!(failed_write.reason(), StorageReason::StorageWriteFailed);
        assert_eq!(
            repository
                .load_scan_progress(running.operation_id())
                .await
                .expect("rolled-back operation")
                .state,
            ScanOperationState::Running
        );
        sqlx::query("DROP TRIGGER fixture_fail_scan_outbox")
            .execute(&repository.writer)
            .await
            .expect("drop failure trigger");

        let mut recovered = repository
            .recover_interrupted_scan_operations(100, 60)
            .await
            .expect("recover active operations");
        recovered.sort_by_key(|record| record.operation_id);
        assert_eq!(recovered.len(), 2);
        assert!(recovered.iter().all(|record| {
            record.state == ScanTerminalState::Interrupted && record.delivered_at_ms.is_none()
        }));
        assert!(
            repository
                .recover_interrupted_scan_operations(100, 70)
                .await
                .expect("recovery is bounded and repeat-safe")
                .is_empty()
        );

        let pending = repository
            .load_pending_scan_terminals(100)
            .await
            .expect("strict pending terminals");
        assert_eq!(pending.len(), 2);
        let payloads: Vec<String> = sqlx::query_scalar(
            "SELECT payload_json FROM outbox_events WHERE aggregate_type = ? ORDER BY id",
        )
        .bind(SCAN_AGGREGATE)
        .fetch_all(&repository.writer)
        .await
        .expect("stored terminal payloads");
        for payload in payloads {
            let lowercase = payload.to_lowercase();
            assert!(!lowercase.contains("path"));
            assert!(!lowercase.contains("secret"));
            assert!(!payload.contains("Sensitive-Canary"));
        }
        let first_pending = pending[0];
        assert!(
            repository
                .mark_scan_terminal_delivered(
                    first_pending.outbox_id,
                    first_pending.operation_id,
                    80,
                )
                .await
                .expect("first delivery mark")
        );
        assert!(
            !repository
                .mark_scan_terminal_delivered(
                    first_pending.outbox_id,
                    first_pending.operation_id,
                    90,
                )
                .await
                .expect("delivery mark is idempotent")
        );
        let terminal_conflict = repository
            .finish_scan_operation(
                accepted.operation_id(),
                ScanTerminalState::Completed,
                &[accepted.roots()[0].job_id()],
                100,
            )
            .await
            .expect_err("a terminal operation cannot change terminal kind");
        assert_eq!(terminal_conflict.reason(), StorageReason::RevisionConflict);
        drop(repository);
        storage.close().await;
    }

    #[tokio::test]
    async fn invalid_or_oversized_batches_are_rejected_without_partial_writes() {
        let (temp, storage, repository) = fixture().await;
        let root_id = add_root(&repository, &temp.path().join("Music"), 10).await;
        let operation = accept_and_start(&repository, root_id, 20).await;
        let job_id = operation.roots()[0].job_id();
        let oversized: Vec<_> = (0..=MAX_SCAN_BATCH)
            .map(|index| {
                track(
                    &format!("{index}.mp3"),
                    None,
                    ScanTrackAvailability::Available,
                )
            })
            .collect();
        let oversized_error = repository
            .upsert_scan_batch(
                operation.operation_id(),
                job_id,
                &oversized,
                ScanCounters {
                    files_seen: u64::try_from(oversized.len()).expect("bounded fixture"),
                    tracks_indexed: u64::try_from(oversized.len()).expect("bounded fixture"),
                    errors_count: 0,
                },
                30,
            )
            .await
            .expect_err("oversized batch");
        assert_eq!(oversized_error.reason(), StorageReason::InvalidSetting);
        let traversal_error = repository
            .upsert_scan_batch(
                operation.operation_id(),
                job_id,
                &[track(
                    "../outside.mp3",
                    None,
                    ScanTrackAvailability::Available,
                )],
                ScanCounters {
                    files_seen: 1,
                    tracks_indexed: 1,
                    errors_count: 0,
                },
                40,
            )
            .await
            .expect_err("parent traversal");
        assert_eq!(traversal_error.reason(), StorageReason::PathOutsideScope);
        let track_count: i64 = sqlx::query_scalar("SELECT count(*) FROM tracks")
            .fetch_one(&repository.writer)
            .await
            .expect("track count");
        assert_eq!(track_count, 0);
        drop(repository);
        storage.close().await;
    }
}
