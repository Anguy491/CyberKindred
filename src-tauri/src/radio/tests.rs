use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, TimeZone, Utc};
use tokio::sync::{Notify, mpsc, watch};
use uuid::Uuid;

use crate::{
    contracts::{
        PlaybackState, PlaybackStateCapabilities, PlaybackStateSourceKind, PlaybackStateStatus,
        PlaybackStateTrack, PlaybackStateTrackOrigin, ProgramPlan, ProgramPlanMode,
        ProgramPlanSegmentsItem, ProgramPlanTrackSegment, ProgramPlanVoiceSegment,
        ProgramPlanVoiceSegmentTrigger,
    },
    ipc::{ApiError, ErrorId, InternalReason, ProcessSequence},
    program::{CandidateSelection, PlanDegradation, PlanOrigin, PlannedProgram, ProgramCandidate},
};

use super::service::COMPANION_RECONNECTED_MESSAGE;
use super::*;

#[derive(Clone)]
struct FixedClock;

impl RadioClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_800_000_000_000)
            .single()
            .expect("fixed test time")
    }
}

struct FixedId(Uuid);

impl RadioIdFactory for FixedId {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

struct FakePlanner {
    calls: AtomicUsize,
    fail: AtomicBool,
}

impl FakePlanner {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        }
    }
}

impl ProgramRadioPlanner for FakePlanner {
    fn plan_local(
        &self,
        program_id: Uuid,
    ) -> traits::RadioFuture<'_, Result<PlannedProgram, ApiError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fail = self.fail.load(Ordering::SeqCst);
        Box::pin(async move {
            if fail {
                Err(ApiError::from_reason(InternalReason::SourceUnavailable))
            } else {
                Ok(six_track_plan(program_id))
            }
        })
    }
}

#[derive(Default)]
struct StoreState {
    begin_calls: usize,
    persist_calls: usize,
    phase: Option<ProgramRunPhase>,
    revision: u64,
    segments: HashMap<Uuid, ProgramSegmentPhase>,
    transitions: Vec<ProgramRunPhase>,
}

struct FakeStore {
    state: Mutex<StoreState>,
    fail_begin: AtomicBool,
    fail_persist: AtomicBool,
    voice_allowed: AtomicBool,
}

impl FakeStore {
    fn new() -> Self {
        Self {
            state: Mutex::new(StoreState::default()),
            fail_begin: AtomicBool::new(false),
            fail_persist: AtomicBool::new(false),
            voice_allowed: AtomicBool::new(true),
        }
    }

    fn phase(&self) -> Option<ProgramRunPhase> {
        self.state.lock().expect("test store").phase
    }
}

