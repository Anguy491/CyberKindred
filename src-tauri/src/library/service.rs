use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    LibraryRoot, LibraryRootAck, LibraryRootPicker, LibraryRootsResponse,
    PickAndAddLibraryRootRequest, PickAndAddLibraryRootResponse, RemoveLibraryRootRequest,
};
use crate::{
    ipc::{ApiError, InternalReason, PublicField, RequestHash, canonical_request_hash},
    storage::{Repository, StorageError, StorageReason},
};

const IDEMPOTENCY_CAPACITY: usize = 256;
const IDEMPOTENCY_WINDOW: Duration = Duration::from_mins(10);
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub trait LibraryRootClock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemLibraryRootClock;

impl LibraryRootClock for SystemLibraryRootClock {
    fn now_ms(&self) -> i64 {
        Utc::now().timestamp_millis()
    }
}

struct IdempotencyEntry<T> {
    request_hash: RequestHash,
    expires_at: std::sync::Mutex<Option<Instant>>,
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

struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<Uuid, Arc<IdempotencyEntry<T>>>>,
    capacity: usize,
}

impl<T: Clone> AsyncIdempotency<T> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(capacity.max(1))),
            capacity: capacity.max(1),
        }
    }

    async fn execute<F, Fut>(
        &self,
        client_request_id: Uuid,
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
            if let Some(entry) = entries.get(&client_request_id) {
                if entry.request_hash != request_hash {
                    return Err(
                        ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                            .with_field(PublicField::ClientRequestId),
                    );
                }
                Arc::clone(entry)
            } else {
                if entries.len() >= self.capacity {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let entry = Arc::new(IdempotencyEntry {
                    request_hash,
                    expires_at: std::sync::Mutex::new(None),
                    result: Mutex::new(None),
                });
                entries.insert(client_request_id, Arc::clone(&entry));
                entry
            }
        };

        let mut result = entry.result.lock().await;
        if let Some(result) = result.as_ref() {
            return result.clone();
        }
        let executed = operation().await;
        *result = Some(executed.clone());
        entry.mark_completed();
        executed
    }
}

/// API-010–012 service. Absolute paths enter only through its injected picker
/// and never appear in public DTOs or errors.
pub struct LibraryRootService {
    repository: Repository,
    picker: Arc<dyn LibraryRootPicker>,
    clock: Arc<dyn LibraryRootClock>,
    mutations: Mutex<()>,
    add_requests: AsyncIdempotency<PickAndAddLibraryRootResponse>,
    remove_requests: AsyncIdempotency<LibraryRootAck>,
}

impl LibraryRootService {
    #[must_use]
    pub fn new(
        repository: Repository,
        picker: Arc<dyn LibraryRootPicker>,
        clock: Arc<dyn LibraryRootClock>,
    ) -> Self {
        Self::with_capacity(repository, picker, clock, IDEMPOTENCY_CAPACITY)
    }

    pub(super) fn with_capacity(
        repository: Repository,
        picker: Arc<dyn LibraryRootPicker>,
        clock: Arc<dyn LibraryRootClock>,
        capacity: usize,
    ) -> Self {
        Self {
            repository,
            picker,
            clock,
            mutations: Mutex::new(()),
            add_requests: AsyncIdempotency::new(capacity),
            remove_requests: AsyncIdempotency::new(capacity),
        }
    }

    /// Returns the current path-free root collection.
    ///
    /// # Errors
    ///
    /// Returns a stable, redacted storage error.
    pub async fn list_library_roots(&self) -> Result<LibraryRootsResponse, ApiError> {
        let stored: crate::storage::StoredLibraryRoots = self
            .repository
            .load_library_roots()
            .await
            .map_err(|error| map_storage_error(&error))?;
        Ok(LibraryRootsResponse {
            roots: stored.roots.into_iter().map(map_root).collect(),
            revision: stored.revision,
        })
    }

    /// Opens the injected native picker and adds its selected directory.
    ///
    /// # Errors
    ///
    /// Returns stable validation, path, capacity, or storage errors only.
    pub async fn pick_and_add_library_root(
        &self,
        request: PickAndAddLibraryRootRequest,
    ) -> Result<PickAndAddLibraryRootResponse, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.add_requests
            .execute(request_id, request_hash, || async {
                let selected = self
                    .picker
                    .pick_directory()
                    .await
                    .map_err(|_| ApiError::from_reason(InternalReason::PathDenied))?;
                let Some(selected) = selected else {
                    let current = self.list_library_roots().await?;
                    return Ok(PickAndAddLibraryRootResponse {
                        request_id,
                        root: None,
                        revision: current.revision,
                    });
                };

                let _mutation = self.mutations.lock().await;
                let stored: crate::storage::StoredLibraryRootAddition = self
                    .repository
                    .add_library_root(&selected, self.clock.now_ms())
                    .await
                    .map_err(|error| map_storage_error(&error))?;
                Ok(PickAndAddLibraryRootResponse {
                    request_id,
                    root: Some(map_root(stored.root)),
                    revision: stored.revision,
                })
            })
            .await
    }

    /// Soft-disables one root under optimistic concurrency.
    ///
    /// # Errors
    ///
    /// Returns stable validation, conflict, not-found, capacity, or storage errors.
    pub async fn remove_library_root(
        &self,
        request: RemoveLibraryRootRequest,
    ) -> Result<LibraryRootAck, ApiError> {
        if request.expected_revision > MAX_JS_SAFE_INTEGER {
            return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                .with_field(PublicField::ExpectedRevision));
        }
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.remove_requests
            .execute(request_id, request_hash, || async {
                let _mutation = self.mutations.lock().await;
                let current = self.list_library_roots().await?;
                if request.expected_revision != current.revision {
                    return Err(ApiError::from_reason(InternalReason::RevisionConflict)
                        .with_field(PublicField::ExpectedRevision)
                        .with_current_revision(current.revision));
                }
                let revision = self
                    .repository
                    .remove_library_root(
                        request.root_id,
                        request.expected_revision,
                        self.clock.now_ms(),
                    )
                    .await
                    .map_err(|error| map_storage_error(&error))?;
                Ok(LibraryRootAck {
                    request_id,
                    revision,
                })
            })
            .await
    }
}

fn map_root(root: crate::storage::StoredLibraryRoot) -> LibraryRoot {
    LibraryRoot {
        root_id: root.root_id,
        display_name: root.display_name,
        available: root.available,
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
