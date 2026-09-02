use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::{
    contracts::{ContractRegistry, PlaybackEvent, PlaybackStateStatus},
    ipc::{EmptyRequest, ErrorId, ProcessSequence, SourceCapabilities},
};

#[derive(Default)]
struct FakeEventSink(Mutex<Vec<PlaybackEvent>>);

impl PlaybackEventSink for FakeEventSink {
    fn publish(&self, event: PlaybackEvent) -> Result<(), crate::ipc::ApiError> {
        self.0
            .lock()
            .map_err(|_| crate::ipc::ApiError::unexpected())?
            .push(event);
        Ok(())
    }
}

struct FixedClock;

impl PlaybackClock for FixedClock {
    fn now_rfc3339(&self) -> String {
        "2026-09-03T01:02:03.004Z".to_owned()
    }
}

#[derive(Default)]
struct FakeEngineState {
    calls: Vec<String>,
    session_id: Option<Uuid>,
    snapshot: Option<AudioSnapshot>,
    event_sink: Option<Arc<dyn LocalAudioEngineEventSink>>,
}

struct FakeEngine(Arc<Mutex<FakeEngineState>>);

impl LocalAudioEngine for FakeEngine {
    fn set_event_sink(&mut self, sink: Arc<dyn LocalAudioEngineEventSink>) {
        if let Ok(mut state) = self.0.lock() {
            state.event_sink = Some(sink);
        }
    }

    fn load(
        &mut self,
        track: &LocalTrack,
        session_id: Uuid,
        position_ms: u64,
        autoplay: bool,
    ) -> Result<AudioSnapshot, AudioEngineError> {
        let snapshot = AudioSnapshot {
            position_ms,
            duration_ms: track.duration_ms(),
        };
        let mut state = self.0.lock().map_err(|_| AudioEngineError::MediaInvalid)?;
        state.calls.push(if autoplay {
            "load_playing".to_owned()
        } else {
            "load_paused".to_owned()
        });
        state.session_id = Some(session_id);
        state.snapshot = Some(snapshot);
        Ok(snapshot)
    }

    fn play(&mut self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        self.action("play", session_id)
    }

    fn pause(&mut self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        self.action("pause", session_id)
    }

    fn seek(
        &mut self,
        session_id: Uuid,
        position_ms: u64,
    ) -> Result<AudioSnapshot, AudioEngineError> {
        let mut state = self.0.lock().map_err(|_| AudioEngineError::MediaInvalid)?;
        if state.session_id != Some(session_id) {
            return Err(AudioEngineError::MediaInvalid);
        }
        state.calls.push("seek".to_owned());
        let mut snapshot = state.snapshot.ok_or(AudioEngineError::MediaInvalid)?;
        snapshot.position_ms = position_ms;
        state.snapshot = Some(snapshot);
        Ok(snapshot)
    }

    fn snapshot(&self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        let state = self.0.lock().map_err(|_| AudioEngineError::MediaInvalid)?;
        if state.session_id != Some(session_id) {
            return Err(AudioEngineError::MediaInvalid);
        }
        state.snapshot.ok_or(AudioEngineError::MediaInvalid)
    }

    fn stop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.calls.push("stop".to_owned());
            state.session_id = None;
        }
    }
}

impl FakeEngine {
    fn action(&self, name: &str, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        let mut state = self.0.lock().map_err(|_| AudioEngineError::MediaInvalid)?;
        if state.session_id != Some(session_id) {
            return Err(AudioEngineError::MediaInvalid);
        }
        state.calls.push(name.to_owned());
        state.snapshot.ok_or(AudioEngineError::MediaInvalid)
    }
}

struct FakeResolver {
    tracks: HashMap<String, LocalTrack>,
}

impl LocalTrackResolver for FakeResolver {
    fn resolve(&self, track_id: &str) -> Result<LocalTrack, LocalSourceError> {
        self.tracks
            .get(track_id)
            .cloned()
            .ok_or(LocalSourceError::NotFound)
    }
}

struct Harness {
    service: PlaybackService,
    engine: Arc<Mutex<FakeEngineState>>,
    events: Arc<FakeEventSink>,
    _temp: tempfile::TempDir,
}

