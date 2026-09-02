use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::ipc::{ApiError, InternalReason, PublicField, RequestHash};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct IdempotencyKey {
    command: &'static str,
    request_id: Uuid,
}

struct Stored<T> {
    request_hash: RequestHash,
    operation: Mutex<OperationState<T>>,
}

struct OperationState<T> {
    result: Option<Result<T, ApiError>>,
    expires_at: Option<Instant>,
}

pub(super) struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<IdempotencyKey, Arc<Stored<T>>>>,
    capacity: usize,
}

impl<T: Clone> AsyncIdempotency<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(capacity)),
            capacity,
        }
    }

    pub(super) async fn execute<F, Fut>(
        &self,
        command: &'static str,
        request_id: Uuid,
        request_hash: RequestHash,
        retention: Duration,
        operation: F,
    ) -> Result<T, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let key = IdempotencyKey {
            command,
            request_id,
        };
        let stored = {
            let now = Instant::now();
            let mut entries = self.entries.lock().await;
            entries.retain(|_, entry| {
                entry.operation.try_lock().map_or(true, |state| {
                    state.expires_at.is_none_or(|expiry| expiry > now)
                })
            });
            if let Some(existing) = entries.get(&key) {
                if existing.request_hash != request_hash {
                    return Err(
                        ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                            .with_field(PublicField::ClientRequestId),
                    );
                }
                Arc::clone(existing)
            } else {
                if entries.len() >= self.capacity {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let entry = Arc::new(Stored {
                    request_hash,
                    operation: Mutex::new(OperationState {
                        result: None,
                        expires_at: None,
                    }),
                });
                entries.insert(key, Arc::clone(&entry));
                entry
            }
        };

        let mut state = stored.operation.lock().await;
        if let Some(result) = &state.result {
            return result.clone();
        }
        let result = operation().await;
        state.result = Some(result.clone());
        state.expires_at = Some(Instant::now() + retention);
        result
    }
}