impl RadioProgramStore for FakeStore {
    fn begin_program(
        &self,
        _program_id: Uuid,
        _created_at_ms: i64,
    ) -> traits::RadioFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| ApiError::unexpected())?;
            state.begin_calls += 1;
            if self.fail_begin.load(Ordering::SeqCst) {
                return Err(ApiError::from_reason(InternalReason::StorageWriteFailed));
            }
            state.phase = Some(ProgramRunPhase::Planning);
            Ok(0)
        })
    }

    fn begin_system_program(
        &self,
        program_id: Uuid,
        created_at_ms: i64,
    ) -> traits::RadioFuture<'_, Result<u64, ApiError>> {
        self.begin_program(program_id, created_at_ms)
    }

    fn persist_plan<'a>(
        &'a self,
        planned: &'a PlannedProgram,
    ) -> traits::RadioFuture<'a, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| ApiError::unexpected())?;
            state.persist_calls += 1;
            if self.fail_persist.load(Ordering::SeqCst) {
                return Err(ApiError::from_reason(InternalReason::StorageWriteFailed));
            }
            if state.phase != Some(ProgramRunPhase::Planning) {
                return Err(ApiError::from_reason(InternalReason::RevisionConflict));
            }
            state.phase = Some(ProgramRunPhase::Ready);
            state.revision = 1;
            state.segments = planned
                .plan
                .segments
                .iter()
                .map(|segment| {
                    let value = match segment {
                        ProgramPlanSegmentsItem::TrackSegment(track) => &track.segment_id,
                        ProgramPlanSegmentsItem::VoiceSegment(voice) => &voice.segment_id,
                    };
                    (
                        Uuid::parse_str(value).expect("valid test segment"),
                        ProgramSegmentPhase::Planned,
                    )
                })
                .collect();
            Ok(1)
        })
    }

    fn transition_program(
        &self,
        _program_id: Uuid,
        expected: ProgramRunPhase,
        next: ProgramRunPhase,
        current_revision: u64,
        _occurred_at_ms: i64,
        _failure_code: Option<&'static str>,
    ) -> traits::RadioFuture<'_, Result<u64, ApiError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| ApiError::unexpected())?;
            if state.phase != Some(expected) || state.revision != current_revision {
                return Err(ApiError::from_reason(InternalReason::RevisionConflict));
            }
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or_else(ApiError::unexpected)?;
            state.phase = Some(next);
            state.transitions.push(next);
            Ok(state.revision)
        })
    }

    fn transition_segment(
        &self,
        _program_id: Uuid,
        segment_id: Uuid,
        expected: ProgramSegmentPhase,
        next: ProgramSegmentPhase,
        _occurred_at_ms: i64,
        _failure_code: Option<&'static str>,
    ) -> traits::RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| ApiError::unexpected())?;
            if state.segments.get(&segment_id) != Some(&expected) {
                return Err(ApiError::from_reason(InternalReason::RevisionConflict));
            }
            state.segments.insert(segment_id, next);
            Ok(())
        })
    }

    fn voice_allowed_after_feedback(
        &self,
        _program_id: Uuid,
    ) -> traits::RadioFuture<'_, Result<bool, ApiError>> {
        Box::pin(async { Ok(self.voice_allowed.load(Ordering::SeqCst)) })
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<RadioEvent>>);

impl RadioEventSink for RecordingSink {
    fn publish(&self, event: RadioEvent) -> Result<(), ApiError> {
        self.0
            .lock()
            .map_err(|_| ApiError::unexpected())?
            .push(event);
        Ok(())
    }
}

struct FakeSpeech {
    calls: AtomicUsize,
    block: AtomicBool,
    cancelled: AtomicBool,
    started: Notify,
    actions: Arc<Mutex<Vec<String>>>,
}

impl ProgramSpeech for FakeSpeech {
    fn present<'a>(
        &'a self,
        _program_id: Uuid,
        _segment_id: Uuid,
        text: &'a str,
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
    ) -> traits::RadioFuture<'a, Result<ProgramSpeechOutcome, ApiError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let action = format!("voice:{text}");
        Box::pin(async move {
            if *cancellation.borrow() {
                return Err(ApiError::from_reason(InternalReason::OperationCancelled));
            }
            self.actions
                .lock()
                .map_err(|_| ApiError::unexpected())?
                .push(action);
            self.started.notify_waiters();
            if self.block.load(Ordering::SeqCst) {
                loop {
                    cancellation
                        .changed()
                        .await
                        .map_err(|_| ApiError::unexpected())?;
                    if *cancellation.borrow() {
                        self.cancelled.store(true, Ordering::SeqCst);
                        return Err(ApiError::from_reason(InternalReason::OperationCancelled));
                    }
                }
            }
            Ok(ProgramSpeechOutcome::TextOnly)
        })
    }

    fn cancel(&self, _segment_id: Uuid) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

struct FakePlayback {
    calls: AtomicUsize,
    stop_calls: AtomicUsize,
    block: AtomicBool,
    started: Notify,
    actions: Arc<Mutex<Vec<String>>>,
}

struct FakeApple;

impl AppleCompanion for FakeApple {
    fn connect(&self) -> traits::RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async { Ok(system_playback_state()) })
    }

    fn run(
        &self,
        _program_id: Uuid,
        _initial: PlaybackState,
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> traits::RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            signals
                .send(AppleCompanionSignal::Reaction(
                    "deterministic apple reaction".to_owned(),
                ))
                .await
                .map_err(|_| ApiError::unexpected())?;
            loop {
                cancellation
                    .changed()
                    .await
                    .map_err(|_| ApiError::unexpected())?;
                if *cancellation.borrow() {
                    return Ok(());
                }
            }
        })
    }
}

