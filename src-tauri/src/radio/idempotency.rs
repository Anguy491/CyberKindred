use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::ipc::{ApiError, InternalReason, PublicField, RequestHash};

const IDEMPOTENCY_WINDOW: Duration = Duration::from_mins(10);

struct Entry<T> {
    request_hash: RequestHash,
    expires_at: StdMutex<Option<Instant>>,
    result: Mutex<Option<Result<T, ApiError>>>,
}

impl<T> Entry<T> {
    fn retain_at(&self, now: Instant) -> bool {
        self.expires_at
            .lock()
            .map_or(true, |expires| expires.is_none_or(|value| value > now))
    }

    fn mark_completed(&self) {
        if let Ok(mut expires) = self.expires_at.lock() {
            *expires = Some(Instant::now() + IDEMPOTENCY_WINDOW);
        }
    }
}

/// Completion-based, bounded single-flight store. A live entry is never
/// evicted, and unrelated request IDs do not share an operation lock.
pub(super) struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<Uuid, Arc<Entry<T>>>>,
    capacity: usize,
}

impl<T: Clone> AsyncIdempotency<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(capacity.max(1))),
            capacity: capacity.max(1),
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
                if entries.len() >= self.capacity {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let entry = Arc::new(Entry {
                    request_hash,
                    expires_at: StdMutex::new(None),
                    result: Mutex::new(None),
                });
                entries.insert(request_id, Arc::clone(&entry));
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
