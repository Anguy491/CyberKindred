use super::{
    AudioFormat, MAX_SPEECH_BYTES, SPEECH_CACHE_TTL_MS, SpeechArtifact, SpeechCacheKey,
    SpeechInput, invalid_response, valid_operation_id,
};
use crate::providers::{ProviderFailure, ProviderFailureCategory};
use crate::speech::provider::validate_mp3;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum SpeechOwner {
    Preview(Uuid),
    Segment(Uuid),
}

impl SpeechOwner {
    pub(crate) fn is_valid(self) -> bool {
        match self {
            Self::Preview(id) | Self::Segment(id) => valid_operation_id(id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechCacheMetadata {
    pub content_hash: String,
    pub format: AudioFormat,
    pub byte_length: u64,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub owners: Vec<SpeechOwner>,
    pub active_lease_count: usize,
}

struct CacheEntry {
    artifact: SpeechArtifact,
    bytes: Arc<[u8]>,
    created_at_ms: i64,
    expires_at_ms: i64,
    owners: HashSet<SpeechOwner>,
    leases: HashSet<Uuid>,
    last_access: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<SpeechCacheKey, CacheEntry>,
    access_sequence: u64,
    total_bytes: usize,
}

struct SpeechCacheInner {
    max_entries: usize,
    max_bytes: usize,
    state: Mutex<CacheState>,
}

#[derive(Clone)]
pub struct SpeechCache(Arc<SpeechCacheInner>);

impl SpeechCache {
    #[must_use]
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self(Arc::new(SpeechCacheInner {
            max_entries: max_entries.clamp(1, 16_384),
            max_bytes: max_bytes.clamp(1, 512 * 1_024 * 1_024),
            state: Mutex::new(CacheState::default()),
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        match self.0.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Atomically promotes a validated synthesis result into the bounded cache.
    ///
    /// # Errors
    ///
    /// Rejects mismatched hashes/lengths, invalid ownership, oversized bytes,
    /// or a cache whose active leases prevent bounded insertion.
    pub fn insert(
        &self,
        input: &SpeechInput,
        artifact: SpeechArtifact,
        bytes: Vec<u8>,
        owner: SpeechOwner,
        now_ms: i64,
    ) -> Result<(), ProviderFailure> {
        let key = input.cache_key();
        if !owner.is_valid()
            || bytes.is_empty()
            || bytes.len() > MAX_SPEECH_BYTES
            || bytes.len() > self.0.max_bytes
            || artifact.content_hash != key.to_hex()
            || artifact.format != input.format()
            || artifact.byte_length != u64::try_from(bytes.len()).map_err(|_| invalid_response())?
        {
            return Err(invalid_response());
        }
        validate_mp3(&bytes, "audio/mpeg")?;
        let mut state = self.lock();
        prune_expired(&mut state, now_ms);
        state.access_sequence = state.access_sequence.saturating_add(1);
        let sequence = state.access_sequence;
        if let Some(existing) = state.entries.get_mut(&key) {
            if existing.bytes.as_ref() != bytes.as_slice() {
                return Err(invalid_response());
            }
            existing.owners.insert(owner);
            existing.last_access = sequence;
            return Ok(());
        }
        let byte_length = bytes.len();
        let mut owners = HashSet::new();
        owners.insert(owner);
        state.total_bytes = state.total_bytes.saturating_add(byte_length);
        state.entries.insert(
            key,
            CacheEntry {
                artifact,
                bytes: Arc::from(bytes),
                created_at_ms: now_ms,
                expires_at_ms: now_ms.saturating_add(SPEECH_CACHE_TTL_MS),
                owners,
                leases: HashSet::new(),
                last_access: sequence,
            },
        );
        while state.entries.len() > self.0.max_entries || state.total_bytes > self.0.max_bytes {
            let oldest = state
                .entries
                .iter()
                .filter(|(_, entry)| entry.leases.is_empty())
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(entry_key, _)| *entry_key);
            let Some(oldest) = oldest else {
                remove_entry(&mut state, key);
                return Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
            };
            remove_entry(&mut state, oldest);
        }
        if state.entries.contains_key(&key) {
            Ok(())
        } else {
            Err(ProviderFailure::new(ProviderFailureCategory::Unavailable))
        }
    }

    #[must_use]
    pub fn acquire(
        &self,
        key: SpeechCacheKey,
        owner: SpeechOwner,
        now_ms: i64,
    ) -> Option<SpeechLease> {
        if !owner.is_valid() {
            return None;
        }
        let mut state = self.lock();
        prune_expired(&mut state, now_ms);
        state.access_sequence = state.access_sequence.saturating_add(1);
        let sequence = state.access_sequence;
        let entry = state.entries.get_mut(&key)?;
        let lease_id = Uuid::now_v7();
        entry.owners.insert(owner);
        entry.leases.insert(lease_id);
        entry.last_access = sequence;
        Some(SpeechLease {
            inner: Arc::clone(&self.0),
            key,
            lease_id,
            artifact: entry.artifact.clone(),
            bytes: Arc::clone(&entry.bytes),
        })
    }

    pub fn remove_owner(&self, key: SpeechCacheKey, owner: SpeechOwner) {
        if let Some(entry) = self.lock().entries.get_mut(&key) {
            entry.owners.remove(&owner);
        }
    }

    pub fn prune(&self, now_ms: i64) {
        prune_expired(&mut self.lock(), now_ms);
    }

    #[must_use]
    pub fn snapshot_metadata(&self) -> Vec<SpeechCacheMetadata> {
        let state = self.lock();
        let mut metadata = state
            .entries
            .values()
            .map(|entry| {
                let mut owners = entry.owners.iter().copied().collect::<Vec<_>>();
                owners.sort_by_key(|owner| match owner {
                    SpeechOwner::Preview(id) => (0, id.as_u128()),
                    SpeechOwner::Segment(id) => (1, id.as_u128()),
                });
                SpeechCacheMetadata {
                    content_hash: entry.artifact.content_hash.clone(),
                    format: entry.artifact.format,
                    byte_length: entry.artifact.byte_length,
                    created_at_ms: entry.created_at_ms,
                    expires_at_ms: entry.expires_at_ms,
                    owners,
                    active_lease_count: entry.leases.len(),
                }
            })
            .collect::<Vec<_>>();
        metadata.sort_by(|left, right| left.content_hash.cmp(&right.content_hash));
        metadata
    }
}

fn prune_expired(state: &mut CacheState, now_ms: i64) {
    let expired = state
        .entries
        .iter()
        .filter(|(_, entry)| entry.expires_at_ms <= now_ms && entry.leases.is_empty())
        .map(|(key, _)| *key)
        .collect::<Vec<_>>();
    for key in expired {
        remove_entry(state, key);
    }
}

fn remove_entry(state: &mut CacheState, key: SpeechCacheKey) {
    if let Some(removed) = state.entries.remove(&key) {
        state.total_bytes = state.total_bytes.saturating_sub(removed.bytes.len());
    }
}

/// Active lease pins bytes against expiry/LRU until playback releases it.
pub struct SpeechLease {
    inner: Arc<SpeechCacheInner>,
    key: SpeechCacheKey,
    lease_id: Uuid,
    artifact: SpeechArtifact,
    bytes: Arc<[u8]>,
}

impl SpeechLease {
    #[must_use]
    pub const fn artifact(&self) -> &SpeechArtifact {
        &self.artifact
    }

    #[must_use]
    pub fn bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }
}

impl Drop for SpeechLease {
    fn drop(&mut self) {
        let mut state = match self.inner.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(entry) = state.entries.get_mut(&self.key) {
            entry.leases.remove(&self.lease_id);
        }
    }
}
