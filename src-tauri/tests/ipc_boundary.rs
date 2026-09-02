use std::cell::Cell;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use cyberkindred_lib::ipc::{
    ApiError, CapabilitiesService, CommandTimeout, Deadline, EmptyRequest, ErrorId,
    IdempotencyStore, IdempotencyWindow, InternalReason, ProcessSequence, ProviderKind,
    RedactedDiagnostic, Revision, SequenceObservation, SequenceTracker, SourceCapabilities,
    SourceKind, SourceSummary, canonical_request_hash,
};
use serde_json::json;
use uuid::Uuid;

// API-001; FR-RAD-004; FR-SET-004; NFR-SEC-002.
#[test]
fn ipc_api_001_fr_rad_004_returns_strict_local_capabilities() {
    let local = SourceSummary::new(
        "local:primary",
        SourceKind::Local,
        "本地曲库",
        true,
        SourceCapabilities {
            play: true,
            pause: true,
            seek: true,
            next: true,
            previous: true,
            set_queue: true,
        },
    )
    .unwrap_or_else(|error| panic!("safe fixture should be accepted: {error:?}"));
    let service = CapabilitiesService::new(
        "0.1.0",
        "Windows 11",
        false,
        vec![local],
        vec![ProviderKind::Metadata],
    )
    .unwrap_or_else(|error| panic!("safe capabilities should be accepted: {error:?}"));

    let value = serde_json::to_value(service.get_capabilities(EmptyRequest {}))
        .unwrap_or_else(|error| panic!("capabilities should serialize: {error}"));
    assert_eq!(value["protocolVersion"], "1.0.0");
    assert_eq!(value["platform"], "windows");
    assert_eq!(value["features"]["localLibrary"], true);
    assert_eq!(value["features"]["musicKit"], false);
    assert_eq!(value["features"]["appleMusicDomControl"], false);
    assert_eq!(value["features"]["externalHttpApi"], false);
    assert_eq!(value["sources"][0]["capabilities"]["setQueue"], true);
    assert!(serde_json::from_value::<EmptyRequest>(json!({ "unexpected": true })).is_err());
}

// API-001; FR-RAD-004; NFR-MAINT-003.
#[test]
fn ipc_api_001_fr_rad_004_rejects_fabricated_system_queue_capability() {
    let result = SourceSummary::new(
        "system:apple_music",
        SourceKind::SystemSession,
        "Apple Music",
        true,
        SourceCapabilities {
            play: true,
            pause: true,
            seek: true,
            next: true,
            previous: true,
            set_queue: true,
        },
    );
    let error = result
        .err()
        .unwrap_or_else(|| panic!("system session setQueue must be rejected"));
    assert_eq!(error.error_id, ErrorId::RequestInvalid);
}

// API Contract section 5.1; NFR-SEC-004; NFR-MAINT-003.
#[test]
fn ipc_error_nfr_sec_004_maps_every_reason_and_redacts_untrusted_text() {
    assert_eq!(InternalReason::ALL.len(), 40);
    let mut reason_codes = HashSet::new();
    for reason in InternalReason::ALL {
        let error = ApiError::from_reason(reason);
        let serialized = serde_json::to_string(&error).unwrap_or_else(|serialize_error| {
            panic!("ApiError should serialize: {serialize_error}")
        });
        assert_eq!(error.error_id, reason.error_id());
        assert_eq!(error.schema_version, "1.0.0");
        assert!((1..=300).contains(&error.safe_message.chars().count()));
        assert!(Uuid::parse_str(&error.correlation_id).is_ok());
        assert!(serialized.contains(reason.as_str()));
        assert!(reason_codes.insert(reason.as_str()));
    }
    assert_eq!(reason_codes.len(), 40);

    let canary = "sk-secret-canary Authorization: Bearer C:\\Users\\Alice\\Music private body";
    let public_error = ApiError::from_reason(InternalReason::ProviderInvalidResponse);
    let serialized = serde_json::to_string(&public_error)
        .unwrap_or_else(|error| panic!("ApiError should serialize: {error}"));
    assert!(!serialized.contains(canary));
    assert_eq!(
        format!("{:?}", RedactedDiagnostic::from_untrusted(canary)),
        "[redacted]"
    );
    assert_eq!(
        RedactedDiagnostic::from_untrusted(canary).to_string(),
        "[redacted]"
    );
}

// API Contract idempotency; NFR-MAINT-003; NFR-SEC-004.
#[test]
fn ipc_idempotency_nfr_maint_003_replays_exact_payload_and_rejects_conflict() {
    let mut store = IdempotencyStore::<u64>::new(4);
    let now = Instant::now();
    let request_id = Uuid::now_v7();
    let executions = Cell::new(0_u64);
    let first_payload = json!({ "sourceId": "local:primary", "expectedRevision": 7 });
    let reordered_payload = json!({ "expectedRevision": 7, "sourceId": "local:primary" });

    let first = store.execute(
        now,
        "api_v1_select_music_source",
        request_id,
        &first_payload,
        IdempotencyWindow::Standard,
        || {
            executions.set(executions.get() + 1);
            Ok(42)
        },
    );
    let replay = store.execute(
        now + Duration::from_secs(1),
        "api_v1_select_music_source",
        request_id,
        &reordered_payload,
        IdempotencyWindow::Standard,
        || {
            executions.set(executions.get() + 1);
            Ok(99)
        },
    );
    assert_eq!(first, Ok(42));
    assert_eq!(replay, Ok(42));
    assert_eq!(executions.get(), 1);

    let conflict = store.execute(
        now + Duration::from_secs(2),
        "api_v1_select_music_source",
        request_id,
        &json!({ "sourceId": "local:other", "expectedRevision": 7 }),
        IdempotencyWindow::Standard,
        || Ok(100),
    );
    let error = conflict
        .err()
        .unwrap_or_else(|| panic!("different payload must conflict"));
    assert_eq!(error.error_id, ErrorId::Conflict);
    assert_eq!(executions.get(), 1);

    let left = canonical_request_hash(&first_payload)
        .unwrap_or_else(|error| panic!("hash should succeed: {error:?}"));
    let right = canonical_request_hash(&reordered_payload)
        .unwrap_or_else(|error| panic!("hash should succeed: {error:?}"));
    assert!(left == right);
}