struct DisconnectingApple;

impl AppleCompanion for DisconnectingApple {
    fn connect(&self) -> traits::RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async { Ok(system_playback_state()) })
    }

    fn run(
        &self,
        _program_id: Uuid,
        _initial: PlaybackState,
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> traits::RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            signals
                .send(AppleCompanionSignal::Disconnected)
                .await
                .map_err(|_| ApiError::unexpected())?;
            loop {
                cancellation
                    .changed()
                    .await
                    .map_err(|_| ApiError::unexpected())?;
                if *cancellation.borrow() {
                    return Ok(());
                }
            }
        })
    }
}

struct ReconnectingApple(Arc<Notify>);

impl AppleCompanion for ReconnectingApple {
    fn connect(&self) -> traits::RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async { Ok(system_playback_state()) })
    }

    fn run(
        &self,
        _program_id: Uuid,
        _initial: PlaybackState,
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> traits::RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            signals
                .send(AppleCompanionSignal::Disconnected)
                .await
                .map_err(|_| ApiError::unexpected())?;
            self.0.notified().await;
            signals
                .send(AppleCompanionSignal::Reconnected)
                .await
                .map_err(|_| ApiError::unexpected())?;
            loop {
                cancellation
                    .changed()
                    .await
                    .map_err(|_| ApiError::unexpected())?;
                if *cancellation.borrow() {
                    return Ok(());
                }
            }
        })
    }
}

impl FakePlayback {
    fn new(actions: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            stop_calls: AtomicUsize::new(0),
            block: AtomicBool::new(false),
            started: Notify::new(),
            actions,
        }
    }
}

impl ProgramPlayback for FakePlayback {
    fn play_batch<'a>(
        &'a self,
        _program_id: Uuid,
        tracks: &'a [ProgramTrack],
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<ProgramPlaybackSignal>,
    ) -> traits::RadioFuture<'a, Result<(), ApiError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let ids = tracks
            .iter()
            .map(|track| track.track_id.clone())
            .collect::<Vec<_>>();
        let segments = tracks
            .iter()
            .map(|track| track.segment_id)
            .collect::<Vec<_>>();
        Box::pin(async move {
            self.actions
                .lock()
                .map_err(|_| ApiError::unexpected())?
                .push(format!("tracks:{}", ids.join(",")));
            self.started.notify_waiters();
            if self.block.load(Ordering::SeqCst) {
                loop {
                    cancellation
                        .changed()
                        .await
                        .map_err(|_| ApiError::unexpected())?;
                    if *cancellation.borrow() {
                        return Err(ApiError::from_reason(InternalReason::OperationCancelled));
                    }
                }
            }
            for segment in segments {
                signals
                    .send(ProgramPlaybackSignal::Started(segment))
                    .await
                    .map_err(|_| ApiError::unexpected())?;
                signals
                    .send(ProgramPlaybackSignal::Completed(segment))
                    .await
                    .map_err(|_| ApiError::unexpected())?;
            }
            Ok(())
        })
    }

    fn stop(&self) -> traits::RadioFuture<'_, Result<(), ApiError>> {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    service: RadioService,
    planner: Arc<FakePlanner>,
    store: Arc<FakeStore>,
    playback: Arc<FakePlayback>,
    speech: Arc<FakeSpeech>,
    sink: Arc<RecordingSink>,
    actions: Arc<Mutex<Vec<String>>>,
    program_id: Uuid,
}

fn fixture() -> Fixture {
    fixture_with_apple(Arc::new(FakeApple))
}

