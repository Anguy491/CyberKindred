use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::{
    ipc::ErrorId,
    storage::{
        AppPaths, ScanCounters, ScanTrackAvailability, ScanTrackRecord, Storage, StorageError,
        StorageReason,
    },
};

struct FakeTrackCatalog {
    queries: Mutex<Vec<TrackCatalogQuery>>,
    results: Mutex<VecDeque<Result<TracksPage, StorageError>>>,
}

impl FakeTrackCatalog {
    fn new(results: impl IntoIterator<Item = Result<TracksPage, StorageError>>) -> Self {
        Self {
            queries: Mutex::new(Vec::new()),
            results: Mutex::new(results.into_iter().collect()),
        }
    }

    fn queries(&self) -> Vec<TrackCatalogQuery> {
        self.queries.lock().expect("catalog query lock").clone()
    }
}

impl TrackCatalog for FakeTrackCatalog {
    fn list_tracks(&self, query: TrackCatalogQuery) -> TrackCatalogFuture<'_> {
        self.queries.lock().expect("catalog query lock").push(query);
        let result = self
            .results
            .lock()
            .expect("catalog result lock")
            .pop_front()
            .unwrap_or_else(|| Err(StorageError::new(StorageReason::StorageReadFailed)));
        Box::pin(async move { result })
    }
}

fn list_tracks_request() -> ListTracksRequest {
    ListTracksRequest {
        cursor: None,
        limit: 50,
        query: Some(" 夜曲 ".to_owned()),
        sort: TrackSort::Artist,
        filters: TrackFilters {
            availability: Some(TrackAvailabilityFilter::Playable),
            match_status: Some(TrackMatchStatus::Matched),
        },
    }
}

fn track_view(track_id: Uuid) -> TrackView {
    TrackView {
        track_id,
        availability: TrackAvailability::Playable,
        duration_ms: 180_000,
        artwork_available: true,
        original: TrackTagView {
            title: Some("原始曲名".to_owned()),
            artist: Some("本地艺术家".to_owned()),
            album: None,
        },
        enriched: Some(EnrichedTrackTagView {
            title: Some("补全曲名".to_owned()),
            artist: Some("补全艺术家".to_owned()),
            album: Some("补全专辑".to_owned()),
            provider: TrackMetadataProvider::Musicbrainz,
            confidence: 0.97,
            fetched_at: "2026-09-03T01:02:03.000Z".to_owned(),
        }),
        match_status: TrackMatchStatus::Matched,
    }
}

struct FakePicker {
    outcomes: Mutex<VecDeque<Result<Option<PathBuf>, LibraryRootPickerError>>>,
    calls: AtomicUsize,
}

impl FakePicker {
    fn new(
        outcomes: impl IntoIterator<Item = Result<Option<PathBuf>, LibraryRootPickerError>>,
    ) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

impl LibraryRootPicker for FakePicker {
    fn pick_directory(&self) -> LibraryRootPickerFuture<'_> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let outcome = self
            .outcomes
            .lock()
            .expect("fake picker lock")
            .pop_front()
            .unwrap_or(Ok(None));
        Box::pin(async move { outcome })
    }
}

struct FixedClock(i64);

impl LibraryRootClock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

