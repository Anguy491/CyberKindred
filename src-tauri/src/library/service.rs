use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    LibraryRoot, LibraryRootAck, LibraryRootPicker, LibraryRootsResponse, ListTracksRequest,
    MAX_TRACK_PAGE_SIZE, PickAndAddLibraryRootRequest, PickAndAddLibraryRootResponse,
    RemoveLibraryRootRequest, TrackAvailabilityFilter, TrackMatchStatus, TrackSort, TrackView,
    TracksPage,
};
use crate::{
    ipc::{ApiError, InternalReason, PublicField, RequestHash, canonical_request_hash},
    storage::{
        Repository, StorageError, StorageReason,
        track_catalog::{
            StoredTrackAvailability, StoredTrackAvailabilityFilter, StoredTrackCatalogItem,
            StoredTrackCatalogPage, StoredTrackCatalogQuery, StoredTrackMatchFilter,
            StoredTrackMatchStatus, StoredTrackSort,
        },
    },
};

const IDEMPOTENCY_CAPACITY: usize = 256;
const IDEMPOTENCY_WINDOW: Duration = Duration::from_mins(10);
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_TRACK_CURSOR_CHARS: usize = 512;
const MAX_TRACK_QUERY_CHARS: usize = 200;
const MAX_TRACK_TAG_CHARS: usize = 1_000;

pub trait LibraryRootClock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemLibraryRootClock;

impl LibraryRootClock for SystemLibraryRootClock {
    fn now_ms(&self) -> i64 {
        Utc::now().timestamp_millis()
    }
}

/// Validated API-015 query passed to the SQLite-owned catalog adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackCatalogQuery {
    pub cursor: Option<String>,
    pub limit: u32,
    pub query: Option<String>,
    pub sort: TrackSort,
    pub availability: Option<TrackAvailabilityFilter>,
    pub match_status: Option<TrackMatchStatus>,
}

pub type TrackCatalogFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TracksPage, StorageError>> + Send + 'a>>;

/// Storage seam for API-015. Implementations use an opaque bounded cursor and
/// must never return a path-bearing record.
pub trait TrackCatalog: Send + Sync {
    fn list_tracks(&self, query: TrackCatalogQuery) -> TrackCatalogFuture<'_>;
}

impl TrackCatalog for Repository {
    fn list_tracks(&self, query: TrackCatalogQuery) -> TrackCatalogFuture<'_> {
        Box::pin(async move {
            let stored = self.list_track_catalog(map_catalog_query(query)?).await?;
            map_catalog_page(stored)
        })
    }
}

/// Read-only API-015 service with strict input and adapter-output validation.
pub struct TrackCatalogService {
    catalog: Arc<dyn TrackCatalog>,
}

impl TrackCatalogService {
    #[must_use]
    pub fn new(catalog: Arc<dyn TrackCatalog>) -> Self {
        Self { catalog }
    }