// API Contract idempotency TTL; NFR-MAINT-003.
#[test]
fn ipc_idempotency_nfr_maint_003_uses_exact_standard_and_media_windows() {
    let now = Instant::now();
    let payload = json!({ "expectedStateRevision": 1 });
    let request_id = Uuid::now_v7();
    let executions = Cell::new(0_u64);
    let mut media_store = IdempotencyStore::<u64>::new(2);

    let run = || {
        executions.set(executions.get() + 1);
        Ok(executions.get())
    };
    assert_eq!(
        media_store.execute(
            now,
            "api_v1_play",
            request_id,
            &payload,
            IdempotencyWindow::MediaControl,
            run,
        ),
        Ok(1)
    );
    assert_eq!(
        media_store.execute(
            now + Duration::from_secs(29),
            "api_v1_play",
            request_id,
            &payload,
            IdempotencyWindow::MediaControl,
            run,
        ),
        Ok(1)
    );
    assert_eq!(
        media_store.execute(
            now + Duration::from_secs(30),
            "api_v1_play",
            request_id,
            &payload,
            IdempotencyWindow::MediaControl,
            run,
        ),
        Ok(2)
    );

    let mut standard_store = IdempotencyStore::<u64>::new(2);
    assert_eq!(
        standard_store.execute(
            now,
            "api_v1_update_settings",
            request_id,
            &payload,
            IdempotencyWindow::Standard,
            run,
        ),
        Ok(3)
    );
    assert_eq!(
        standard_store.execute(
            now + Duration::from_secs(599),
            "api_v1_update_settings",
            request_id,
            &payload,
            IdempotencyWindow::Standard,
            run,
        ),
        Ok(3)
    );
    assert_eq!(
        standard_store.execute(
            now + Duration::from_secs(600),
            "api_v1_update_settings",
            request_id,
            &payload,
            IdempotencyWindow::Standard,
            run,
        ),
        Ok(4)
    );
}

// API Contract revision semantics; FR-RAD-004; NFR-MAINT-003.
#[test]
fn ipc_revision_fr_rad_004_rejects_stale_intent_with_current_revision() {
    let mut revision = Revision::new(8);
    assert!(revision.ensure_expected(8).is_ok());
    assert_eq!(revision.advance(), Ok(9));
    let error = revision
        .ensure_expected(8)
        .err()
        .unwrap_or_else(|| panic!("stale intent must conflict"));
    assert_eq!(error.error_id, ErrorId::Conflict);
    let details = error
        .details
        .unwrap_or_else(|| panic!("revision conflict needs safe details"));
    assert_eq!(details.current_revision, Some(9));
    assert_eq!(details.field.as_deref(), Some("expectedRevision"));
}

// API Contract section 4; FR-RAD-004; NFR-MAINT-003.
#[test]
fn ipc_events_fr_rad_004_require_snapshot_for_gap_and_resume() {
    let sequence = ProcessSequence::default();
    assert_eq!(sequence.next(), Ok(1));
    assert_eq!(sequence.next(), Ok(2));

    let mut tracker = SequenceTracker::new();
    assert!(tracker.requires_snapshot());
    assert_eq!(tracker.observe(1), SequenceObservation::AwaitingSnapshot);
    tracker.snapshot_applied();
    assert_eq!(tracker.observe(2), SequenceObservation::Apply);
    assert_eq!(
        tracker.observe(4),
        SequenceObservation::Gap {
            expected: 3,
            received: 4,
        }
    );
    assert_eq!(tracker.observe(5), SequenceObservation::AwaitingSnapshot);
    tracker.snapshot_applied();
    assert_eq!(tracker.observe(6), SequenceObservation::Apply);
    tracker.require_snapshot();
    assert_eq!(tracker.observe(7), SequenceObservation::AwaitingSnapshot);
}

// API-001 timeout; NFR-MAINT-003.
#[test]
fn ipc_timeout_api_001_is_observable_without_implied_cancellation() {
    let started = Instant::now();
    let deadline = Deadline::new(started, CommandTimeout::Read);
    assert_eq!(CommandTimeout::Read.duration(), Duration::from_secs(2));
    assert!(!deadline.is_elapsed(started + Duration::from_millis(1_999)));
    assert!(deadline.is_elapsed(started + Duration::from_secs(2)));
    assert_eq!(
        deadline.remaining(started + Duration::from_millis(500)),
        Duration::from_millis(1_500)
    );
}
