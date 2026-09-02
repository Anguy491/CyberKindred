use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use super::error::{ApiError, InternalReason, PublicField};

/// Fixed retention windows from API Contract v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdempotencyWindow {
    Standard,
    MediaControl,
}

impl IdempotencyWindow {
    const fn duration(self) -> Duration {
        match self {
            Self::Standard => Duration::from_mins(10),
            Self::MediaControl => Duration::from_secs(30),
        }
    }
}

/// Payload fingerprint retained by the in-memory idempotency store.
///
/// It intentionally implements neither `Debug` nor serialization, preventing a
/// request body or secret from being exposed through diagnostics or IPC.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RequestHash([u64; 4]);

/// Hashes a recursively canonicalized JSON request without retaining its body.
///
/// # Errors
///
/// Returns `ERR-1001` when the request cannot be represented as JSON.
pub fn canonical_request_hash<T: Serialize>(request: &T) -> Result<RequestHash, ApiError> {
    let value = serde_json::to_value(request).map_err(|_| {
        ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
    })?;
    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| {
        ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
    })?;
    Ok(RequestHash(hash_bytes(&bytes)))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = serde_json::Map::new();
            for (key, item) in entries {
                canonical.insert(key, canonicalize(item));
            }
            Value::Object(canonical)
        }
        scalar => scalar,
    }
}

fn hash_bytes(bytes: &[u8]) -> [u64; 4] {
    const OFFSETS: [u64; 4] = [
        0xcbf2_9ce4_8422_2325,
        0x8422_2325_cbf2_9ce4,
        0x9e37_79b9_7f4a_7c15,
        0xd6e8_feb8_6659_fd93,
    ];
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut state = OFFSETS;
    let lanes = [(1_u32, 5_u32, 0_u64), (2, 12, 1), (3, 19, 2), (4, 26, 3)];
    for (index, byte) in bytes.iter().copied().enumerate() {
        for (value, (input_rotation, output_rotation, lane)) in state.iter_mut().zip(lanes) {
            let mixed = u64::from(byte) ^ ((index as u64).rotate_left(input_rotation));
            *value ^= mixed.wrapping_add(lane << 8);
            *value = value.wrapping_mul(PRIME).rotate_left(output_rotation);
        }
    }
    for (value, rotation) in state.iter_mut().zip([0_u32, 11, 22, 33]) {
        *value ^= (bytes.len() as u64).rotate_left(rotation);
    }
    state
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct IdempotencyKey {
    command: &'static str,
    client_request_id: Uuid,
}

#[derive(Clone)]
struct StoredResult<T> {
    request_hash: RequestHash,
    result: Result<T, ApiError>,
    expires_at: Instant,
}

/// Bounded in-memory result cache for a single response type.
pub struct IdempotencyStore<T> {
    entries: HashMap<IdempotencyKey, StoredResult<T>>,
    max_entries: usize,
}

impl<T: Clone> IdempotencyStore<T> {
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(max_entries.max(1)),
            max_entries: max_entries.max(1),
        }
    }

    /// Returns the original result for a retry or executes once for a new key.
    ///
    /// Identical IDs with a different canonical payload fail with `ERR-1003`.
    /// Neither request payloads nor secrets are retained.
    ///
    /// # Errors
    ///
    /// Returns the cached/executed operation error, a payload validation error,
    /// or the stable idempotency conflict.
    pub fn execute<R: Serialize, F: FnOnce() -> Result<T, ApiError>>(
        &mut self,
        now: Instant,
        command: &'static str,
        client_request_id: Uuid,
        request: &R,
        window: IdempotencyWindow,
        operation: F,
    ) -> Result<T, ApiError> {
        self.entries.retain(|_, stored| stored.expires_at > now);
        let key = IdempotencyKey {
            command,
            client_request_id,
        };
        let request_hash = canonical_request_hash(request)?;
        if let Some(stored) = self.entries.get(&key) {
            if stored.request_hash == request_hash {
                return stored.result.clone();
            }
            return Err(
                ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                    .with_field(PublicField::ClientRequestId),
            );
        }

        if self.entries.len() >= self.max_entries
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, stored)| stored.expires_at)
                .map(|(entry_key, _)| *entry_key)
        {
            self.entries.remove(&oldest);
        }

        let result = operation();
        self.entries.insert(
            key,
            StoredResult {
                request_hash,
                result: result.clone(),
                expires_at: now + window.duration(),
            },
        );
        result
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