fn fixture_with_apple(apple: Arc<dyn AppleCompanion>) -> Fixture {
    let actions = Arc::new(Mutex::new(Vec::new()));
    let planner = Arc::new(FakePlanner::new());
    let store = Arc::new(FakeStore::new());
    let playback = Arc::new(FakePlayback::new(Arc::clone(&actions)));
    let speech = Arc::new(FakeSpeech {
        calls: AtomicUsize::new(0),
        block: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        started: Notify::new(),
        actions: Arc::clone(&actions),
    });
    let sink = Arc::new(RecordingSink::default());
    let program_id = Uuid::now_v7();
    let service = RadioService::new(RadioServiceDependencies {
        planner: planner.clone(),
        authorizer: Arc::new(ManualProgramStartAuthorizer),
        store: store.clone(),
        playback: playback.clone(),
        speech: speech.clone(),
        apple,
        event_sink: sink.clone(),
        clock: Arc::new(FixedClock),
        sequence: Arc::new(ProcessSequence::default()),
        id_factory: Arc::new(FixedId(program_id)),
    });
    Fixture {
        service,
        planner,
        store,
        playback,
        speech,
        sink,
        actions,
        program_id,
    }
}

fn system_playback_state() -> PlaybackState {
    PlaybackState {
        schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
        source_id: "apple_music".to_owned(),
        source_kind: PlaybackStateSourceKind::SystemSession,
        status: PlaybackStateStatus::Playing,
        capabilities: PlaybackStateCapabilities {
            play: true,
            pause: true,
            seek: false,
            next: true,
            previous: true,
            set_queue: false,
        },
        current_track: Some(PlaybackStateTrack {
            track_id: "system:0123456789abcdef0123456789abcdef".to_owned(),
            title: "GSMTC CANARY".to_owned(),
            artist: None,
            album: None,
            artwork_uri: None,
            origin: PlaybackStateTrackOrigin::SystemSession,
        }),
        position_ms: 0,
        duration_ms: Some(10_000),
        revision: 1,
        updated_at: "2026-09-03T01:00:00.000Z".to_owned(),
        last_error: None,
    }
}

fn start_request() -> StartProgramRequest {
    StartProgramRequest {
        client_request_id: Uuid::now_v7(),
        source_id: "local".to_owned(),
        trigger: StartProgramTrigger::Manual,
    }
}

async fn wait_for_phase(store: &FakeStore, expected: ProgramRunPhase) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while store.phase() != Some(expected) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("runner reached expected phase");
}

#[tokio::test]
async fn apple_program_returns_no_queue_plan_and_emits_only_local_companion_text() {
    let fixture = fixture();
    let response = fixture
        .service
        .start_local_program(StartProgramRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
            trigger: StartProgramTrigger::Manual,
        })
        .await
        .expect("Apple companion start");
    assert!(response.plan.is_none());
    assert_eq!(fixture.planner.calls.load(Ordering::SeqCst), 0);
    wait_for_phase(&fixture.store, ProgramRunPhase::Music).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let has_reaction = fixture.sink.0.lock().expect("events").iter().any(|event| {
                matches!(event, RadioEvent::ProgramState(value)
                    if value.safe_message.as_deref() == Some("deterministic apple reaction"))
            });
            if has_reaction {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("local deterministic reaction");
    let stopped = fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: fixture.program_id,
        })
        .await
        .expect("safe stop");
    assert!(stopped.revision >= 4);
    assert_eq!(fixture.playback.stop_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn apple_program_persists_session_loss_and_can_stop_while_paused() {
    let fixture = fixture_with_apple(Arc::new(DisconnectingApple));
    fixture
        .service
        .start_local_program(StartProgramRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
            trigger: StartProgramTrigger::Manual,
        })
        .await
        .expect("Apple companion start");
    wait_for_phase(&fixture.store, ProgramRunPhase::Paused).await;

    fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: fixture.program_id,
        })
        .await
        .expect("paused companion stop");

    assert_eq!(fixture.store.phase(), Some(ProgramRunPhase::Completed));
    let transitions = fixture
        .store
        .state
        .lock()
        .expect("store")
        .transitions
        .clone();
    assert_eq!(
        transitions.get(transitions.len().saturating_sub(3)..),
        Some(
            &[
                ProgramRunPhase::Paused,
                ProgramRunPhase::Stopping,
                ProgramRunPhase::Completed,
            ][..]
        )
    );
}

