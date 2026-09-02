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
    storage::{AppPaths, Storage},
};

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
