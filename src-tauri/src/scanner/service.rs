use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use tokio::{sync::Mutex, task::JoinError};
use uuid::Uuid;

use super::{
    CancelLibraryScanRequest, CancelLibraryScanResponse, CancelLibraryScanState, OperationAccepted,
    ScanEvent, ScanEventSink, ScanEventState, StartLibraryScanRequest,
    events::{ScanEventCounters, SharedScanEventSink},
    extractor::{ExtractError, ExtractedAvailability, ExtractedTrack, extract_track},
    traversal::{
        ScanCancellation, TRAVERSAL_CHANNEL_CAPACITY, TraversalError, TraversalItem,
        walk_authorized_root,
    },
};
use crate::{
    ipc::{
        ApiError, InternalReason, ProcessSequence, PublicField, RequestHash, canonical_request_hash,
    },
    storage::{
        AuthorizedScanOperation, AuthorizedScanRoot, Repository, ScanCancelOutcome, ScanCounters,
        ScanOperationState, ScanTerminalRecord, ScanTerminalState, ScanTrackAvailability,
        ScanTrackRecord, StorageError, StorageReason,
    },
};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(2);
const IDEMPOTENCY_WINDOW: Duration = Duration::from_mins(10);
const IDEMPOTENCY_CAPACITY: usize = 256;
const MAX_REQUESTED_ROOTS: usize = 256;
const MAX_ACTIVE_SCANS: usize = 4;
const SCAN_BATCH_SIZE: usize = 100;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const RECOVERY_BATCH: u32 = 100;
const MAX_RECOVERY_OPERATIONS: usize = 1_000;

pub trait ScanClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;

    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }

    fn now_rfc3339(&self) -> String {
        self.now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemScanClock;

impl ScanClock for SystemScanClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

struct IdempotencyEntry<T> {
    request_hash: RequestHash,
    expires_at: StdMutex<Option<Instant>>,
    result: Mutex<Option<Result<T, ApiError>>>,
}

impl<T> IdempotencyEntry<T> {
    fn retain_at(&self, now: Instant) -> bool {
        self.expires_at.lock().map_or(true, |expires_at| {
            expires_at.is_none_or(|value| value > now)
        })
    }

    fn mark_completed(&self) {
        if let Ok(mut expires_at) = self.expires_at.lock() {
            *expires_at = Some(Instant::now() + IDEMPOTENCY_WINDOW);
        }
    }
}

pub(super) struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<Uuid, Arc<IdempotencyEntry<T>>>>,
}

impl<T: Clone> AsyncIdempotency<T> {
    pub(super) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(IDEMPOTENCY_CAPACITY)),
        }
    }

    pub(super) async fn execute<F, Fut>(
        &self,
        request_id: Uuid,
        request_hash: RequestHash,
        operation: F,
    ) -> Result<T, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let now = Instant::now();
        let entry = {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, entry| entry.retain_at(now));
            if let Some(entry) = entries.get(&request_id) {
                if entry.request_hash != request_hash {
                    return Err(
                        ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                            .with_field(PublicField::ClientRequestId),
                    );
                }
                Arc::clone(entry)
            } else {
                if entries.len() >= IDEMPOTENCY_CAPACITY {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let entry = Arc::new(IdempotencyEntry {
                    request_hash,
                    expires_at: StdMutex::new(None),
                    result: Mutex::new(None),
                });
                entries.insert(request_id, Arc::clone(&entry));
                entry
            }
        };
        let mut stored = entry.result.lock().await;
        if let Some(result) = stored.as_ref() {
            return result.clone();
        }
        let result = operation().await;
        *stored = Some(result.clone());
        entry.mark_completed();
        result
    }
}

struct ActiveScan {
    cancellation: ScanCancellation,
    successful_jobs: Mutex<Vec<Uuid>>,
    terminal_transition: Mutex<()>,
}

impl ActiveScan {
    fn new() -> Self {
        Self {
            cancellation: ScanCancellation::default(),
            successful_jobs: Mutex::new(Vec::new()),
            terminal_transition: Mutex::new(()),
        }
    }
}

struct ScannerRuntime {
    repository: Repository,
    events: SharedScanEventSink,
    clock: Arc<dyn ScanClock>,
    sequence: Arc<ProcessSequence>,
    acceptance: Mutex<()>,
    active: StdMutex<HashMap<Uuid, Arc<ActiveScan>>>,
}