fn harness(capabilities: SourceCapabilities) -> Harness {
    let temp = tempfile::tempdir().expect("temporary playback root");
    let first = track(temp.path().join("first.wav"), "track-1", "第一首", 60_000);
    let second = track(temp.path().join("second.flac"), "track-2", "第二首", 90_000);
    let resolver = Arc::new(FakeResolver {
        tracks: [
            ("track-1".to_owned(), first),
            ("track-2".to_owned(), second),
        ]
        .into_iter()
        .collect(),
    });
    let engine = Arc::new(Mutex::new(FakeEngineState::default()));
    let events = Arc::new(FakeEventSink::default());
    let service = PlaybackService::new(
        resolver,
        Box::new(FakeEngine(Arc::clone(&engine))),
        events.clone(),
        Arc::new(FixedClock),
        Arc::new(ProcessSequence::default()),
        capabilities,
    )
    .expect("valid playback service");
    Harness {
        service,
        engine,
        events,
        _temp: temp,
    }
}

fn track(path: PathBuf, id: &str, title: &str, duration_ms: u64) -> LocalTrack {
    LocalTrack::new(
        id,
        title,
        Some("Fixture Artist".to_owned()),
        Some("Fixture Album".to_owned()),
        Some("asset://covers/fixture.webp".to_owned()),
        path,
        duration_ms,
    )
    .expect("valid fake track")
}

fn control(revision: u64) -> PlaybackControlRequest {
    PlaybackControlRequest {
        client_request_id: Uuid::now_v7(),
        expected_state_revision: revision,
    }
}

