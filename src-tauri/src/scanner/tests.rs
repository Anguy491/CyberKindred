use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{
    CancelLibraryScanRequest, OperationAccepted, ScanEvent, ScanEventState,
    StartLibraryScanRequest,
    events::ScanEventCounters,
    extractor::{AudioFormat, ExtractedAvailability, detect_format_for_test, extract_track},
    service::{AsyncIdempotency, validate_start_request},
    traversal::{
        ScanCancellation, TRAVERSAL_CHANNEL_CAPACITY, TraversalFile, TraversalItem,
        walk_authorized_root,
    },
};
use crate::ipc::{ApiError, ErrorId, ProcessSequence, canonical_request_hash};

fn copy_audio_fixture(temp: &tempfile::TempDir, fixture_name: &str) -> (PathBuf, TraversalFile) {
    let root = temp.path().join("authorized-library");
    std::fs::create_dir_all(&root).expect("authorized root");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("spikes")
        .join("audio")
        .join("fixtures")
        .join(fixture_name);
    let target = root.join(fixture_name);
    std::fs::copy(source, &target).expect("copy licensed fixture");
    (
        root,
        TraversalFile {
            relative_path: PathBuf::from(fixture_name),
            relative_path_text: fixture_name.to_owned(),
        },
    )
}

#[test]
fn scanner_dtos_are_exact_and_reject_unknown_fields() {
    let request_id = Uuid::now_v7();
    let root_id = Uuid::now_v7();
    let request = StartLibraryScanRequest {
        client_request_id: request_id,
        root_ids: vec![root_id],
    };
    assert_eq!(
        serde_json::to_value(request).expect("request JSON"),
        json!({ "clientRequestId": request_id, "rootIds": [root_id] })
    );
    assert!(
        serde_json::from_value::<StartLibraryScanRequest>(json!({
            "clientRequestId": request_id,
            "rootIds": [],
            "path": "C:/private/canary"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<CancelLibraryScanRequest>(json!({
            "clientRequestId": request_id,
            "operationId": Uuid::now_v7(),
            "force": true
        }))
        .is_err()
    );
}

#[test]
fn scanner_event_is_exact_path_free_and_uses_shared_process_sequence() {
    let sequence = ProcessSequence::default();
    let operation_id = Uuid::now_v7();
    let event = ScanEvent::new(
        &sequence,
        "2026-09-03T01:02:03.004Z".to_owned(),
        operation_id,
        ScanEventState::Completed,
        ScanEventCounters {
            scanned: 9,
            discovered: 7,
            failed: 2,
        },
        None,
    )
    .expect("valid event");
    assert_eq!(event.sequence, 1);
    assert!(event.state.is_terminal());
    assert_eq!(
        serde_json::to_value(&event).expect("event JSON"),
        json!({
            "schemaVersion": "1.0.0",
            "sequence": 1,
            "occurredAt": "2026-09-03T01:02:03.004Z",
            "operationId": operation_id,
            "state": "completed",
            "scanned": 9,
            "discovered": 7,
            "failed": 2,
            "safeMessage": null
        })
    );
    let encoded = serde_json::to_string(&event).expect("encoded event");
    assert!(!encoded.contains(":\\"));
    assert!(!encoded.contains("private/canary"));
}

#[test]
fn scanner_request_rejects_duplicate_non_v7_and_excessive_roots() {
    let root_id = Uuid::now_v7();
    let duplicate = StartLibraryScanRequest {
        client_request_id: Uuid::now_v7(),
        root_ids: vec![root_id, root_id],
    };
    assert_eq!(
        validate_start_request(&duplicate)
            .expect_err("duplicate roots")
            .error_id,
        ErrorId::RequestInvalid
    );
    let invalid = StartLibraryScanRequest {
        client_request_id: Uuid::now_v7(),
        root_ids: vec![Uuid::nil()],
    };
    assert_eq!(
        validate_start_request(&invalid)
            .expect_err("root IDs are opaque v7 IDs")
            .error_id,
        ErrorId::RequestInvalid
    );
    let excessive = StartLibraryScanRequest {
        client_request_id: Uuid::now_v7(),
        root_ids: (0..257).map(|_| Uuid::now_v7()).collect(),
    };
    assert_eq!(
        validate_start_request(&excessive)
            .expect_err("bounded root list")
            .error_id,
        ErrorId::RequestInvalid
    );
}

#[tokio::test]
async fn scanner_idempotency_never_expires_or_evicts_a_live_accept() {
    let store = Arc::new(AsyncIdempotency::<OperationAccepted>::new());
    let request_id = Uuid::now_v7();
    let request = StartLibraryScanRequest {
        client_request_id: request_id,
        root_ids: Vec::new(),
    };
    let hash = canonical_request_hash(&request).expect("request hash");
    let gate = Arc::new(tokio::sync::Notify::new());
    let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
    let first_store = Arc::clone(&store);
    let first_gate = Arc::clone(&gate);
    let first = tokio::spawn(async move {
        first_store
            .execute(request_id, hash, || async move {
                let _ = ready_sender.send(());
                first_gate.notified().await;
                Ok(OperationAccepted {
                    operation_id: Uuid::now_v7(),
                    accepted_at: "2026-09-03T01:02:03.004Z".to_owned(),
                })
            })
            .await
    });
    ready_receiver.await.expect("first operation entered");
    let replay_store = Arc::clone(&store);
    let replay = tokio::spawn(async move {
        replay_store
            .execute(request_id, hash, || async { Err(ApiError::unexpected()) })
            .await
    });
    gate.notify_one();
    let accepted = first.await.expect("first task").expect("first accepted");
    assert_eq!(
        replay.await.expect("replay task").expect("replayed result"),
        accepted
    );
}

#[tokio::test]
async fn scanner_traversal_is_relative_bounded_and_does_not_follow_links() {
    let temp = tempfile::tempdir().expect("temporary root");
    let root = temp.path().join("授权曲库");
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).expect("nested root");
    std::fs::write(nested.join("track.wav"), b"fixture").expect("fixture");
    let outside = temp.path().join("outside.mp3");
    std::fs::write(&outside, b"outside").expect("outside fixture");
    let link = root.join("linked.mp3");
    let link_created = create_file_link(&outside, &link);

    let (sender, mut receiver) = mpsc::channel(TRAVERSAL_CHANNEL_CAPACITY);
    let cancellation = ScanCancellation::default();
    let walked = tokio::task::spawn_blocking({
        let root = root.clone();
        let cancellation = cancellation.clone();
        move || walk_authorized_root(&root, &cancellation, &sender)
    });
    let mut files = Vec::new();
    let mut rejected = 0_u64;
    while let Some(item) = receiver.recv().await {
        match item {
            TraversalItem::File(file) => files.push(file.relative_path_text),
            TraversalItem::Rejected => rejected += 1,
        }
    }
    walked
        .await
        .expect("traversal task")
        .expect("bounded traversal");
    assert_eq!(files, vec!["nested/track.wav"]);
    if link_created {
        assert_eq!(rejected, 1);
    }
    assert!(!files.iter().any(|path| path.contains("outside")));
}

#[test]
fn scanner_extractor_supports_six_formats_and_isolates_bad_files() {
    let matrix = [
        ("tone.mp3", AudioFormat::Mp3),
        ("tone.flac", AudioFormat::Flac),
        ("tone.m4a", AudioFormat::M4a),
        ("tone.aac", AudioFormat::Aac),
        ("tone.wav", AudioFormat::Wav),
        ("tone.ogg", AudioFormat::Ogg),
    ];
    for (fixture, expected_format) in matrix {
        let temp = tempfile::tempdir().expect("temporary root");
        let (root, file) = copy_audio_fixture(&temp, fixture);
        assert_eq!(
            detect_format_for_test(&root.join(fixture)),
            Some(expected_format)
        );
        let extracted =
            extract_track(&root, file, &ScanCancellation::default()).expect("supported fixture");
        assert_eq!(extracted.availability, ExtractedAvailability::Available);
        assert!(extracted.duration_ms > 0);
        assert_eq!(extracted.format, expected_format.as_str());
        assert!(extracted.file_identity.is_some());
    }

    let corrupt_temp = tempfile::tempdir().expect("temporary root");
    let (root, corrupt) = copy_audio_fixture(&corrupt_temp, "corrupt.mp3");
    let extracted = extract_track(&root, corrupt, &ScanCancellation::default())
        .expect("corrupt file becomes an isolated record");
    assert_eq!(extracted.availability, ExtractedAvailability::Corrupt);

    let unsupported_temp = tempfile::tempdir().expect("temporary root");
    let root = unsupported_temp.path().join("authorized-library");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(root.join("notes.txt"), b"not audio").expect("unsupported fixture");
    let extracted = extract_track(
        &root,
        TraversalFile {
            relative_path: PathBuf::from("notes.txt"),
            relative_path_text: "notes.txt".to_owned(),
        },
        &ScanCancellation::default(),
    )
    .expect("unsupported file becomes an isolated record");
    assert_eq!(extracted.availability, ExtractedAvailability::Unsupported);
}

#[cfg(windows)]
fn create_file_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn create_file_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(not(any(unix, windows)))]
fn create_file_link(_target: &std::path::Path, _link: &std::path::Path) -> bool {
    false
}
