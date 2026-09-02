use std::{collections::HashSet, sync::Arc};

use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{
    contracts::{
        PlaybackEvent, PlaybackEventReason, PlaybackEventType, PlaybackState,
        PlaybackStateCapabilities, PlaybackStateSafeError, PlaybackStateSourceKind,
        PlaybackStateStatus, PlaybackStateTrack, PlaybackStateTrackOrigin,
    },
    ipc::{ApiError, CapabilityName, InternalReason, ProcessSequence, SourceCapabilities},
};

use super::{
    engine::{
        AudioEngineError, AudioSnapshot, LocalAudioEngine, LocalAudioEngineEvent,
        LocalAudioEngineEventSink, LocalSourceError, LocalTrack, LocalTrackResolver,
        PlaybackStartAuthorization,
    },
    events::SharedPlaybackEventSink,
    service::PlaybackClock,
};

pub(super) const ACTOR_CHANNEL_CAPACITY: usize = 64;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_POSITION_MS: u64 = 86_400_000;
const MAX_QUEUE_TRACKS: usize = 200;

pub(super) enum ActorMessage {
    SelectSource {
        source_id: String,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    GetState {
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Play {
        expected_revision: u64,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Pause {
        expected_revision: u64,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Seek {
        expected_revision: u64,
        position_ms: u64,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Next {
        expected_revision: u64,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Previous {
        expected_revision: u64,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    SetQueue {
        track_ids: Vec<String>,
        authorization: Option<PlaybackStartAuthorization>,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    EngineEvent(LocalAudioEngineEvent),
    Suspend {
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    ResumeSilent {
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Stop {
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Shutdown {
        response: oneshot::Sender<()>,
    },
}

pub(super) fn spawn_actor(
    source_id: String,
    capabilities: SourceCapabilities,
    resolver: Arc<dyn LocalTrackResolver>,
    mut engine: Box<dyn LocalAudioEngine>,
    events: SharedPlaybackEventSink,
    clock: Arc<dyn PlaybackClock>,
    sequence: Arc<ProcessSequence>,
) -> Result<mpsc::Sender<ActorMessage>, ApiError> {
    let (sender, receiver) = mpsc::channel(ACTOR_CHANNEL_CAPACITY);
    engine.set_event_sink(Arc::new(ActorEngineEventSink {
        sender: sender.clone(),
    }));
    let actor = PlaybackActor::new(
        source_id,
        capabilities,
        resolver,
        engine,
        events,
        clock,
        sequence,
    );
    std::thread::Builder::new()
        .name("cyberkindred-playback".to_owned())
        .spawn(move || actor.run(receiver))
        .map_err(|_| ApiError::unexpected())?;
    Ok(sender)
}

struct ActorEngineEventSink {
    sender: mpsc::Sender<ActorMessage>,
}

impl LocalAudioEngineEventSink for ActorEngineEventSink {
    fn publish(&self, event: LocalAudioEngineEvent) {
        let _ = self.sender.blocking_send(ActorMessage::EngineEvent(event));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActorMode {
    Idle,
    Loading,
    Playing,
    Paused,
    RecoveringOutput,
    Error,
    Stopped,
}

struct PlaybackActor {
    source_id: String,
    capabilities: SourceCapabilities,
    resolver: Arc<dyn LocalTrackResolver>,
    engine: Box<dyn LocalAudioEngine>,
    events: SharedPlaybackEventSink,
    clock: Arc<dyn PlaybackClock>,
    sequence: Arc<ProcessSequence>,
    state: PlaybackState,
    mode: ActorMode,
    queue: Vec<LocalTrack>,
    current_index: Option<usize>,
    engine_session_id: Option<Uuid>,
    recovery_session_id: Option<Uuid>,
    resume_pending: bool,
}

impl PlaybackActor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source_id: String,
        capabilities: SourceCapabilities,
        resolver: Arc<dyn LocalTrackResolver>,
        engine: Box<dyn LocalAudioEngine>,
        events: SharedPlaybackEventSink,
        clock: Arc<dyn PlaybackClock>,
        sequence: Arc<ProcessSequence>,
    ) -> Self {
        let state = PlaybackState {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            source_id: source_id.clone(),
            source_kind: PlaybackStateSourceKind::Local,
            status: PlaybackStateStatus::Idle,
            capabilities: generated_capabilities(capabilities),
            current_track: None,
            position_ms: 0,
            duration_ms: None,
            revision: 0,
            updated_at: clock.now_rfc3339(),
            last_error: None,
        };
        Self {
            source_id,
            capabilities,
            resolver,
            engine,
            events,
            clock,
            sequence,
            state,
            mode: ActorMode::Idle,
            queue: Vec::new(),
            current_index: None,
            engine_session_id: None,
            recovery_session_id: None,
            resume_pending: false,
        }
    }

    fn run(mut self, mut receiver: mpsc::Receiver<ActorMessage>) {
        while let Some(message) = receiver.blocking_recv() {
            if self.handle(message) {
                return;
            }
        }
        self.engine.stop();
    }

    #[allow(clippy::too_many_lines)]
    fn handle(&mut self, message: ActorMessage) -> bool {
        match message {
            ActorMessage::SelectSource {
                source_id,
                response,
            } => {
                let _ = response.send(self.select_source(&source_id));
            }
            ActorMessage::GetState { response } => {
                let _ = response.send(self.get_state());
            }
            ActorMessage::Play {
                expected_revision,
                response,
            } => {
                let _ = response.send(self.play(expected_revision));
            }
            ActorMessage::Pause {
                expected_revision,
                response,
            } => {
                let _ = response.send(self.pause(expected_revision));
            }
            ActorMessage::Seek {
                expected_revision,
                position_ms,
                response,
            } => {
                let _ = response.send(self.seek(expected_revision, position_ms));
            }
            ActorMessage::Next {
                expected_revision,
                response,
            } => {
                let _ = response.send(self.next(expected_revision));
            }
            ActorMessage::Previous {
                expected_revision,
                response,
            } => {
                let _ = response.send(self.previous(expected_revision));
            }
            ActorMessage::SetQueue {
                track_ids,
                authorization,
                response,
            } => {
                let _ = response.send(self.set_queue(track_ids, authorization));
            }
            ActorMessage::EngineEvent(event) => self.handle_engine_event(event),
            ActorMessage::Suspend { response } => {
                let _ = response.send(self.suspend());
            }
            ActorMessage::ResumeSilent { response } => {
                let _ = response.send(self.resume_silent());
            }
            ActorMessage::Stop { response } => {
                let _ = response.send(self.stop());
            }
            ActorMessage::Shutdown { response } => {
                self.engine.stop();
                let _ = response.send(());
                return true;
            }
        }
        false
    }

    fn select_source(&mut self, source_id: &str) -> Result<PlaybackState, ApiError> {
        if source_id != self.source_id {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        }
        Ok(self.state.clone())
    }

    fn get_state(&mut self) -> Result<PlaybackState, ApiError> {
        if let Some(session_id) = self.engine_session_id {
            match self.engine.snapshot(session_id) {
                Ok(snapshot) if snapshot_is_valid(snapshot) => {
                    self.state.position_ms = snapshot.position_ms;
                    self.state.duration_ms = Some(snapshot.duration_ms);
                }
                Ok(_) => {
                    return self.fail(AudioEngineError::MediaInvalid);
                }
                Err(error) => return self.fail(error),
            }
        }
        Ok(self.state.clone())
    }

    fn play(&mut self, expected_revision: u64) -> Result<PlaybackState, ApiError> {
        self.ensure_revision(expected_revision)?;
        Self::ensure_capability(self.capabilities.play, CapabilityName::Play)?;
        if self.mode == ActorMode::Playing {
            return Ok(self.state.clone());
        }
        if self.mode == ActorMode::Idle && !self.queue.is_empty() {
            return self.load_from(0, true, PlaybackEventReason::UserCommand);
        }
        if !matches!(self.mode, ActorMode::Paused) {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        }
        let session_id = self
            .engine_session_id
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        let snapshot = self.engine.play(session_id).map_err(|error| {
            let api_error = audio_error(error);
            let _ = self.transition_error(error);
            api_error
        })?;
        self.apply_snapshot(snapshot)?;
        self.mode = ActorMode::Playing;
        self.state.status = PlaybackStateStatus::Playing;
        self.state.last_error = None;
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::UserCommand,
        )
    }

    fn pause(&mut self, expected_revision: u64) -> Result<PlaybackState, ApiError> {
        self.ensure_revision(expected_revision)?;
        Self::ensure_capability(self.capabilities.pause, CapabilityName::Pause)?;
        if self.mode == ActorMode::Paused {
            return Ok(self.state.clone());
        }
        if self.mode != ActorMode::Playing {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        }
        let session_id = self
            .engine_session_id
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        let snapshot = self.engine.pause(session_id).map_err(|error| {
            let api_error = audio_error(error);
            let _ = self.transition_error(error);
            api_error
        })?;
        self.apply_snapshot(snapshot)?;
        self.mode = ActorMode::Paused;
        self.state.status = PlaybackStateStatus::Paused;
        self.state.last_error = None;
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::UserCommand,
        )
    }

    fn seek(
        &mut self,
        expected_revision: u64,
        position_ms: u64,
    ) -> Result<PlaybackState, ApiError> {
        self.ensure_revision(expected_revision)?;
        Self::ensure_capability(self.capabilities.seek, CapabilityName::Seek)?;
        let duration_ms = self.state.duration_ms.ok_or_else(invalid_request)?;
        if position_ms > duration_ms || position_ms > MAX_POSITION_MS {
            return Err(invalid_request());
        }
        if !matches!(self.mode, ActorMode::Playing | ActorMode::Paused) {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        }
        let session_id = self
            .engine_session_id
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        let snapshot = self.engine.seek(session_id, position_ms).map_err(|error| {
            let api_error = audio_error(error);
            let _ = self.transition_error(error);
            api_error
        })?;
        self.apply_snapshot(snapshot)?;
        self.state.last_error = None;
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::UserCommand,
        )
    }

    fn next(&mut self, expected_revision: u64) -> Result<PlaybackState, ApiError> {
        self.ensure_revision(expected_revision)?;
        Self::ensure_capability(self.capabilities.next, CapabilityName::Next)?;
        let Some(current) = self.current_index else {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        };
        let next = current.saturating_add(1);
        if next >= self.queue.len() {
            self.engine.stop();
            return self.transition_idle(PlaybackEventReason::UserCommand);
        }
        let autoplay = self.mode == ActorMode::Playing;
        self.engine.stop();
        self.load_from(next, autoplay, PlaybackEventReason::UserCommand)
    }

    fn previous(&mut self, expected_revision: u64) -> Result<PlaybackState, ApiError> {
        self.ensure_revision(expected_revision)?;
        Self::ensure_capability(self.capabilities.previous, CapabilityName::Previous)?;
        let Some(current) = self.current_index else {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        };
        let position = self.current_snapshot()?.position_ms;
        if position > 5_000 || current == 0 {
            return self.seek(expected_revision, 0);
        }
        let autoplay = self.mode == ActorMode::Playing;
        self.engine.stop();
        self.load_from(current - 1, autoplay, PlaybackEventReason::UserCommand)
    }

    fn set_queue(
        &mut self,
        track_ids: Vec<String>,
        authorization: Option<PlaybackStartAuthorization>,
    ) -> Result<PlaybackState, ApiError> {
        Self::ensure_capability(self.capabilities.set_queue, CapabilityName::SetQueue)?;
        validate_queue_ids(&track_ids)?;
        let mut resolved = Vec::with_capacity(track_ids.len());
        for track_id in track_ids {
            resolved.push(self.resolver.resolve(&track_id).map_err(source_error)?);
        }
        self.engine.stop();
        self.queue = resolved;
        self.current_index = None;
        self.engine_session_id = None;
        self.recovery_session_id = None;
        self.resume_pending = false;
        if authorization.is_some() {
            self.load_from(0, true, PlaybackEventReason::UserCommand)
        } else if self.state.current_track.is_some()
            || !matches!(self.mode, ActorMode::Idle | ActorMode::Stopped)
        {
            self.transition_idle(PlaybackEventReason::UserCommand)
        } else {
            self.mode = ActorMode::Idle;
            self.state.status = PlaybackStateStatus::Idle;
            Ok(self.state.clone())
        }
    }

    fn load_from(
        &mut self,
        start_index: usize,
        autoplay: bool,
        reason: PlaybackEventReason,
    ) -> Result<PlaybackState, ApiError> {
        let mut last_error = None;
        for index in start_index..self.queue.len() {
            let track_id = self.queue[index].track_id().to_owned();
            let track = match self.resolver.resolve(&track_id) {
                Ok(track) => track,
                Err(error) => {
                    last_error = Some(source_error(error));
                    let _ = self.transition_source_error(error);
                    continue;
                }
            };
            self.current_index = Some(index);
            self.state.current_track = Some(contract_track(&track));
            self.state.position_ms = 0;
            self.state.duration_ms = Some(track.duration_ms());
            self.state.status = PlaybackStateStatus::Loading;
            self.state.last_error = None;
            self.mode = ActorMode::Loading;
            self.commit(PlaybackEventType::TrackChanged, reason.clone())?;

            let session_id = Uuid::now_v7();
            match self.engine.load(&track, session_id, 0, autoplay) {
                Ok(snapshot) if snapshot_is_valid(snapshot) => {
                    self.apply_snapshot(snapshot)?;
                    self.engine_session_id = Some(session_id);
                    self.recovery_session_id = None;
                    self.mode = if autoplay {
                        ActorMode::Playing
                    } else {
                        ActorMode::Paused
                    };
                    self.state.status = if autoplay {
                        PlaybackStateStatus::Playing
                    } else {
                        PlaybackStateStatus::Paused
                    };
                    self.state.last_error = None;
                    return self.commit(PlaybackEventType::StateChanged, reason);
                }
                Ok(_) => {
                    last_error = Some(audio_error(AudioEngineError::MediaInvalid));
                    let _ = self.transition_error(AudioEngineError::MediaInvalid);
                }
                Err(error) => {
                    last_error = Some(audio_error(error));
                    let _ = self.transition_error(error);
                    if error == AudioEngineError::OutputUnavailable {
                        break;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| ApiError::from_reason(InternalReason::MediaUnreadable)))
    }

    fn handle_engine_event(&mut self, event: LocalAudioEngineEvent) {
        match event {
            LocalAudioEngineEvent::Position {
                session_id,
                position_ms,
            } if self.engine_session_id == Some(session_id) => {
                if position_ms <= self.state.duration_ms.unwrap_or(0)
                    && position_ms <= MAX_POSITION_MS
                {
                    self.state.position_ms = position_ms;
                    self.state.updated_at = self.clock.now_rfc3339();
                }
            }
            LocalAudioEngineEvent::Ended { session_id }
                if self.engine_session_id == Some(session_id) =>
            {
                self.handle_ended();
            }
            LocalAudioEngineEvent::Failed { session_id, error }
                if self.engine_session_id == Some(session_id) =>
            {
                let _ = self.transition_error(error);
            }
            LocalAudioEngineEvent::OutputLost {
                session_id,
                position_ms,
            } if self.engine_session_id == Some(session_id) => {
                self.handle_output_lost(session_id, position_ms);
            }
            LocalAudioEngineEvent::OutputRestored {
                recovery_session_id,
            } if self.recovery_session_id == Some(recovery_session_id) => {
                let _ = self.recover_output();
            }
            _ => {}
        }
    }

    fn handle_ended(&mut self) {
        let next = self
            .current_index
            .map_or(0, |index| index.saturating_add(1));
        self.engine.stop();
        if next >= self.queue.len() {
            let _ = self.transition_idle(PlaybackEventReason::AdapterUpdate);
        } else {
            let _ = self.load_from(next, true, PlaybackEventReason::AdapterUpdate);
        }
    }

    fn handle_output_lost(&mut self, session_id: Uuid, position_ms: u64) {
        if position_ms <= self.state.duration_ms.unwrap_or(0) {
            self.state.position_ms = position_ms;
        }
        self.engine.stop();
        self.engine_session_id = None;
        self.recovery_session_id = Some(session_id);
        self.mode = ActorMode::RecoveringOutput;
        self.state.status = PlaybackStateStatus::Error;
        self.state.last_error = Some(safe_error(AudioEngineError::OutputUnavailable));
        let _ = self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::MediaError,
        );
    }

    fn recover_output(&mut self) -> Result<PlaybackState, ApiError> {
        let index = self
            .current_index
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        let track_id = self.queue[index].track_id().to_owned();
        let track = self.resolver.resolve(&track_id).map_err(source_error)?;
        let position = self.state.position_ms.min(track.duration_ms());
        let session_id = Uuid::now_v7();
        let snapshot = self
            .engine
            .load(&track, session_id, position, false)
            .map_err(audio_error)?;
        self.apply_snapshot(snapshot)?;
        self.engine_session_id = Some(session_id);
        self.recovery_session_id = None;
        self.resume_pending = false;
        self.mode = ActorMode::Paused;
        self.state.status = PlaybackStateStatus::Paused;
        self.state.last_error = None;
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::AdapterUpdate,
        )
    }

    fn suspend(&mut self) -> Result<PlaybackState, ApiError> {
        let Some(session_id) = self.engine_session_id else {
            return Ok(self.state.clone());
        };
        if let Ok(snapshot) = self.engine.snapshot(session_id)
            && snapshot_is_valid(snapshot)
        {
            self.apply_snapshot(snapshot)?;
        }
        self.engine.stop();
        self.engine_session_id = None;
        self.recovery_session_id = Some(session_id);
        self.resume_pending = self.current_index.is_some();
        self.mode = ActorMode::Paused;
        self.state.status = PlaybackStateStatus::Paused;
        self.state.last_error = None;
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::AdapterUpdate,
        )
    }

    fn resume_silent(&mut self) -> Result<PlaybackState, ApiError> {
        if !self.resume_pending {
            return Ok(self.state.clone());
        }
        self.recover_output()
    }

    fn stop(&mut self) -> Result<PlaybackState, ApiError> {
        self.engine.stop();
        self.queue.clear();
        self.current_index = None;
        self.engine_session_id = None;
        self.recovery_session_id = None;
        self.resume_pending = false;
        self.mode = ActorMode::Stopped;
        self.state.status = PlaybackStateStatus::Stopped;
        self.state.current_track = None;
        self.state.position_ms = 0;
        self.state.duration_ms = None;
        self.state.last_error = None;
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::UserCommand,
        )
    }

    fn transition_idle(&mut self, reason: PlaybackEventReason) -> Result<PlaybackState, ApiError> {
        self.current_index = None;
        self.engine_session_id = None;
        self.recovery_session_id = None;
        self.mode = ActorMode::Idle;
        self.state.status = PlaybackStateStatus::Idle;
        self.state.current_track = None;
        self.state.position_ms = 0;
        self.state.duration_ms = None;
        self.state.last_error = None;
        self.commit(PlaybackEventType::TrackChanged, reason)
    }

    fn fail<T>(&mut self, error: AudioEngineError) -> Result<T, ApiError> {
        let api_error = audio_error(error);
        let _ = self.transition_error(error);
        Err(api_error)
    }

    fn transition_error(&mut self, error: AudioEngineError) -> Result<PlaybackState, ApiError> {
        self.engine.stop();
        self.engine_session_id = None;
        self.mode = ActorMode::Error;
        self.state.status = PlaybackStateStatus::Error;
        self.state.last_error = Some(safe_error(error));
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::MediaError,
        )
    }

    fn transition_source_error(
        &mut self,
        error: LocalSourceError,
    ) -> Result<PlaybackState, ApiError> {
        let (reason, error_id) = match error {
            LocalSourceError::NotFound => (InternalReason::EntityNotFound, "ERR-1004"),
            LocalSourceError::PathDenied => (InternalReason::PathDenied, "ERR-1501"),
            LocalSourceError::MediaInvalid => (InternalReason::MediaMetadataInvalid, "ERR-1204"),
            LocalSourceError::Unavailable => (InternalReason::SourceUnavailable, "ERR-1202"),
        };
        let api_error = ApiError::from_reason(reason);
        self.engine.stop();
        self.engine_session_id = None;
        self.mode = ActorMode::Error;
        self.state.status = PlaybackStateStatus::Error;
        self.state.last_error = Some(PlaybackStateSafeError {
            error_id: error_id.to_owned(),
            safe_message: api_error.safe_message,
            retryable: api_error.retryable,
        });
        self.commit(
            PlaybackEventType::StateChanged,
            PlaybackEventReason::MediaError,
        )
    }

    fn current_snapshot(&mut self) -> Result<AudioSnapshot, ApiError> {
        let session_id = self
            .engine_session_id
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        let snapshot = self.engine.snapshot(session_id).map_err(audio_error)?;
        if !snapshot_is_valid(snapshot) {
            return Err(ApiError::from_reason(InternalReason::MediaMetadataInvalid));
        }
        self.apply_snapshot(snapshot)?;
        Ok(snapshot)
    }

    fn apply_snapshot(&mut self, snapshot: AudioSnapshot) -> Result<(), ApiError> {
        if !snapshot_is_valid(snapshot) {
            return Err(ApiError::from_reason(InternalReason::MediaMetadataInvalid));
        }
        self.state.position_ms = snapshot.position_ms;
        self.state.duration_ms = Some(snapshot.duration_ms);
        Ok(())
    }

    fn ensure_revision(&self, expected: u64) -> Result<(), ApiError> {
        if expected > MAX_JS_SAFE_INTEGER {
            return Err(invalid_request());
        }
        if expected == self.state.revision {
            Ok(())
        } else {
            Err(ApiError::from_reason(InternalReason::RevisionConflict)
                .with_current_revision(self.state.revision))
        }
    }

    fn ensure_capability(available: bool, capability: CapabilityName) -> Result<(), ApiError> {
        if available {
            Ok(())
        } else {
            Err(ApiError::from_reason(InternalReason::CapabilityAbsent).with_capability(capability))
        }
    }

    fn commit(
        &mut self,
        event_type: PlaybackEventType,
        reason: PlaybackEventReason,
    ) -> Result<PlaybackState, ApiError> {
        let revision = self
            .state
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_JS_SAFE_INTEGER)
            .ok_or_else(ApiError::unexpected)?;
        let sequence = self.sequence.next()?;
        let occurred_at = self.clock.now_rfc3339();
        self.state.revision = revision;
        self.state.updated_at.clone_from(&occurred_at);
        let event = PlaybackEvent {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            event_id: Uuid::now_v7().to_string(),
            sequence,
            r#type: event_type,
            occurred_at,
            source_id: self.source_id.clone(),
            state_revision: self.state.revision,
            reason: Some(reason),
            state: Some(self.state.clone()),
        };
        let _ = self.events.publish(event);
        Ok(self.state.clone())
    }
}

fn validate_queue_ids(track_ids: &[String]) -> Result<(), ApiError> {
    if !(1..=MAX_QUEUE_TRACKS).contains(&track_ids.len()) {
        return Err(invalid_request());
    }
    let mut unique = HashSet::with_capacity(track_ids.len());
    if track_ids
        .iter()
        .any(|track_id| !is_safe_id(track_id) || !unique.insert(track_id.as_str()))
    {
        return Err(invalid_request());
    }
    Ok(())
}

fn contract_track(track: &LocalTrack) -> PlaybackStateTrack {
    PlaybackStateTrack {
        track_id: track.track_id().to_owned(),
        title: track.title().to_owned(),
        artist: track.artist().map(ToOwned::to_owned),
        album: track.album().map(ToOwned::to_owned),
        artwork_uri: track.artwork_uri().map(ToOwned::to_owned),
        origin: PlaybackStateTrackOrigin::Local,
    }
}

const fn generated_capabilities(capabilities: SourceCapabilities) -> PlaybackStateCapabilities {
    PlaybackStateCapabilities {
        play: capabilities.play,
        pause: capabilities.pause,
        seek: capabilities.seek,
        next: capabilities.next,
        previous: capabilities.previous,
        set_queue: capabilities.set_queue,
    }
}

fn snapshot_is_valid(snapshot: AudioSnapshot) -> bool {
    (1..=MAX_POSITION_MS).contains(&snapshot.duration_ms)
        && snapshot.position_ms <= snapshot.duration_ms
}

fn safe_error(error: AudioEngineError) -> PlaybackStateSafeError {
    let (error_id, api_error) = match error {
        AudioEngineError::OutputUnavailable => (
            "ERR-1202",
            ApiError::from_reason(InternalReason::SourceUnavailable),
        ),
        AudioEngineError::MediaInvalid => (
            "ERR-1204",
            ApiError::from_reason(InternalReason::MediaUnreadable),
        ),
    };
    PlaybackStateSafeError {
        error_id: error_id.to_owned(),
        safe_message: api_error.safe_message,
        retryable: api_error.retryable,
    }
}

fn audio_error(error: AudioEngineError) -> ApiError {
    match error {
        AudioEngineError::OutputUnavailable => {
            ApiError::from_reason(InternalReason::SourceUnavailable)
        }
        AudioEngineError::MediaInvalid => ApiError::from_reason(InternalReason::MediaUnreadable),
    }
}

fn source_error(error: LocalSourceError) -> ApiError {
    match error {
        LocalSourceError::NotFound => ApiError::from_reason(InternalReason::EntityNotFound),
        LocalSourceError::PathDenied => ApiError::from_reason(InternalReason::PathDenied),
        LocalSourceError::MediaInvalid => {
            ApiError::from_reason(InternalReason::MediaMetadataInvalid)
        }
        LocalSourceError::Unavailable => ApiError::from_reason(InternalReason::SourceUnavailable),
    }
}

fn invalid_request() -> ApiError {
    ApiError::from_reason(InternalReason::RequestInvalid)
}

fn is_safe_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}
