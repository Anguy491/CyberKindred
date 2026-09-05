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
    ipc::{CapabilityName, EmptyRequest, ErrorId, ProcessSequence, SourceCapabilities},
};

use super::system_media::{
    SystemMediaBackend, SystemMediaControl, SystemMediaError, SystemMediaEventSink,
    SystemMediaSnapshot,
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

struct FakeSystemState {
    available: bool,
    snapshot: SystemMediaSnapshot,
    controls: Vec<SystemMediaControl>,
    refreshes: u64,
    sink: Option<Arc<dyn SystemMediaEventSink>>,
}

struct FakeSystemBackend(Arc<Mutex<FakeSystemState>>);

impl SystemMediaBackend for FakeSystemBackend {
    fn set_event_sink(&mut self, sink: Arc<dyn SystemMediaEventSink>) {
        if let Ok(mut state) = self.0.lock() {
            state.sink = Some(sink);
        }
    }

    fn refresh(&mut self) -> Result<SystemMediaSnapshot, SystemMediaError> {
        let mut state = self.0.lock().map_err(|_| SystemMediaError::Unavailable)?;
        state.refreshes = state.refreshes.saturating_add(1);
        if state.available {
            Ok(state.snapshot.clone())
        } else {
            Err(SystemMediaError::Unavailable)
        }
    }

    fn control(
        &mut self,
        expected_identity: &str,
        action: SystemMediaControl,
    ) -> Result<SystemMediaSnapshot, SystemMediaError> {
        let mut state = self.0.lock().map_err(|_| SystemMediaError::Unavailable)?;
        if !state.available {
            return Err(SystemMediaError::Unavailable);
        }
        if state.snapshot.identity != expected_identity {
            return Err(SystemMediaError::SessionChanged);
        }
        let (enabled, capability) = match action {
            SystemMediaControl::Play => (state.snapshot.capabilities.play, CapabilityName::Play),
            SystemMediaControl::Pause => (state.snapshot.capabilities.pause, CapabilityName::Pause),
            SystemMediaControl::Seek(_) => (state.snapshot.capabilities.seek, CapabilityName::Seek),
            SystemMediaControl::Next => (state.snapshot.capabilities.next, CapabilityName::Next),
            SystemMediaControl::Previous => (
                state.snapshot.capabilities.previous,
                CapabilityName::Previous,
            ),
        };
        if !enabled {
            return Err(SystemMediaError::CapabilityAbsent(capability));
        }
        state.controls.push(action);
        match action {
            SystemMediaControl::Play => state.snapshot.status = PlaybackStateStatus::Playing,
            SystemMediaControl::Pause => state.snapshot.status = PlaybackStateStatus::Paused,
            SystemMediaControl::Seek(position_ms) => state.snapshot.position_ms = position_ms,
            SystemMediaControl::Next | SystemMediaControl::Previous => {}
        }
        Ok(state.snapshot.clone())
    }
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
        ArtworkAssetStore::default(),
    )
    .expect("valid playback service");
    Harness {
        service,
        engine,
        events,
        _temp: temp,
    }
}

fn system_snapshot() -> SystemMediaSnapshot {
    SystemMediaSnapshot {
        identity: "bound-session-a".to_owned(),
        status: PlaybackStateStatus::Paused,
        capabilities: SourceCapabilities {
            play: true,
            pause: true,
            seek: false,
            next: true,
            previous: true,
            set_queue: false,
        },
        title: Some("真实标题".to_owned()),
        artist: Some("真实艺术家".to_owned()),
        album: None,
        artwork_uri: Some(
            "asset://artwork/system/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
        ),
        position_ms: 7_000,
        duration_ms: Some(180_000),
    }
}

fn system_harness() -> (Harness, Arc<Mutex<FakeSystemState>>) {
    let temp = tempfile::tempdir().expect("temporary playback root");
    let resolver = Arc::new(FakeResolver {
        tracks: HashMap::new(),
    });
    let engine = Arc::new(Mutex::new(FakeEngineState::default()));
    let events = Arc::new(FakeEventSink::default());
    let system = Arc::new(Mutex::new(FakeSystemState {
        available: true,
        snapshot: system_snapshot(),
        controls: Vec::new(),
        refreshes: 0,
        sink: None,
    }));
    let service = PlaybackService::new_with_system_backend(
        resolver,
        Box::new(FakeEngine(Arc::clone(&engine))),
        events.clone(),
        Arc::new(FixedClock),
        Arc::new(ProcessSequence::default()),
        PlaybackService::local_capabilities(),
        Box::new(FakeSystemBackend(Arc::clone(&system))),
    )
    .expect("playback service with fake system source");
    (
        Harness {
            service,
            engine,
            events,
            _temp: temp,
        },
        system,
    )
}