#[test]
fn local_source_dtos_are_exact_and_reject_unknown_fields() {
    let request_id = Uuid::now_v7();
    let request: SeekPlaybackRequest = serde_json::from_value(json!({
        "clientRequestId": request_id,
        "expectedStateRevision": 3,
        "positionMs": 99
    }))
    .expect("exact seek request");
    assert_eq!(request.position_ms, 99);
    assert!(
        serde_json::from_value::<SeekPlaybackRequest>(json!({
            "clientRequestId": request_id,
            "expectedStateRevision": 3,
            "positionMs": 99,
            "path": "C:\\private\\track.wav"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn playback_actor_loads_silently_until_explicit_play_and_replays_idempotently() {
    let harness = harness(PlaybackService::local_capabilities());
    let loaded = harness
        .service
        .set_local_queue(vec!["track-1".to_owned(), "track-2".to_owned()], None)
        .await
        .expect("queue accepted silently");
    assert_eq!(loaded.status, PlaybackStateStatus::Idle);
    assert_eq!(
        harness.engine.lock().expect("engine state").calls,
        vec!["stop"]
    );

    let request_id = Uuid::now_v7();
    let request = PlaybackControlRequest {
        client_request_id: request_id,
        expected_state_revision: loaded.revision,
    };
    let playing = harness.service.play(request.clone()).await.expect("play");
    let replay = harness.service.play(request).await.expect("replayed play");
    assert_eq!(playing, replay);
    assert_eq!(playing.status, PlaybackStateStatus::Playing);
    assert_eq!(
        harness
            .engine
            .lock()
            .expect("engine state")
            .calls
            .iter()
            .filter(|call| call.as_str() == "load_playing")
            .count(),
        1
    );
}

#[tokio::test]
async fn playback_actor_rejects_stale_revision_and_missing_capability_before_side_effect() {
    let mut capabilities = PlaybackService::local_capabilities();
    capabilities.seek = false;
    let harness = harness(capabilities);
    let loaded = harness
        .service
        .set_local_queue(vec!["track-1".to_owned()], None)
        .await
        .expect("queue loaded");
    let calls_before = harness.engine.lock().expect("engine state").calls.len();
    let error = harness
        .service
        .seek(SeekPlaybackRequest {
            client_request_id: Uuid::now_v7(),
            expected_state_revision: loaded.revision,
            position_ms: 1_000,
        })
        .await
        .expect_err("seek capability absent");
    assert_eq!(error.error_id, ErrorId::CapabilityUnsupported);
    assert_eq!(
        harness.engine.lock().expect("engine state").calls.len(),
        calls_before
    );

    let stale = harness
        .service
        .play(control(loaded.revision.saturating_add(1)))
        .await
        .expect_err("stale state revision");
    assert_eq!(stale.error_id, ErrorId::Conflict);
    assert_eq!(
        stale.details.and_then(|details| details.current_revision),
        Some(loaded.revision)
    );
    assert_eq!(
        harness.engine.lock().expect("engine state").calls.len(),
        calls_before
    );
}

#[tokio::test]
async fn local_source_controls_queue_position_previous_and_safe_stop() {
    let harness = harness(PlaybackService::local_capabilities());
    let queued = harness
        .service
        .set_local_queue(vec!["track-1".to_owned(), "track-2".to_owned()], None)
        .await
        .expect("queue accepted silently");
    let first = harness
        .service
        .play(control(queued.revision))
        .await
        .expect("first track playing");
    let paused = harness
        .service
        .pause(control(first.revision))
        .await
        .expect("pause first");
    let second = harness
        .service
        .next(control(paused.revision))
        .await
        .expect("next");
    assert_eq!(second.status, PlaybackStateStatus::Paused);
    assert_eq!(
        second
            .current_track
            .as_ref()
            .map(|track| track.track_id.as_str()),
        Some("track-2")
    );
    let sought = harness
        .service
        .seek(SeekPlaybackRequest {
            client_request_id: Uuid::now_v7(),
            expected_state_revision: second.revision,
            position_ms: 6_000,
        })
        .await
        .expect("seek second");
    let restarted = harness
        .service
        .previous(control(sought.revision))
        .await
        .expect("previous after five seconds restarts current");
    assert_eq!(restarted.position_ms, 0);
    assert_eq!(
        restarted
            .current_track
            .as_ref()
            .map(|track| track.track_id.as_str()),
        Some("track-2")
    );
    let stopped = harness.service.stop_local().await.expect("safe stop");
    assert_eq!(stopped.status, PlaybackStateStatus::Stopped);
    assert!(stopped.current_track.is_none());
    assert_eq!(stopped.position_ms, 0);
}

#[tokio::test]
async fn playback_actor_drops_stale_session_events_and_recovers_silent_after_device_or_sleep() {
    let harness = harness(PlaybackService::local_capabilities());
    let loaded = harness
        .service
        .set_local_queue(
            vec!["track-1".to_owned()],
            Some(PlaybackStartAuthorization::Manual),
        )
        .await
        .expect("authorized autoplay");
    assert_eq!(loaded.status, PlaybackStateStatus::Playing);
    let session_id = harness
        .engine
        .lock()
        .expect("engine state")
        .session_id
        .expect("active session");

    harness
        .service
        .handle_engine_event(LocalAudioEngineEvent::Position {
            session_id: Uuid::now_v7(),
            position_ms: 45_000,
        })
        .await
        .expect("stale event accepted as no-op");
    let unchanged = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("state after stale event");
    assert_eq!(unchanged.revision, loaded.revision);
    assert_eq!(unchanged.position_ms, 0);

    harness
        .service
        .handle_engine_event(LocalAudioEngineEvent::OutputLost {
            session_id,
            position_ms: 7_000,
        })
        .await
        .expect("device loss queued");
    let recovering = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("recovering snapshot");
    assert_eq!(recovering.status, PlaybackStateStatus::Error);
    assert_eq!(
        recovering
            .last_error
            .as_ref()
            .map(|error| error.error_id.as_str()),
        Some("ERR-1202")
    );

    harness
        .service
        .handle_engine_event(LocalAudioEngineEvent::OutputRestored {
            recovery_session_id: session_id,
        })
        .await
        .expect("device restore queued");
    let restored = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("restored state");
    assert_eq!(restored.status, PlaybackStateStatus::Paused);
    assert_eq!(restored.position_ms, 7_000);
    assert_eq!(
        harness
            .engine
            .lock()
            .expect("engine state")
            .calls
            .last()
            .map(String::as_str),
        Some("load_paused")
    );

    let playing = harness
        .service
        .play(control(restored.revision))
        .await
        .expect("explicit play after recovery");
    let suspended = harness.service.prepare_suspend().await.expect("suspend");
    assert_eq!(suspended.status, PlaybackStateStatus::Paused);
    let resumed = harness
        .service
        .resume_silent()
        .await
        .expect("resume silent");
    assert_eq!(resumed.status, PlaybackStateStatus::Paused);
    assert!(resumed.revision > playing.revision);
    assert_eq!(
        harness
            .engine
            .lock()
            .expect("engine state")
            .calls
            .last()
            .map(String::as_str),
        Some("load_paused")
    );
}

#[tokio::test]
async fn playback_actor_emits_schema_valid_monotonic_at_most_once_events_without_paths() {
    let harness = harness(PlaybackService::local_capabilities());
    let queued = harness
        .service
        .set_local_queue(vec!["track-1".to_owned()], None)
        .await
        .expect("queue accepted silently");
    harness
        .service
        .play(control(queued.revision))
        .await
        .expect("play");
    let events = harness.events.0.lock().expect("events").clone();
    assert!(!events.is_empty());
    let registry = ContractRegistry::new().expect("contract registry");
    for (index, event) in events.iter().enumerate() {
        assert_eq!(
            event.sequence,
            u64::try_from(index + 1).expect("small index")
        );
        registry
            .validate(
                "playback-event",
                &serde_json::to_value(event).expect("event JSON"),
            )
            .expect("EVT-001 schema");
        let encoded = serde_json::to_string(event).expect("event JSON");
        assert!(!encoded.contains(":\\"));
        assert!(!encoded.contains("private"));
    }
}