/// API-013/API-014 application service. Construction is side-effect free; all
/// filesystem work requires a repository-authorized root and explicit start.
pub struct ScannerService {
    runtime: Arc<ScannerRuntime>,
    start_requests: AsyncIdempotency<OperationAccepted>,
    cancel_requests: AsyncIdempotency<CancelLibraryScanResponse>,
    accept_timeout: Duration,
}

impl ScannerService {
    #[must_use]
    pub fn new(
        repository: Repository,
        events: Arc<dyn ScanEventSink>,
        clock: Arc<dyn ScanClock>,
        sequence: Arc<ProcessSequence>,
    ) -> Self {
        Self {
            runtime: Arc::new(ScannerRuntime {
                repository,
                events,
                clock,
                sequence,
                acceptance: Mutex::new(()),
                active: StdMutex::new(HashMap::new()),
            }),
            start_requests: AsyncIdempotency::new(),
            cancel_requests: AsyncIdempotency::new(),
            accept_timeout: ACCEPT_TIMEOUT,
        }
    }

    /// Validates, atomically accepts, and owns one background scan.
    ///
    /// # Errors
    ///
    /// Returns a redacted request, authorization, busy, path, or storage error.
    pub async fn start_scan(
        &self,
        request: StartLibraryScanRequest,
    ) -> Result<OperationAccepted, ApiError> {
        validate_start_request(&request)?;
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.start_requests
            .execute(request_id, request_hash, || async {
                self.accept_scan_once(request).await
            })
            .await
    }

    async fn accept_scan_once(
        &self,
        request: StartLibraryScanRequest,
    ) -> Result<OperationAccepted, ApiError> {
        let _acceptance = self.runtime.acceptance.lock().await;
        if self
            .runtime
            .active
            .lock()
            .map_err(|_| ApiError::unexpected())?
            .len()
            >= MAX_ACTIVE_SCANS
        {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        let operation_id = Uuid::now_v7();
        let accepted_at = self.runtime.clock.now_rfc3339();
        let accepted_at_ms = self.runtime.clock.now_ms();
        let operation = tokio::time::timeout(
            self.accept_timeout,
            self.runtime.repository.accept_scan_operation(
                operation_id,
                &request.root_ids,
                accepted_at_ms,
                env!("CARGO_PKG_VERSION"),
            ),
        )
        .await
        .map_err(|_| ApiError::from_reason(InternalReason::ResourceBusy))?
        .map_err(|error| map_storage_error(&error))?;
        let active = Arc::new(ActiveScan::new());
        {
            let mut active_scans = self
                .runtime
                .active
                .lock()
                .map_err(|_| ApiError::unexpected())?;
            if active_scans
                .insert(operation_id, Arc::clone(&active))
                .is_some()
            {
                return Err(ApiError::unexpected());
            }
        }
        let runtime = Arc::clone(&self.runtime);
        tokio::spawn(async move {
            run_scan(runtime, operation, active).await;
        });
        Ok(OperationAccepted {
            operation_id,
            accepted_at,
        })
    }

    /// Atomically transitions a live scan to cancelled, then publishes its
    /// persisted terminal. Signalling the worker happens before SQLite I/O.
    ///
    /// # Errors
    ///
    /// Returns a redacted not-found, idempotency, or storage error.
    pub async fn cancel_scan(
        &self,
        request: CancelLibraryScanRequest,
    ) -> Result<CancelLibraryScanResponse, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.cancel_requests
            .execute(request_id, request_hash, || async {
                self.cancel_scan_once(request).await
            })
            .await
    }