#[tokio::test]
async fn apple_program_persists_reconnection_before_returning_to_running() {
    let reconnect = Arc::new(Notify::new());
    let fixture = fixture_with_apple(Arc::new(ReconnectingApple(Arc::clone(&reconnect))));
    fixture
        .service
        .start_local_program(StartProgramRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "apple_music".to_owned(),
            trigger: StartProgramTrigger::Manual,
        })
        .await
        .expect("Apple companion start");
    wait_for_phase(&fixture.store, ProgramRunPhase::Paused).await;
    reconnect.notify_one();
    wait_for_phase(&fixture.store, ProgramRunPhase::Music).await;

    let states = fixture
        .sink
        .0
        .lock()
        .expect("events")
        .iter()
        .filter_map(|event| match event {
            RadioEvent::ProgramState(event) => Some((event.state, event.safe_message.clone())),
            RadioEvent::ProgramSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(
        states
            .iter()
            .any(|(state, _)| *state == ProgramEventState::Paused)
    );
    assert!(states.iter().any(|(state, message)| {
        *state == ProgramEventState::Running
            && message.as_deref() == Some(COMPANION_RECONNECTED_MESSAGE)
    }));

    fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: fixture.program_id,
        })
        .await
        .expect("reconnected companion stop");
}

#[tokio::test]
async fn program_runner_executes_six_tracks_in_bounded_plan_order_and_text_only_continues() {
    let fixture = fixture();
    let response = fixture
        .service
        .start_local_program(start_request())
        .await
        .expect("confirmed start");
    assert_eq!(response.program_id, fixture.program_id);
    let plan = response.plan.as_ref().expect("validated plan returned");
    wait_for_phase(&fixture.store, ProgramRunPhase::Completed).await;

    let actions = fixture.actions.lock().expect("actions").clone();
    assert_eq!(actions.len(), 6);
    assert_eq!(actions[0], "voice:opening");
    assert!(actions[1].starts_with("tracks:"));
    assert_eq!(actions[2], "voice:bridge-one");
    assert!(actions[3].starts_with("tracks:"));
    assert_eq!(actions[4], "voice:bridge-two");
    assert!(actions[5].starts_with("tracks:"));
    assert_eq!(
        actions
            .iter()
            .filter(|value| value.starts_with("tracks:"))
            .count(),
        3
    );
    for batch in actions.iter().filter(|value| value.starts_with("tracks:")) {
        assert_eq!(batch.split(',').count(), 2);
    }
    let played_track_ids = actions
        .iter()
        .filter_map(|action| action.strip_prefix("tracks:"))
        .flat_map(|batch| batch.split(','))
        .collect::<Vec<_>>();
    let planned_track_ids = plan
        .segments
        .iter()
        .filter_map(|segment| match segment {
            ProgramPlanSegmentsItem::TrackSegment(track) => Some(track.track_id.as_str()),
            ProgramPlanSegmentsItem::VoiceSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(played_track_ids, planned_track_ids);
    assert_eq!(fixture.speech.calls.load(Ordering::SeqCst), 3);
    assert_eq!(fixture.playback.calls.load(Ordering::SeqCst), 3);

    let events = fixture.sink.0.lock().expect("events");
    let json = events.iter().map(event_json).collect::<Vec<_>>().join("\n");
    assert!(!json.contains("path"));
    assert!(!json.contains("secret"));
    let sequences = events.iter().map(event_sequence).collect::<Vec<_>>();
    assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        matches!(events.first(), Some(RadioEvent::ProgramState(event)) if event.state == ProgramEventState::Planning)
    );
    assert!(
        matches!(events.last(), Some(RadioEvent::ProgramState(event)) if event.state == ProgramEventState::Completed)
    );
    let messages = events
        .iter()
        .filter_map(|event| match event {
            RadioEvent::ProgramState(event) => event.safe_message.as_deref(),
            RadioEvent::ProgramSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.contains("确定性本地队列"))
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.contains("语音不可用"))
            .count(),
        1
    );
}