    /// Returns one validated, bounded, path-free catalog page.
    ///
    /// # Errors
    ///
    /// Returns a stable request or storage error without query/tag/path text.
    pub async fn list_tracks(&self, request: ListTracksRequest) -> Result<TracksPage, ApiError> {
        let query = validate_track_query(request)?;
        let limit = query.limit;
        let page = self
            .catalog
            .list_tracks(query)
            .await
            .map_err(|error| map_storage_error(&error))?;
        validate_track_page(&page, limit)?;
        Ok(page)
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

fn validate_track_query(request: ListTracksRequest) -> Result<TrackCatalogQuery, ApiError> {
    if !(1..=MAX_TRACK_PAGE_SIZE).contains(&request.limit)
        || request
            .cursor
            .as_deref()
            .is_some_and(|cursor| !valid_cursor(cursor))
    {
        return Err(invalid_track_request());
    }
    let query = match request.query {
        Some(query) => {
            let trimmed = query.trim();
            if trimmed.chars().count() > MAX_TRACK_QUERY_CHARS
                || trimmed.chars().any(char::is_control)
            {
                return Err(invalid_track_request());
            }
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        None => None,
    };
    Ok(TrackCatalogQuery {
        cursor: request.cursor,
        limit: request.limit,
        query,
        sort: request.sort,
        availability: request.filters.availability,
        match_status: request.filters.match_status,
    })
}

fn map_catalog_query(query: TrackCatalogQuery) -> Result<StoredTrackCatalogQuery, StorageError> {
    Ok(StoredTrackCatalogQuery {
        offset: query
            .cursor
            .as_deref()
            .map(decode_track_cursor)
            .transpose()?
            .unwrap_or(0),
        limit: query.limit,
        query: query.query,
        sort: match query.sort {
            TrackSort::Title => StoredTrackSort::Title,
            TrackSort::Artist => StoredTrackSort::Artist,
            TrackSort::Album => StoredTrackSort::Album,
            TrackSort::Recent => StoredTrackSort::Recent,
        },
        availability: query.availability.map(|availability| match availability {
            TrackAvailabilityFilter::Playable => StoredTrackAvailabilityFilter::Playable,
            TrackAvailabilityFilter::Missing => StoredTrackAvailabilityFilter::Missing,
        }),
        match_status: query.match_status.map(|status| match status {
            TrackMatchStatus::Matched => StoredTrackMatchFilter::Matched,
            TrackMatchStatus::Unmatched => StoredTrackMatchFilter::Unmatched,
            TrackMatchStatus::Review => StoredTrackMatchFilter::Review,
        }),
    })
}

fn map_catalog_page(page: StoredTrackCatalogPage) -> Result<TracksPage, StorageError> {
    Ok(TracksPage {
        items: page.items.into_iter().map(map_catalog_item).collect(),
        next_cursor: page.next_offset.map(encode_track_cursor).transpose()?,
    })
}

fn map_catalog_item(item: StoredTrackCatalogItem) -> TrackView {
    TrackView {
        track_id: item.track_id,
        availability: match item.availability {
            StoredTrackAvailability::Playable => super::TrackAvailability::Playable,
            StoredTrackAvailability::Missing => super::TrackAvailability::Missing,
            StoredTrackAvailability::Corrupt => super::TrackAvailability::Corrupt,
            StoredTrackAvailability::Unsupported => super::TrackAvailability::Unsupported,
        },
        duration_ms: item.duration_ms,
        artwork_available: item.artwork_available,
        original: super::TrackTagView {
            title: item.original.title,
            artist: item.original.artist,
            album: item.original.album,
        },
        enriched: item.enriched.map(|enriched| super::EnrichedTrackTagView {
            title: enriched.tags.title,
            artist: enriched.tags.artist,
            album: enriched.tags.album,
            provider: super::TrackMetadataProvider::Musicbrainz,
            confidence: enriched.confidence,
            fetched_at: enriched.fetched_at,
        }),
        match_status: match item.match_status {
            StoredTrackMatchStatus::Matched => TrackMatchStatus::Matched,
            StoredTrackMatchStatus::Unmatched => TrackMatchStatus::Unmatched,
            StoredTrackMatchStatus::Review => TrackMatchStatus::Review,
        },
    }
}

fn decode_track_cursor(cursor: &str) -> Result<u64, StorageError> {
    let Some(value) = cursor.strip_prefix("v1:") else {
        return Err(StorageError::new(StorageReason::InvalidSetting));
    };
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err(StorageError::new(StorageReason::InvalidSetting));
    }
    let offset = value
        .parse::<u64>()
        .map_err(|_| StorageError::new(StorageReason::InvalidSetting))?;
    if offset > MAX_JS_SAFE_INTEGER {
        return Err(StorageError::new(StorageReason::InvalidSetting));
    }
    Ok(offset)
}

fn encode_track_cursor(offset: u64) -> Result<String, StorageError> {
    if offset == 0 || offset > MAX_JS_SAFE_INTEGER {
        return Err(StorageError::new(StorageReason::StorageIntegrityFailed));
    }
    Ok(format!("v1:{offset}"))
}

fn validate_track_page(page: &TracksPage, limit: u32) -> Result<(), ApiError> {
    let limit = usize::try_from(limit).map_err(|_| invalid_catalog_output())?;
    if page.items.len() > limit
        || page
            .next_cursor
            .as_deref()
            .is_some_and(|cursor| !valid_cursor(cursor))
    {
        return Err(invalid_catalog_output());
    }
    let mut track_ids = HashSet::with_capacity(page.items.len());
    for track in &page.items {
        if track.track_id.get_version_num() != 7
            || !track_ids.insert(track.track_id)
            || track.duration_ms > MAX_JS_SAFE_INTEGER
            || !valid_tag_view(&track.original)
            || !valid_enrichment(track)
        {
            return Err(invalid_catalog_output());
        }
    }
    Ok(())
}

fn valid_cursor(cursor: &str) -> bool {
    if cursor.chars().count() > MAX_TRACK_CURSOR_CHARS || cursor.chars().any(char::is_control) {
        return false;
    }
    let Some(value) = cursor.strip_prefix("v1:") else {
        return false;
    };
    !(value.is_empty() || value.len() > 1 && value.starts_with('0'))
        && value
            .parse::<u64>()
            .is_ok_and(|offset| offset <= MAX_JS_SAFE_INTEGER)
}

fn valid_tag_view(tags: &super::TrackTagView) -> bool {
    [
        tags.title.as_deref(),
        tags.artist.as_deref(),
        tags.album.as_deref(),
    ]
    .into_iter()
    .flatten()
    .all(valid_track_tag)
}

fn valid_enrichment(track: &TrackView) -> bool {
    let enrichment_valid = track.enriched.as_ref().is_none_or(|enriched| {
        [
            enriched.title.as_deref(),
            enriched.artist.as_deref(),
            enriched.album.as_deref(),
        ]
        .into_iter()
        .flatten()
        .all(valid_track_tag)
            && enriched.confidence.is_finite()
            && (0.0..=1.0).contains(&enriched.confidence)
            && chrono::DateTime::parse_from_rfc3339(&enriched.fetched_at).is_ok()
    });
    enrichment_valid
        && match track.match_status {
            TrackMatchStatus::Matched | TrackMatchStatus::Review => track.enriched.is_some(),
            TrackMatchStatus::Unmatched => track.enriched.is_none(),
        }
}

fn valid_track_tag(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_TRACK_TAG_CHARS
        && !value.chars().any(char::is_control)
}

fn invalid_track_request() -> ApiError {
    ApiError::from_reason(InternalReason::RequestInvalid).with_field(PublicField::Request)
}

fn invalid_catalog_output() -> ApiError {
    ApiError::from_reason(InternalReason::StorageIntegrityFailed)
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