    async fn cancel_scan_once(
        &self,
        request: CancelLibraryScanRequest,
    ) -> Result<CancelLibraryScanResponse, ApiError> {
        let active = self
            .runtime
            .active
            .lock()
            .map_err(|_| ApiError::unexpected())?
            .get(&request.operation_id)
            .cloned();
        if let Some(active) = &active {
            active.cancellation.cancel();
        }
        let terminal_guard = if let Some(active) = &active {
            Some(active.terminal_transition.lock().await)
        } else {
            None
        };
        let successful_jobs = match &active {
            Some(active) => active.successful_jobs.lock().await.clone(),
            None => Vec::new(),
        };
        let outcome = self
            .runtime
            .repository
            .cancel_scan_operation(
                request.operation_id,
                &successful_jobs,
                self.runtime.clock.now_ms(),
            )
            .await
            .map_err(|error| map_storage_error(&error))?;
        let (state, terminal) = match outcome {
            ScanCancelOutcome::Cancelled(record) => (CancelLibraryScanState::Cancelled, record),
            ScanCancelOutcome::AlreadyTerminal(record) => {
                (CancelLibraryScanState::AlreadyTerminal, record)
            }
        };
        drop(terminal_guard);
        let delivery = deliver_terminal(&self.runtime, terminal).await;
        remove_active(&self.runtime, request.operation_id);
        delivery?;
        Ok(CancelLibraryScanResponse {
            request_id: request.client_request_id,
            operation_id: request.operation_id,
            state,
        })
    }