#[tokio::test]
async fn program_runner_applies_less_talk_before_the_next_voice_segment() {
    let fixture = fixture();
    fixture.store.voice_allowed.store(false, Ordering::SeqCst);
    fixture
        .service
        .start_local_program(start_request())
        .await
        .expect("confirmed start");
    wait_for_phase(&fixture.store, ProgramRunPhase::Completed).await;

    assert_eq!(fixture.speech.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.playback.calls.load(Ordering::SeqCst), 3);
    assert!(
        fixture
            .sink
            .0
            .lock()
            .expect("events")
            .iter()
            .filter(|event| matches!(
                event,
                RadioEvent::ProgramSegment(segment)
                    if segment.state == ProgramSegmentEventState::Skipped
            ))
            .count()
            >= 3
    );
}

#[tokio::test]
async fn program_runner_stop_cancels_blocking_playback_and_persists_terminal() {
    let fixture = fixture();
    fixture.playback.block.store(true, Ordering::SeqCst);
    let response = fixture
        .service
        .start_local_program(start_request())
        .await
        .expect("confirmed start");
    tokio::time::timeout(Duration::from_secs(2), fixture.playback.started.notified())
        .await
        .expect("playback began");
    let ack = fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: response.program_id,
        })
        .await
        .expect("stop completes");
    assert!(ack.revision > 1);
    assert_eq!(fixture.store.phase(), Some(ProgramRunPhase::Completed));
    assert!(fixture.playback.stop_calls.load(Ordering::SeqCst) >= 1);
    let transitions = fixture
        .store
        .state
        .lock()
        .expect("store")
        .transitions
        .clone();
    assert!(transitions.contains(&ProgramRunPhase::Stopping));
    assert_eq!(transitions.last(), Some(&ProgramRunPhase::Completed));
    let coarse_states = fixture
        .sink
        .0
        .lock()
        .expect("events")
        .iter()
        .filter_map(|event| match event {
            RadioEvent::ProgramState(event) => Some(event.state),
            RadioEvent::ProgramSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        coarse_states.get(coarse_states.len().saturating_sub(2)..),
        Some(&[ProgramEventState::Stopping, ProgramEventState::Completed][..])
    );
    let second = fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: response.program_id,
        })
        .await
        .expect("terminal stop is idempotent");
    assert_eq!(second.revision, ack.revision);
}

#[tokio::test]
async fn recovery_suspend_interrupts_blocking_program_without_marking_it_completed() {
    let fixture = fixture();
    fixture.playback.block.store(true, Ordering::SeqCst);
    fixture
        .service
        .start_local_program(start_request())
        .await
        .expect("confirmed start");
    tokio::time::timeout(Duration::from_secs(2), fixture.playback.started.notified())
        .await
        .expect("playback began");

    fixture
        .service
        .prepare_suspend()
        .await
        .expect("program interrupted before suspend");

    assert_eq!(fixture.store.phase(), Some(ProgramRunPhase::Interrupted));
    assert!(fixture.playback.stop_calls.load(Ordering::SeqCst) >= 1);
    assert_eq!(fixture.service.active_program_id().await, None);
    assert_eq!(
        fixture
            .store
            .state
            .lock()
            .expect("store")
            .transitions
            .last(),
        Some(&ProgramRunPhase::Interrupted)
    );
    let blocked = fixture
        .service
        .start_local_program(start_request())
        .await
        .expect_err("suspended service rejects fresh sound authorization");
    assert_eq!(blocked.error_id, ErrorId::ResourceBusy);
    fixture.service.resume_after_suspend();
}

#[tokio::test]
async fn program_runner_allows_only_one_active_run() {
    let fixture = fixture();
    fixture.playback.block.store(true, Ordering::SeqCst);
    let first = fixture
        .service
        .start_local_program(start_request())
        .await
        .expect("first start");
    tokio::time::timeout(Duration::from_secs(2), fixture.playback.started.notified())
        .await
        .expect("first playback began");
    let busy = fixture
        .service
        .start_local_program(start_request())
        .await
        .expect_err("second active run rejected");
    assert_eq!(busy.error_id, ErrorId::ResourceBusy);
    assert_eq!(fixture.planner.calls.load(Ordering::SeqCst), 1);
    fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: first.program_id,
        })
        .await
        .expect("cleanup first run");
}