#[tokio::test]
async fn system_media_source_waits_for_explicit_selection_before_reading_the_session() {
    let (harness, system) = system_harness();

    let sources = harness.service.list_music_sources(EmptyRequest {}).sources;
    let apple = sources
        .iter()
        .find(|source| source.source_id == "apple_music")
        .expect("Apple source summary");
    assert!(!apple.connected);
    assert_eq!(system.lock().expect("system state").refreshes, 0);

    harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("explicit Apple selection reads the session");
    assert!(system.lock().expect("system state").refreshes >= 1);
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

#[tokio::test]
async fn system_media_source_maps_only_observed_fields_and_never_advertises_queue_control() {
    let (harness, _) = system_harness();
    let selected = harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected")
        .state;
    assert_eq!(
        selected.source_kind,
        crate::contracts::PlaybackStateSourceKind::SystemSession
    );
    assert_eq!(selected.status, PlaybackStateStatus::Paused);
    assert!(!selected.capabilities.set_queue);
    let track = selected
        .current_track
        .expect("observed title creates track");
    assert_eq!(track.title, "真实标题");
    assert_eq!(track.artist.as_deref(), Some("真实艺术家"));
    assert!(track.album.is_none());
    assert_eq!(
        track.artwork_uri.as_deref(),
        Some(
            "asset://artwork/system/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        )
    );
    assert!(track.track_id.starts_with("system:"));
    assert!(!track.track_id.contains("真实标题"));

    let sources = harness.service.list_music_sources(EmptyRequest {}).sources;
    let apple = sources
        .iter()
        .find(|source| source.source_id == "apple_music")
        .expect("Apple source summary");
    assert!(apple.connected);
    assert!(!apple.capabilities.set_queue);
}

#[tokio::test]
async fn system_media_source_treats_closed_snapshot_as_disconnected_and_unselectable() {
    let (harness, system) = system_harness();
    {
        let mut system = system.lock().expect("system state");
        system.snapshot.status = PlaybackStateStatus::Disconnected;
        system.snapshot.position_ms = 42_000;
        system.snapshot.duration_ms = Some(180_000);
    }
    let error = harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect_err("closed session must not be selectable");
    assert_eq!(error.error_id, ErrorId::SourceUnavailable);
    let apple = harness
        .service
        .list_music_sources(EmptyRequest {})
        .sources
        .into_iter()
        .find(|source| source.source_id == "apple_music")
        .expect("Apple source summary");
    assert!(!apple.connected);
    assert!(!apple.capabilities.play);
    assert!(!apple.capabilities.pause);
    assert!(!apple.capabilities.seek);
    assert!(!apple.capabilities.next);
    assert!(!apple.capabilities.previous);
    assert!(!apple.capabilities.set_queue);
}

#[tokio::test]
async fn system_media_source_publishes_coalesced_position_state_for_ui_and_companion() {
    let (harness, system) = system_harness();
    let selected = harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected")
        .state;
    system.lock().expect("system state").snapshot.position_ms = 8_000;

    let refreshed = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("position refresh");
    assert_eq!(refreshed.position_ms, 8_000);
    assert!(refreshed.revision > selected.revision);
    let event = harness
        .events
        .0
        .lock()
        .expect("events")
        .last()
        .cloned()
        .expect("position event");
    assert_eq!(
        event.r#type,
        crate::contracts::PlaybackEventType::StateChanged
    );
    assert_eq!(event.state_revision, refreshed.revision);
    assert_eq!(
        event.state.as_ref().map(|state| state.position_ms),
        Some(8_000)
    );
}

#[tokio::test]
async fn persisted_apple_default_selects_the_disconnected_system_source_without_control() {
    let (harness, system) = system_harness();
    system.lock().expect("system state").available = false;

    harness
        .service
        .apply_initial_source(Some("apple_music"))
        .expect("known default source");
    let state = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("disconnected Apple projection");

    assert_eq!(state.source_id, "apple_music");
    assert_eq!(state.status, PlaybackStateStatus::Disconnected);
    assert!(system.lock().expect("system state").controls.is_empty());
}

#[test]
fn persisted_unknown_default_source_is_rejected_as_storage_corruption() {
    let (harness, _system) = system_harness();
    let error = harness
        .service
        .apply_initial_source(Some("untrusted-player"))
        .expect_err("unknown persisted source must fail closed");
    assert_eq!(error.error_id, ErrorId::StorageFailed);
}

#[tokio::test]
async fn system_media_source_rechecks_capability_before_control_and_refreshes_failure_state() {
    let (harness, system) = system_harness();
    let selected = harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected")
        .state;
    system
        .lock()
        .expect("system state")
        .snapshot
        .capabilities
        .next = false;
    let error = harness
        .service
        .next(control(selected.revision))
        .await
        .expect_err("fresh capability blocks control");
    assert_eq!(error.error_id, ErrorId::CapabilityUnsupported);
    let state = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("refreshed state");
    assert!(!state.capabilities.next);
    assert!(system.lock().expect("system state").controls.is_empty());
}

#[tokio::test]
async fn system_media_source_rejects_replaced_session_and_disconnects_without_fighting_user() {
    let (harness, system) = system_harness();
    let selected = harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected")
        .state;
    system.lock().expect("system state").snapshot.identity = "bound-session-b".to_owned();
    let error = harness
        .service
        .play(control(selected.revision))
        .await
        .expect_err("stale session control rejected");
    assert_eq!(error.error_id, ErrorId::MediaSessionChanged);
    assert!(system.lock().expect("system state").controls.is_empty());

    let sink = {
        let mut state = system.lock().expect("system state");
        state.available = false;
        state.sink.clone().expect("event sink")
    };
    sink.changed();
    let disconnected = harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("disconnected snapshot");
    assert_eq!(disconnected.status, PlaybackStateStatus::Disconnected);
    assert!(disconnected.current_track.is_none());
    assert!(!disconnected.capabilities.play);
}

#[tokio::test]
async fn tts_interruption_resumes_only_the_unchanged_paused_system_session() {
    let (harness, system) = system_harness();
    let selected = harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected")
        .state;
    assert_eq!(selected.status, PlaybackStateStatus::Paused);
    system.lock().expect("system state").snapshot.status = PlaybackStateStatus::Playing;
    harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("playing refresh");
    let token = harness
        .service
        .begin_system_interruption()
        .await
        .expect("safe pause token");
    assert_eq!(
        system.lock().expect("system state").controls,
        vec![SystemMediaControl::Pause]
    );
    let resumed = harness
        .service
        .finish_system_interruption(token)
        .await
        .expect("safe resume");
    assert_eq!(resumed.status, PlaybackStateStatus::Playing);
    assert_eq!(
        system.lock().expect("system state").controls,
        vec![SystemMediaControl::Pause, SystemMediaControl::Play]
    );
}

#[tokio::test]
async fn tts_interruption_user_override_or_session_replacement_aborts_resume() {
    let (harness, system) = system_harness();
    harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected");
    system.lock().expect("system state").snapshot.status = PlaybackStateStatus::Playing;
    harness
        .service
        .get_playback_state(EmptyRequest {})
        .await
        .expect("playing refresh");
    let token = harness
        .service
        .begin_system_interruption()
        .await
        .expect("safe pause token");
    system.lock().expect("system state").snapshot.status = PlaybackStateStatus::Playing;
    let overridden = harness
        .service
        .finish_system_interruption(token)
        .await
        .expect("override leaves external state untouched");
    assert_eq!(overridden.status, PlaybackStateStatus::Playing);
    assert_eq!(
        system.lock().expect("system state").controls,
        vec![SystemMediaControl::Pause]
    );

    let token = harness
        .service
        .begin_system_interruption()
        .await
        .expect("second safe pause token");
    system.lock().expect("system state").snapshot.identity = "replacement-session".to_owned();
    let replaced = harness
        .service
        .finish_system_interruption(token)
        .await
        .expect("replacement leaves new session untouched");
    assert_eq!(replaced.status, PlaybackStateStatus::Paused);
    assert_eq!(
        system.lock().expect("system state").controls,
        vec![SystemMediaControl::Pause, SystemMediaControl::Pause]
    );
}

#[tokio::test]
async fn tts_interruption_does_not_control_an_already_paused_session() {
    let (harness, system) = system_harness();
    harness
        .service
        .select_music_source(SelectMusicSourceRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
        })
        .await
        .expect("system source selected");
    let token = harness
        .service
        .begin_system_interruption()
        .await
        .expect("non-resuming token");
    harness
        .service
        .finish_system_interruption(token)
        .await
        .expect("no-op finish");
    assert!(system.lock().expect("system state").controls.is_empty());
}