    /// Converts abandoned accepted/running scans to interrupted terminals and
    /// replays every bounded pending terminal before marking delivery.
    ///
    /// # Errors
    ///
    /// Returns a safe storage/transport error. Failed delivery remains pending.
    pub async fn recover_and_replay(&self) -> Result<(), ApiError> {
        let mut recovered = 0_usize;
        loop {
            let records = self
                .runtime
                .repository
                .recover_interrupted_scan_operations(RECOVERY_BATCH, self.runtime.clock.now_ms())
                .await
                .map_err(|error| map_storage_error(&error))?;
            recovered = recovered
                .checked_add(records.len())
                .ok_or_else(ApiError::unexpected)?;
            if recovered > MAX_RECOVERY_OPERATIONS {
                return Err(ApiError::from_reason(InternalReason::ResourceBusy));
            }
            if records.len() < RECOVERY_BATCH as usize {
                break;
            }
        }

        let mut delivered = 0_usize;
        loop {
            let records = self
                .runtime
                .repository
                .load_pending_scan_terminals(RECOVERY_BATCH)
                .await
                .map_err(|error| map_storage_error(&error))?;
            if records.is_empty() {
                return Ok(());
            }
            for record in records {
                deliver_terminal(&self.runtime, record).await?;
                delivered = delivered.checked_add(1).ok_or_else(ApiError::unexpected)?;
                if delivered > MAX_RECOVERY_OPERATIONS {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
            }
        }
    }
}

impl Drop for ScannerService {
    fn drop(&mut self) {
        if let Ok(active) = self.runtime.active.lock() {
            for scan in active.values() {
                scan.cancellation.cancel();
            }
        }
    }
}

async fn run_scan(
    runtime: Arc<ScannerRuntime>,
    operation: AuthorizedScanOperation,
    active: Arc<ActiveScan>,
) {
    let operation_id = operation.operation_id();
    let started = runtime
        .repository
        .mark_scan_operation_running(operation_id, runtime.clock.now_ms())
        .await;
    let Ok(progress) = started else {
        terminalize_failed(&runtime, &active, operation_id).await;
        remove_active(&runtime, operation_id);
        return;
    };
    let _ = publish_progress(&runtime, progress);

    let progress_runtime = Arc::clone(&runtime);
    let progress_active = Arc::clone(&active);
    let progress_task = tokio::spawn(async move {
        publish_periodic_progress(progress_runtime, operation_id, progress_active).await;
    });

    let mut scan_failed = false;
    for root in operation.roots() {
        if active.cancellation.is_cancelled() {
            break;
        }
        match scan_root(&runtime, &active, operation_id, root).await {
            Ok(()) => {
                let _terminal = active.terminal_transition.lock().await;
                if active.cancellation.is_cancelled() {
                    break;
                }
                active.successful_jobs.lock().await.push(root.job_id());
            }
            Err(ScanRootError::Cancelled) => break,
            Err(ScanRootError::Failed) => {
                scan_failed = true;
                break;
            }
        }
    }

    let _terminal = active.terminal_transition.lock().await;
    let successful_jobs = active.successful_jobs.lock().await.clone();
    let terminal = if active.cancellation.is_cancelled() {
        runtime
            .repository
            .cancel_scan_operation(operation_id, &successful_jobs, runtime.clock.now_ms())
            .await
            .map(|outcome| match outcome {
                ScanCancelOutcome::Cancelled(record)
                | ScanCancelOutcome::AlreadyTerminal(record) => record,
            })
    } else {
        let state = if scan_failed {
            ScanTerminalState::Failed
        } else {
            ScanTerminalState::Completed
        };
        runtime
            .repository
            .finish_scan_operation(
                operation_id,
                state,
                &successful_jobs,
                runtime.clock.now_ms(),
            )
            .await
    };
    if let Ok(record) = terminal {
        let _ = deliver_terminal(&runtime, record).await;
    }
    progress_task.abort();
    remove_active(&runtime, operation_id);
}

async fn terminalize_failed(runtime: &ScannerRuntime, active: &ActiveScan, operation_id: Uuid) {
    let _terminal = active.terminal_transition.lock().await;
    let successful_jobs = active.successful_jobs.lock().await.clone();
    if let Ok(record) = runtime
        .repository
        .finish_scan_operation(
            operation_id,
            ScanTerminalState::Failed,
            &successful_jobs,
            runtime.clock.now_ms(),
        )
        .await
    {
        let _ = deliver_terminal(runtime, record).await;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanRootError {
    Cancelled,
    Failed,
}

async fn scan_root(
    runtime: &ScannerRuntime,
    active: &ActiveScan,
    operation_id: Uuid,
    root: &AuthorizedScanRoot,
) -> Result<(), ScanRootError> {
    let authorized_root = root.canonical_path().to_path_buf();
    let traversal_root = authorized_root.clone();
    let traversal_cancellation = active.cancellation.clone();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(TRAVERSAL_CHANNEL_CAPACITY);
    let traversal = tokio::task::spawn_blocking(move || {
        walk_authorized_root(&traversal_root, &traversal_cancellation, &sender)
    });
    let mut batch = Vec::with_capacity(SCAN_BATCH_SIZE);
    let mut counters = ScanCounters::default();
    let mut since_flush = 0_usize;
    while let Some(item) = receiver.recv().await {
        if active.cancellation.is_cancelled() {
            return Err(ScanRootError::Cancelled);
        }
        counters.files_seen = counters.files_seen.saturating_add(1);
        since_flush = since_flush.saturating_add(1);
        match item {
            TraversalItem::Rejected => {
                counters.errors_count = counters.errors_count.saturating_add(1);
            }
            TraversalItem::File(file) => {
                let extraction_root = authorized_root.clone();
                let extraction_cancellation = active.cancellation.clone();
                match tokio::task::spawn_blocking(move || {
                    extract_track(&extraction_root, file, &extraction_cancellation)
                })
                .await
                .map_err(map_join_error)?
                {
                    Ok(track) => {
                        counters.tracks_indexed = counters.tracks_indexed.saturating_add(1);
                        if track.failed() {
                            counters.errors_count = counters.errors_count.saturating_add(1);
                        }
                        batch.push(map_track(track));
                    }
                    Err(ExtractError::Cancelled) => return Err(ScanRootError::Cancelled),
                    Err(ExtractError::PathDenied | ExtractError::MetadataUnavailable) => {
                        counters.errors_count = counters.errors_count.saturating_add(1);
                    }
                }
            }
        }
        if since_flush >= SCAN_BATCH_SIZE {
            flush_batch(runtime, operation_id, root.job_id(), &mut batch, counters).await?;
            since_flush = 0;
        }
    }
    match traversal.await.map_err(map_join_error)? {
        Ok(()) => {}
        Err(TraversalError::Cancelled) => return Err(ScanRootError::Cancelled),
        Err(
            TraversalError::RootUnavailable
            | TraversalError::WorkLimitExceeded
            | TraversalError::ConsumerStopped,
        ) => return Err(ScanRootError::Failed),
    }
    if active.cancellation.is_cancelled() {
        return Err(ScanRootError::Cancelled);
    }
    flush_batch(runtime, operation_id, root.job_id(), &mut batch, counters).await
}

async fn flush_batch(
    runtime: &ScannerRuntime,
    operation_id: Uuid,
    job_id: Uuid,
    batch: &mut Vec<ScanTrackRecord>,
    counters: ScanCounters,
) -> Result<(), ScanRootError> {
    let records = std::mem::take(batch);
    runtime
        .repository
        .upsert_scan_batch(
            operation_id,
            job_id,
            &records,
            counters,
            runtime.clock.now_ms(),
        )
        .await
        .map_err(|_| ScanRootError::Failed)?;
    Ok(())
}

fn map_track(track: ExtractedTrack) -> ScanTrackRecord {
    let availability = match track.availability {
        ExtractedAvailability::Available => ScanTrackAvailability::Available,
        ExtractedAvailability::Corrupt => ScanTrackAvailability::Corrupt,
        ExtractedAvailability::Unsupported => ScanTrackAvailability::Unsupported,
    };
    let metadata_confidence = if availability == ScanTrackAvailability::Available {
        1.0
    } else {
        0.0
    };
    ScanTrackRecord {
        relative_path: PathBuf::from(track.relative_path),
        file_identity: track.file_identity,
        availability,
        format: track.format,
        file_size_bytes: track.file_size_bytes,
        modified_at_ms: track.modified_at_ms,
        duration_ms: track.duration_ms,
        title: track.title,
        artist: track.artist,
        album: track.album,
        album_artist: track.album_artist,
        track_number: track.track_number,
        disc_number: track.disc_number,
        year: track.year.map(i32::from),
        genres: track.genres,
        metadata_confidence,
        embedded_cover_hash: track.embedded_cover_hash,
    }
}

async fn publish_periodic_progress(
    runtime: Arc<ScannerRuntime>,
    operation_id: Uuid,
    active: Arc<ActiveScan>,
) {
    let mut interval = tokio::time::interval(PROGRESS_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if active.cancellation.is_cancelled() {
            return;
        }
        let Ok(progress) = runtime.repository.load_scan_progress(operation_id).await else {
            return;
        };
        if progress.state != ScanOperationState::Running {
            return;
        }
        let _ = publish_progress(&runtime, progress);
    }
}

fn publish_progress(
    runtime: &ScannerRuntime,
    progress: crate::storage::ScanProgress,
) -> Result<(), ApiError> {
    let event = ScanEvent::new(
        &runtime.sequence,
        runtime.clock.now_rfc3339(),
        progress.operation_id,
        ScanEventState::Running,
        map_counters(progress.counters),
        None,
    )?;
    runtime.events.publish(event)
}

async fn deliver_terminal(
    runtime: &ScannerRuntime,
    terminal: ScanTerminalRecord,
) -> Result<(), ApiError> {
    let occurred_at = Utc
        .timestamp_millis_opt(terminal.occurred_at_ms)
        .single()
        .ok_or_else(ApiError::unexpected)?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let (state, safe_message) = match terminal.state {
        ScanTerminalState::Completed => (ScanEventState::Completed, None),
        ScanTerminalState::Cancelled => (ScanEventState::Cancelled, None),
        ScanTerminalState::Failed => (
            ScanEventState::Failed,
            Some("扫描未能完成，已保留成功提交的曲目。".to_owned()),
        ),
        ScanTerminalState::Interrupted => (
            ScanEventState::Failed,
            Some("扫描因应用重启而中断，请重新扫描。".to_owned()),
        ),
    };
    let event = ScanEvent::new(
        &runtime.sequence,
        occurred_at,
        terminal.operation_id,
        state,
        map_counters(terminal.counters),
        safe_message,
    )?;
    runtime.events.publish(event)?;
    runtime
        .repository
        .mark_scan_terminal_delivered(
            terminal.outbox_id,
            terminal.operation_id,
            runtime.clock.now_ms(),
        )
        .await
        .map_err(|error| map_storage_error(&error))?;
    Ok(())
}

const fn map_counters(counters: ScanCounters) -> ScanEventCounters {
    ScanEventCounters {
        scanned: counters.files_seen,
        discovered: counters.tracks_indexed,
        failed: counters.errors_count,
    }
}

pub(super) fn validate_start_request(request: &StartLibraryScanRequest) -> Result<(), ApiError> {
    if request.root_ids.len() > MAX_REQUESTED_ROOTS {
        return Err(
            ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
        );
    }
    let mut unique = HashSet::with_capacity(request.root_ids.len());
    if request
        .root_ids
        .iter()
        .any(|root_id| root_id.get_version_num() != 7 || !unique.insert(*root_id))
    {
        return Err(
            ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
        );
    }
    Ok(())
}

fn map_join_error(_error: JoinError) -> ScanRootError {
    ScanRootError::Failed
}

fn remove_active(runtime: &ScannerRuntime, operation_id: Uuid) {
    if let Ok(mut active) = runtime.active.lock() {
        active.remove(&operation_id);
    }
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
    };
    ApiError::from_reason(reason)
}