#[tokio::test]
async fn program_runner_stop_cancels_blocking_speech_before_any_track() {
    let fixture = fixture();
    fixture.speech.block.store(true, Ordering::SeqCst);
    let response = fixture
        .service
        .start_local_program(start_request())
        .await
        .expect("confirmed start");
    tokio::time::timeout(Duration::from_secs(2), fixture.speech.started.notified())
        .await
        .expect("speech began");
    fixture
        .service
        .stop_program(StopProgramRequest {
            client_request_id: Uuid::now_v7(),
            program_id: response.program_id,
        })
        .await
        .expect("stop completes");
    assert!(fixture.speech.cancelled.load(Ordering::SeqCst));
    assert_eq!(fixture.playback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.store.phase(), Some(ProgramRunPhase::Completed));
}

#[tokio::test]
async fn program_runner_duplicate_start_returns_original_and_payload_conflict() {
    let fixture = fixture();
    let request = start_request();
    let first = fixture
        .service
        .start_local_program(request.clone())
        .await
        .expect("first");
    let retry = fixture
        .service
        .start_local_program(request.clone())
        .await
        .expect("retry");
    assert_eq!(retry, first);
    assert_eq!(fixture.planner.calls.load(Ordering::SeqCst), 1);

    let conflict = fixture
        .service
        .start_local_program(StartProgramRequest {
            trigger: StartProgramTrigger::Notification,
            ..request
        })
        .await
        .expect_err("changed payload");
    assert_eq!(conflict.error_id, ErrorId::Conflict);
}

#[tokio::test]
async fn program_runner_construction_without_user_start_has_zero_side_effect_calls() {
    let fixture = fixture();
    tokio::task::yield_now().await;
    assert_eq!(fixture.planner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.playback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.speech.calls.load(Ordering::SeqCst), 0);
    let state = fixture.store.state.lock().expect("store");
    assert_eq!(state.begin_calls, 0);
    assert_eq!(state.persist_calls, 0);
}

#[tokio::test]
async fn program_runner_unconfirmed_notification_has_zero_side_effect_calls() {
    let fixture = fixture();
    let error = fixture
        .service
        .start_local_program(StartProgramRequest {
            client_request_id: Uuid::now_v7(),
            source_id: "local".to_owned(),
            trigger: StartProgramTrigger::Notification,
        })
        .await
        .expect_err("bare notification has no proof");
    assert_eq!(error.error_id, ErrorId::RequestInvalid);
    assert_eq!(fixture.planner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.playback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.speech.calls.load(Ordering::SeqCst), 0);
    let state = fixture.store.state.lock().expect("store");
    assert_eq!(state.begin_calls, 0);
    assert_eq!(state.persist_calls, 0);
}

#[tokio::test]
async fn program_runner_planning_failure_is_persisted_and_emitted_before_audio() {
    let fixture = fixture();
    fixture.planner.fail.store(true, Ordering::SeqCst);
    fixture
        .service
        .start_local_program(start_request())
        .await
        .expect_err("planning failure");
    assert_eq!(fixture.store.phase(), Some(ProgramRunPhase::Failed));
    assert_eq!(fixture.playback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.speech.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fixture.sink.0.lock().expect("events").last(),
        Some(RadioEvent::ProgramState(event))
            if event.state == ProgramEventState::Failed
                && event.safe_message.as_deref() == Some("节目计划生成失败，未开始播放。")
    ));
}