async fn fixture(
    picker: Arc<FakePicker>,
) -> (tempfile::TempDir, AppPaths, Storage, LibraryRootService) {
    let temp = tempfile::tempdir().expect("temporary root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("scoped paths");
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let service =
        LibraryRootService::new(storage.repository(), picker, Arc::new(FixedClock(1_000)));
    (temp, paths, storage, service)
}

fn add_request() -> PickAndAddLibraryRootRequest {
    PickAndAddLibraryRootRequest {
        client_request_id: Uuid::now_v7(),
    }
}

#[tokio::test]
async fn library_root_cancel_is_a_cached_revision_preserving_no_op() {
    let picker = Arc::new(FakePicker::new([Ok(None)]));
    let (_temp, _paths, storage, service) = fixture(picker.clone()).await;
    let request = add_request();
    let first = service
        .pick_and_add_library_root(request.clone())
        .await
        .expect("cancel response");
    let retry = service
        .pick_and_add_library_root(request)
        .await
        .expect("idempotent cancel retry");
    assert_eq!(first, retry);
    assert_eq!(first.root, None);
    assert_eq!(first.revision, 0);
    assert_eq!(picker.calls(), 1);
    assert!(
        service
            .list_library_roots()
            .await
            .expect("list")
            .roots
            .is_empty()
    );
    storage.close().await;
}

#[tokio::test]
async fn library_root_duplicate_restart_and_dtos_never_expose_path() {
    let picker = Arc::new(FakePicker::new([]));
    let (temp, paths, storage, _service) = fixture(picker).await;
    let private_parent = temp.path().join("absolute-path-canary-never-cross-ipc");
    let music = private_parent.join("Music");
    std::fs::create_dir_all(&music).expect("music directory");
    let picker = Arc::new(FakePicker::new([Ok(Some(music.clone())), Ok(Some(music))]));
    let service =
        LibraryRootService::new(storage.repository(), picker, Arc::new(FixedClock(1_000)));
    let first = service
        .pick_and_add_library_root(add_request())
        .await
        .expect("first add");
    let duplicate = service
        .pick_and_add_library_root(add_request())
        .await
        .expect("duplicate add");
    assert_eq!(duplicate.root, first.root);
    assert_eq!(duplicate.revision, 1);
    let encoded = serde_json::to_string(&duplicate).expect("response serialization");
    assert!(!encoded.contains("absolute-path-canary-never-cross-ipc"));
    assert!(!encoded.contains("canonicalPath"));
    drop(service);
    storage.close().await;

    let reopened = Storage::open(&paths, "0.1.0").await.expect("reopen");
    let restarted = LibraryRootService::new(
        reopened.repository(),
        Arc::new(FakePicker::new([])),
        Arc::new(FixedClock(2_000)),
    );
    let snapshot = restarted.list_library_roots().await.expect("restart list");
    assert_eq!(snapshot.roots, vec![first.root.expect("added root")]);
    assert_eq!(snapshot.revision, 1);
    let encoded = serde_json::to_string(&snapshot).expect("snapshot serialization");
    assert!(!encoded.contains("absolute-path-canary-never-cross-ipc"));
    reopened.close().await;
}

#[tokio::test]
async fn library_root_remove_enforces_revision_and_only_soft_disables() {
    let picker = Arc::new(FakePicker::new([]));
    let (temp, _paths, storage, _service) = fixture(picker).await;
    let music = temp.path().join("Music");
    std::fs::create_dir(&music).expect("music directory");
    let service = LibraryRootService::new(
        storage.repository(),
        Arc::new(FakePicker::new([Ok(Some(music))])),
        Arc::new(FixedClock(1_000)),
    );
    let added = service
        .pick_and_add_library_root(add_request())
        .await
        .expect("add")
        .root
        .expect("root");
    let stale = service
        .remove_library_root(RemoveLibraryRootRequest {
            client_request_id: Uuid::now_v7(),
            root_id: added.root_id,
            expected_revision: 0,
        })
        .await
        .expect_err("stale revision");
    assert_eq!(stale.error_id, ErrorId::Conflict);
    assert_eq!(
        stale
            .details
            .as_ref()
            .and_then(|details| details.current_revision),
        Some(1)
    );
    assert!(
        !serde_json::to_string(&stale)
            .expect("safe error")
            .contains("Music")
    );

    let remove_request_id = Uuid::now_v7();
    let removed = service
        .remove_library_root(RemoveLibraryRootRequest {
            client_request_id: remove_request_id,
            root_id: added.root_id,
            expected_revision: 1,
        })
        .await
        .expect("remove");
    assert_eq!(removed.revision, 2);
    let payload_conflict = service
        .remove_library_root(RemoveLibraryRootRequest {
            client_request_id: remove_request_id,
            root_id: added.root_id,
            expected_revision: 0,
        })
        .await
        .expect_err("same id with different payload");
    assert_eq!(payload_conflict.error_id, ErrorId::Conflict);
    assert_eq!(
        payload_conflict
            .details
            .as_ref()
            .and_then(|details| details.reason.as_deref()),
        Some("idempotency_payload_conflict")
    );
    let snapshot = service.list_library_roots().await.expect("list");
    assert!(snapshot.roots.is_empty());
    assert_eq!(snapshot.revision, 2);
    storage.close().await;
}

#[tokio::test]
async fn library_root_non_directory_error_is_redacted() {
    let picker = Arc::new(FakePicker::new([]));
    let (temp, _paths, storage, _service) = fixture(picker).await;
    let file = temp.path().join("private-path-canary.txt");
    std::fs::write(&file, b"fixture").expect("fixture file");
    let service = LibraryRootService::new(
        storage.repository(),
        Arc::new(FakePicker::new([Ok(Some(file))])),
        Arc::new(FixedClock(1_000)),
    );
    let error = service
        .pick_and_add_library_root(add_request())
        .await
        .expect_err("file rejected");
    assert_eq!(error.error_id, ErrorId::PathDenied);
    let encoded = serde_json::to_string(&error).expect("safe error");
    assert!(!encoded.contains("private-path-canary"));
    assert_eq!(
        service.list_library_roots().await.expect("list").revision,
        0
    );
    storage.close().await;
}

#[tokio::test]
async fn library_root_idempotency_is_bounded_without_live_eviction() {
    let picker = Arc::new(FakePicker::new([Ok(None)]));
    let (_temp, _paths, storage, _service) = fixture(picker.clone()).await;
    let service = LibraryRootService::with_capacity(
        storage.repository(),
        picker.clone(),
        Arc::new(FixedClock(1_000)),
        1,
    );
    let first_request = add_request();
    let first = service
        .pick_and_add_library_root(first_request.clone())
        .await
        .expect("first result");
    let full = service
        .pick_and_add_library_root(add_request())
        .await
        .expect_err("live key retained");
    assert_eq!(full.error_id, ErrorId::ResourceBusy);
    assert_eq!(
        service
            .pick_and_add_library_root(first_request)
            .await
            .expect("first replay"),
        first
    );
    assert_eq!(picker.calls(), 1);
    storage.close().await;
}

#[test]
fn library_root_requests_are_strict_and_have_no_path_field() {
    let request_id = Uuid::now_v7();
    assert!(
        serde_json::from_value::<PickAndAddLibraryRootRequest>(json!({
            "clientRequestId": request_id,
            "path": "C:\\private-canary"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<RemoveLibraryRootRequest>(json!({
            "clientRequestId": request_id,
            "rootId": Uuid::now_v7(),
            "expectedRevision": 0,
            "path": "C:\\private-canary"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn list_tracks_forwards_a_bounded_normalized_query_and_returns_exact_dto() {
    let page = TracksPage {
        items: vec![track_view(Uuid::now_v7())],
        next_cursor: Some("v1:50".to_owned()),
    };
    let catalog = Arc::new(FakeTrackCatalog::new([Ok(page.clone())]));
    let service = TrackCatalogService::new(catalog.clone());
    let response = service
        .list_tracks(list_tracks_request())
        .await
        .expect("valid track page");
    assert_eq!(response, page);
    assert_eq!(
        catalog.queries(),
        vec![TrackCatalogQuery {
            cursor: None,
            limit: 50,
            query: Some("夜曲".to_owned()),
            sort: TrackSort::Artist,
            availability: Some(TrackAvailabilityFilter::Playable),
            match_status: Some(TrackMatchStatus::Matched),
        }]
    );
    let encoded = serde_json::to_string(&response).expect("track page serialization");
    assert!(encoded.contains("\"provider\":\"musicbrainz\""));
    assert!(encoded.contains("\"fetchedAt\":\"2026-09-03T01:02:03.000Z\""));
    assert!(!encoded.to_lowercase().contains("path"));
}

#[tokio::test]
async fn list_tracks_rejects_invalid_bounds_before_calling_catalog() {
    let catalog = Arc::new(FakeTrackCatalog::new([]));
    let service = TrackCatalogService::new(catalog.clone());
    for request in [
        ListTracksRequest {
            limit: 0,
            ..list_tracks_request()
        },
        ListTracksRequest {
            limit: MAX_TRACK_PAGE_SIZE + 1,
            ..list_tracks_request()
        },
        ListTracksRequest {
            cursor: Some(String::new()),
            ..list_tracks_request()
        },
        ListTracksRequest {
            cursor: Some("not-an-opaque-cursor".to_owned()),
            ..list_tracks_request()
        },
        ListTracksRequest {
            query: Some("private\nquery".to_owned()),
            ..list_tracks_request()
        },
    ] {
        let error = service
            .list_tracks(request)
            .await
            .expect_err("invalid list request");
        assert_eq!(error.error_id, ErrorId::RequestInvalid);
    }
    assert!(catalog.queries().is_empty());
}

#[tokio::test]
async fn list_tracks_fails_closed_on_duplicate_or_inconsistent_adapter_rows() {
    let track_id = Uuid::now_v7();
    let duplicate = track_view(track_id);
    let mut inconsistent = track_view(Uuid::now_v7());
    inconsistent.match_status = TrackMatchStatus::Unmatched;
    let catalog = Arc::new(FakeTrackCatalog::new([
        Ok(TracksPage {
            items: vec![duplicate.clone(), duplicate],
            next_cursor: None,
        }),
        Ok(TracksPage {
            items: vec![inconsistent],
            next_cursor: None,
        }),
    ]));
    let service = TrackCatalogService::new(catalog);
    let mut request = list_tracks_request();
    request.limit = 2;
    for _ in 0..2 {
        let error = service
            .list_tracks(request.clone())
            .await
            .expect_err("invalid catalog output");
        assert_eq!(error.error_id, ErrorId::StorageFailed);
        assert!(!error.safe_message.contains("原始曲名"));
    }
}

#[test]
fn list_tracks_request_is_strict_and_has_no_path_field() {
    let valid = json!({
        "cursor": null,
        "limit": 50,
        "query": "night",
        "sort": "recent",
        "filters": { "availability": "playable", "matchStatus": null }
    });
    assert!(serde_json::from_value::<ListTracksRequest>(valid.clone()).is_ok());
    let mut unknown = valid;
    unknown
        .as_object_mut()
        .expect("request object")
        .insert("path".to_owned(), json!("C:\\private-canary"));
    assert!(serde_json::from_value::<ListTracksRequest>(unknown).is_err());
}

#[tokio::test]
async fn list_tracks_repository_adapter_maps_an_opaque_bounded_cursor_without_paths() {
    let picker = Arc::new(FakePicker::new([]));
    let (temp, _paths, storage, _service) = fixture(picker).await;
    let private_root = temp.path().join("private-path-canary");
    std::fs::create_dir_all(&private_root).expect("library root");
    let repository = storage.repository();
    let root_id = repository
        .add_library_root(&private_root, 100)
        .await
        .expect("authorize root")
        .root
        .root_id;
    let operation = repository
        .accept_scan_operation(Uuid::now_v7(), &[root_id], 200, "0.3.0")
        .await
        .expect("accept scan");
    repository
        .mark_scan_operation_running(operation.operation_id(), 201)
        .await
        .expect("start scan");
    let job_id = operation.roots()[0].job_id();
    let records = ["Alpha", "Beta"].map(|title| ScanTrackRecord {
        relative_path: PathBuf::from(format!("{title}.mp3")),
        file_identity: Some(format!("identity-{title}")),
        availability: ScanTrackAvailability::Available,
        format: "mp3".to_owned(),
        file_size_bytes: 10,
        modified_at_ms: 1,
        duration_ms: 60_000,
        title: Some(title.to_owned()),
        artist: None,
        album: None,
        album_artist: None,
        track_number: None,
        disc_number: None,
        year: None,
        genres: Vec::new(),
        metadata_confidence: 1.0,
        embedded_cover_hash: None,
    });
    repository
        .upsert_scan_batch(
            operation.operation_id(),
            job_id,
            &records,
            ScanCounters {
                files_seen: 2,
                tracks_indexed: 2,
                errors_count: 0,
            },
            300,
        )
        .await
        .expect("seed tracks");
    let service = TrackCatalogService::new(Arc::new(repository.clone()));
    let first = service
        .list_tracks(ListTracksRequest {
            cursor: None,
            limit: 1,
            query: None,
            sort: TrackSort::Title,
            filters: TrackFilters {
                availability: None,
                match_status: None,
            },
        })
        .await
        .expect("first page");
    assert_eq!(first.items[0].original.title.as_deref(), Some("Alpha"));
    assert_eq!(first.next_cursor.as_deref(), Some("v1:1"));
    let second = service
        .list_tracks(ListTracksRequest {
            cursor: first.next_cursor,
            limit: 1,
            query: None,
            sort: TrackSort::Title,
            filters: TrackFilters {
                availability: None,
                match_status: None,
            },
        })
        .await
        .expect("second page");
    assert_eq!(second.items[0].original.title.as_deref(), Some("Beta"));
    assert_eq!(second.next_cursor, None);
    let encoded = serde_json::to_string(&second).expect("path-free response");
    assert!(!encoded.contains("private-path-canary"));
    drop(service);
    drop(repository);
    storage.close().await;
}