#[tokio::test]
async fn program_runner_bad_identity_or_plan_storage_never_reaches_sound() {
    let begin_failure = fixture();
    begin_failure.store.fail_begin.store(true, Ordering::SeqCst);
    begin_failure
        .service
        .start_local_program(start_request())
        .await
        .expect_err("begin failure");
    assert_eq!(begin_failure.planner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(begin_failure.playback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(begin_failure.speech.calls.load(Ordering::SeqCst), 0);

    let plan_failure = fixture();
    plan_failure
        .store
        .fail_persist
        .store(true, Ordering::SeqCst);
    plan_failure
        .service
        .start_local_program(start_request())
        .await
        .expect_err("persist failure");
    assert_eq!(plan_failure.planner.calls.load(Ordering::SeqCst), 1);
    assert_eq!(plan_failure.playback.calls.load(Ordering::SeqCst), 0);
    assert_eq!(plan_failure.speech.calls.load(Ordering::SeqCst), 0);
    assert_eq!(plan_failure.store.phase(), Some(ProgramRunPhase::Failed));
}

#[test]
fn program_runner_public_dtos_reject_unknown_fields() {
    let request_id = Uuid::now_v7();
    let invalid = serde_json::json!({
        "clientRequestId": request_id,
        "sourceId": "local",
        "trigger": "manual",
        "path": "C:\\secret",
    });
    assert!(serde_json::from_value::<StartProgramRequest>(invalid).is_err());

    let invalid_event = serde_json::json!({
        "schemaVersion": "1.0.0",
        "sequence": 1,
        "occurredAt": "2027-01-15T08:00:00.000Z",
        "programId": Uuid::now_v7(),
        "state": "failed",
        "safeMessage": null,
        "secret": "canary",
    });
    assert!(serde_json::from_value::<ProgramStateEvent>(invalid_event).is_err());
}

fn event_sequence(event: &RadioEvent) -> u64 {
    match event {
        RadioEvent::ProgramState(event) => event.envelope.sequence,
        RadioEvent::ProgramSegment(event) => event.envelope.sequence,
    }
}

fn event_json(event: &RadioEvent) -> String {
    match event {
        RadioEvent::ProgramState(event) => serde_json::to_string(event),
        RadioEvent::ProgramSegment(event) => serde_json::to_string(event),
    }
    .expect("serialize public event")
}

fn six_track_plan(program_id: Uuid) -> PlannedProgram {
    let tracks = (0..6).map(|_| Uuid::now_v7()).collect::<Vec<_>>();
    let opening = voice_segment("opening", ProgramPlanVoiceSegmentTrigger::Opening);
    let bridge_one = voice_segment("bridge-one", ProgramPlanVoiceSegmentTrigger::BetweenTracks);
    let bridge_two = voice_segment("bridge-two", ProgramPlanVoiceSegmentTrigger::BetweenTracks);
    let segments = vec![
        opening,
        track_segment(tracks[0]),
        track_segment(tracks[1]),
        bridge_one,
        track_segment(tracks[2]),
        track_segment(tracks[3]),
        bridge_two,
        track_segment(tracks[4]),
        track_segment(tracks[5]),
    ];
    let candidates = tracks
        .iter()
        .map(|track_id| ProgramCandidate {
            track_id: track_id.to_string(),
            title: None,
            artist: None,
            album: None,
            duration_ms: 180_000,
            normalized_tags: Vec::new(),
            recent_play_penalty: 0,
        })
        .collect();
    PlannedProgram {
        plan: ProgramPlan {
            schema_version: "1.0.0".to_owned(),
            program_id: program_id.to_string(),
            source_id: "local".to_owned(),
            mode: ProgramPlanMode::Local,
            created_at: "2027-01-15T08:00:00.000Z".to_owned(),
            segments,
        },
        candidates: CandidateSelection {
            candidates,
            cooldown_relaxed: false,
        },
        origin: PlanOrigin::Deterministic,
        degradation: Some(PlanDegradation::ProviderUnavailable),
    }
}

fn voice_segment(text: &str, trigger: ProgramPlanVoiceSegmentTrigger) -> ProgramPlanSegmentsItem {
    ProgramPlanSegmentsItem::VoiceSegment(ProgramPlanVoiceSegment {
        r#type: "voice".to_owned(),
        segment_id: Uuid::now_v7().to_string(),
        text: text.to_owned(),
        trigger,
    })
}

fn track_segment(track_id: Uuid) -> ProgramPlanSegmentsItem {
    ProgramPlanSegmentsItem::TrackSegment(ProgramPlanTrackSegment {
        r#type: "track".to_owned(),
        segment_id: Uuid::now_v7().to_string(),
        track_id: track_id.to_string(),
        segue_text: None,
    })
}
