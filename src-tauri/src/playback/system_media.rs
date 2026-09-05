use std::{
    sync::{Arc, RwLock, mpsc},
    time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{
    contracts::{
        PlaybackEvent, PlaybackEventReason, PlaybackEventType, PlaybackState,
        PlaybackStateCapabilities, PlaybackStateSourceKind, PlaybackStateStatus,
        PlaybackStateTrack, PlaybackStateTrackOrigin,
    },
    ipc::{
        ApiError, CapabilityName, InternalReason, ProcessSequence, SourceCapabilities, SourceKind,
        SourceSummary,
    },
};

use super::{events::SharedPlaybackEventSink, service::PlaybackClock};

pub(super) const APPLE_SOURCE_ID: &str = "apple_music";
const APPLE_SOURCE_NAME: &str = "Apple Music / Windows App";
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemMediaControl {
    Play,
    Pause,
    Seek(u64),
    Next,
    Previous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemMediaSnapshot {
    pub identity: String,
    pub status: PlaybackStateStatus,
    pub capabilities: SourceCapabilities,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SystemInterruptionToken {
    identity: String,
    revision: u64,
    resume: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SystemMediaError {
    Unavailable,
    SessionChanged,
    CapabilityAbsent(CapabilityName),
    Rejected,
}

pub(super) trait SystemMediaEventSink: Send + Sync + 'static {
    fn changed(&self);
}

/// Narrow adapter contract for the Windows system media session. Implementations
/// retain OS session identities internally and never expose AUMIDs over IPC.
pub trait SystemMediaBackend: Send + 'static {
    fn set_event_sink(&mut self, sink: Arc<dyn SystemMediaEventSink>);
    fn refresh(&mut self) -> Result<SystemMediaSnapshot, SystemMediaError>;
    fn control(
        &mut self,
        expected_identity: &str,
        action: SystemMediaControl,
    ) -> Result<SystemMediaSnapshot, SystemMediaError>;
}

enum SystemMessage {
    Select {
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    GetState {
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Control {
        expected_revision: u64,
        action: SystemMediaControl,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    BeginInterruption {
        response: oneshot::Sender<Result<SystemInterruptionToken, ApiError>>,
    },
    FinishInterruption {
        token: SystemInterruptionToken,
        response: oneshot::Sender<Result<PlaybackState, ApiError>>,
    },
    Refresh,
    Shutdown,
}

struct ChannelEventSink(mpsc::SyncSender<SystemMessage>);

impl SystemMediaEventSink for ChannelEventSink {
    fn changed(&self) {
        let _ = self.0.try_send(SystemMessage::Refresh);
    }
}

struct SystemClient {
    sender: mpsc::SyncSender<SystemMessage>,
}

impl Drop for SystemClient {
    fn drop(&mut self) {
        let _ = self.sender.try_send(SystemMessage::Shutdown);
    }
}

#[derive(Clone)]
pub(super) struct SystemMediaSource {
    client: Arc<SystemClient>,
    state: Arc<RwLock<PlaybackState>>,
}

impl SystemMediaSource {
    pub(super) fn production(
        events: SharedPlaybackEventSink,
        clock: Arc<dyn PlaybackClock>,
        sequence: Arc<ProcessSequence>,
    ) -> Result<Self, ApiError> {
        Self::new(production_backend(), events, clock, sequence)
    }

    pub(super) fn new(
        mut backend: Box<dyn SystemMediaBackend>,
        events: SharedPlaybackEventSink,
        clock: Arc<dyn PlaybackClock>,
        sequence: Arc<ProcessSequence>,
    ) -> Result<Self, ApiError> {
        let state = Arc::new(RwLock::new(disconnected_state(clock.as_ref())));
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        backend.set_event_sink(Arc::new(ChannelEventSink(sender.clone())));
        let mut actor = SystemMediaActor {
            backend,
            events,
            clock,
            sequence,
            shared_state: Arc::clone(&state),
            state: state.read().map_err(|_| ApiError::unexpected())?.clone(),
            identity: None,
        };
        std::thread::Builder::new()
            .name("cyberkindred-system-media".to_owned())
            .spawn(move || actor.run(&receiver))
            .map_err(|_| ApiError::unexpected())?;
        Ok(Self {
            client: Arc::new(SystemClient { sender }),
            state,
        })
    }

    pub(super) fn summary(&self) -> SourceSummary {
        let state = self.state.read().map_or_else(
            |_| disconnected_state(&super::service::SystemPlaybackClock),
            |state| state.clone(),
        );
        SourceSummary {
            source_id: APPLE_SOURCE_ID.to_owned(),
            kind: SourceKind::SystemSession,
            display_name: APPLE_SOURCE_NAME.to_owned(),
            connected: state.status != PlaybackStateStatus::Disconnected,
            capabilities: source_capabilities(&state.capabilities),
        }
    }

    pub(super) async fn select(&self) -> Result<PlaybackState, ApiError> {
        self.request(|response| SystemMessage::Select { response })
            .await
    }

    pub(super) async fn get_state(&self) -> Result<PlaybackState, ApiError> {
        self.request(|response| SystemMessage::GetState { response })
            .await
    }

    pub(super) async fn control(
        &self,
        expected_revision: u64,
        action: SystemMediaControl,
    ) -> Result<PlaybackState, ApiError> {
        self.request(|response| SystemMessage::Control {
            expected_revision,
            action,
            response,
        })
        .await
    }

    pub(super) async fn begin_interruption(&self) -> Result<SystemInterruptionToken, ApiError> {
        let (response, receiver) = oneshot::channel();
        self.client
            .sender
            .send(SystemMessage::BeginInterruption { response })
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())?
    }

    pub(super) async fn finish_interruption(
        &self,
        token: SystemInterruptionToken,
    ) -> Result<PlaybackState, ApiError> {
        let (response, receiver) = oneshot::channel();
        self.client
            .sender
            .send(SystemMessage::FinishInterruption { token, response })
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())?
    }

    async fn request<F>(&self, message: F) -> Result<PlaybackState, ApiError>
    where
        F: FnOnce(oneshot::Sender<Result<PlaybackState, ApiError>>) -> SystemMessage,
    {
        let (response, receiver) = oneshot::channel();
        self.client
            .sender
            .send(message(response))
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())?
    }
}

struct SystemMediaActor {
    backend: Box<dyn SystemMediaBackend>,
    events: SharedPlaybackEventSink,
    clock: Arc<dyn PlaybackClock>,
    sequence: Arc<ProcessSequence>,
    shared_state: Arc<RwLock<PlaybackState>>,
    state: PlaybackState,
    identity: Option<String>,
}

impl SystemMediaActor {
    fn run(&mut self, receiver: &mpsc::Receiver<SystemMessage>) {
        self.refresh(PlaybackEventReason::AdapterUpdate);
        loop {
            match receiver.recv_timeout(REFRESH_INTERVAL) {
                Ok(SystemMessage::Select { response }) => {
                    self.refresh(PlaybackEventReason::AdapterUpdate);
                    let result = if self.identity.is_some() {
                        Ok(self.state.clone())
                    } else {
                        Err(ApiError::from_reason(InternalReason::SourceUnavailable))
                    };
                    let _ = response.send(result);
                }
                Ok(SystemMessage::GetState { response }) => {
                    self.refresh(PlaybackEventReason::AdapterUpdate);
                    let _ = response.send(Ok(self.state.clone()));
                }
                Ok(SystemMessage::Control {
                    expected_revision,
                    action,
                    response,
                }) => {
                    let result = self.control(expected_revision, action);
                    let _ = response.send(result);
                }
                Ok(SystemMessage::BeginInterruption { response }) => {
                    let result = self.begin_interruption();
                    let _ = response.send(result);
                }
                Ok(SystemMessage::FinishInterruption { token, response }) => {
                    let state = self.finish_interruption(&token);
                    let _ = response.send(Ok(state));
                }
                Ok(SystemMessage::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.refresh(PlaybackEventReason::AdapterUpdate);
                }
                Ok(SystemMessage::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn control(
        &mut self,
        expected_revision: u64,
        action: SystemMediaControl,
    ) -> Result<PlaybackState, ApiError> {
        if expected_revision != self.state.revision {
            return Err(ApiError::from_reason(InternalReason::StateRevisionChanged)
                .with_current_revision(self.state.revision));
        }
        let capability = capability_for(action);
        if !capability_enabled(&self.state.capabilities, capability) {
            return Err(
                ApiError::from_reason(InternalReason::CapabilityAbsent).with_capability(capability)
            );
        }
        let identity = self
            .identity
            .clone()
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        match self.backend.control(&identity, action) {
            Ok(snapshot) => {
                if snapshot.identity != identity {
                    self.refresh(PlaybackEventReason::SessionReplaced);
                    return Err(ApiError::from_reason(
                        InternalReason::SessionIdentityChanged,
                    ));
                }
                self.apply(snapshot, PlaybackEventReason::UserCommand);
                Ok(self.state.clone())
            }
            Err(error) => {
                self.refresh(PlaybackEventReason::AdapterUpdate);
                Err(map_error(error))
            }
        }
    }

    fn begin_interruption(&mut self) -> Result<SystemInterruptionToken, ApiError> {
        self.refresh(PlaybackEventReason::AdapterUpdate);
        let identity = self
            .identity
            .clone()
            .ok_or_else(|| ApiError::from_reason(InternalReason::SourceUnavailable))?;
        if self.state.status != PlaybackStateStatus::Playing {
            return Ok(SystemInterruptionToken {
                identity,
                revision: self.state.revision,
                resume: false,
            });
        }
        if !self.state.capabilities.pause {
            return Err(ApiError::from_reason(InternalReason::CapabilityAbsent)
                .with_capability(CapabilityName::Pause));
        }
        let snapshot = self
            .backend
            .control(&identity, SystemMediaControl::Pause)
            .map_err(map_error)?;
        if snapshot.identity != identity || snapshot.status != PlaybackStateStatus::Paused {
            self.refresh(PlaybackEventReason::AdapterUpdate);
            return Err(ApiError::from_reason(
                InternalReason::SessionIdentityChanged,
            ));
        }
        self.apply(snapshot, PlaybackEventReason::UserCommand);
        Ok(SystemInterruptionToken {
            identity,
            revision: self.state.revision,
            resume: true,
        })
    }

    fn finish_interruption(&mut self, token: &SystemInterruptionToken) -> PlaybackState {
        self.refresh(PlaybackEventReason::AdapterUpdate);
        if !token.resume {
            return self.state.clone();
        }
        let same_identity = self.identity.as_deref() == Some(token.identity.as_str());
        let untouched = self.state.revision == token.revision
            && self.state.status == PlaybackStateStatus::Paused;
        if !same_identity || !untouched {
            let event_type = if same_identity {
                PlaybackEventType::UserOverride
            } else {
                PlaybackEventType::ProgramInterrupted
            };
            let reason = if same_identity {
                PlaybackEventReason::UserMediaKey
            } else {
                PlaybackEventReason::SessionReplaced
            };
            self.publish(event_type, reason);
            return self.state.clone();
        }
        if !self.state.capabilities.play {
            self.publish(
                PlaybackEventType::ProgramInterrupted,
                PlaybackEventReason::TtsResumeAborted,
            );
            return self.state.clone();
        }
        match self
            .backend
            .control(&token.identity, SystemMediaControl::Play)
        {
            Ok(snapshot)
                if snapshot.identity == token.identity
                    && snapshot.status == PlaybackStateStatus::Playing =>
            {
                self.apply(snapshot, PlaybackEventReason::UserCommand);
            }
            Ok(_) | Err(_) => {
                self.refresh(PlaybackEventReason::AdapterUpdate);
                self.publish(
                    PlaybackEventType::ProgramInterrupted,
                    PlaybackEventReason::TtsResumeAborted,
                );
            }
        }
        self.state.clone()
    }

    fn refresh(&mut self, reason: PlaybackEventReason) {
        match self.backend.refresh() {
            Ok(snapshot) => {
                let reason = if self
                    .identity
                    .as_deref()
                    .is_some_and(|identity| identity != snapshot.identity)
                {
                    PlaybackEventReason::SessionReplaced
                } else {
                    reason
                };
                self.apply(snapshot, reason);
            }
            Err(_) => self.disconnect(),
        }
    }

    fn apply(&mut self, snapshot: SystemMediaSnapshot, reason: PlaybackEventReason) {
        let previous = self.state.clone();
        let next_track = track_from_snapshot(&snapshot);
        self.identity = Some(snapshot.identity);
        self.state.status = snapshot.status;
        self.state.capabilities = generated_capabilities(snapshot.capabilities);
        self.state.current_track = next_track;
        self.state.position_ms = snapshot.position_ms;
        self.state.duration_ms = snapshot.duration_ms;
        self.state.last_error = None;

        let event_type = semantic_event_type(&previous, &self.state);
        if let Some(event_type) = event_type {
            self.state.revision = self.state.revision.saturating_add(1);
            self.state.updated_at = self.clock.now_rfc3339();
            self.publish(event_type, reason);
        }
        self.update_shared();
    }

    fn disconnect(&mut self) {
        let was_connected = self.identity.take().is_some()
            || self.state.status != PlaybackStateStatus::Disconnected;
        if !was_connected {
            return;
        }
        self.state.status = PlaybackStateStatus::Disconnected;
        self.state.capabilities = generated_capabilities(no_capabilities());
        self.state.current_track = None;
        self.state.position_ms = 0;
        self.state.duration_ms = None;
        self.state.last_error = None;
        self.state.revision = self.state.revision.saturating_add(1);
        self.state.updated_at = self.clock.now_rfc3339();
        self.publish(
            PlaybackEventType::SourceDisconnected,
            PlaybackEventReason::SessionLost,
        );
        self.update_shared();
    }

    fn publish(&self, event_type: PlaybackEventType, reason: PlaybackEventReason) {
        let Ok(sequence) = self.sequence.next() else {
            return;
        };
        let event = PlaybackEvent {
            schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
            event_id: Uuid::now_v7().to_string(),
            sequence,
            r#type: event_type,
            occurred_at: self.clock.now_rfc3339(),
            source_id: APPLE_SOURCE_ID.to_owned(),
            state_revision: self.state.revision,
            reason: Some(reason),
            state: Some(self.state.clone()),
        };
        let _ = self.events.publish(event);
    }

    fn update_shared(&self) {
        if let Ok(mut shared) = self.shared_state.write() {
            *shared = self.state.clone();
        }
    }
}

fn disconnected_state(clock: &dyn PlaybackClock) -> PlaybackState {
    PlaybackState {
        schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
        source_id: APPLE_SOURCE_ID.to_owned(),
        source_kind: PlaybackStateSourceKind::SystemSession,
        status: PlaybackStateStatus::Disconnected,
        capabilities: generated_capabilities(no_capabilities()),
        current_track: None,
        position_ms: 0,
        duration_ms: None,
        revision: 0,
        updated_at: clock.now_rfc3339(),
        last_error: None,
    }
}

const fn no_capabilities() -> SourceCapabilities {
    SourceCapabilities {
        play: false,
        pause: false,
        seek: false,
        next: false,
        previous: false,
        set_queue: false,
    }
}

const fn generated_capabilities(value: SourceCapabilities) -> PlaybackStateCapabilities {
    PlaybackStateCapabilities {
        play: value.play,
        pause: value.pause,
        seek: value.seek,
        next: value.next,
        previous: value.previous,
        set_queue: false,
    }
}

const fn source_capabilities(value: &PlaybackStateCapabilities) -> SourceCapabilities {
    SourceCapabilities {
        play: value.play,
        pause: value.pause,
        seek: value.seek,
        next: value.next,
        previous: value.previous,
        set_queue: false,
    }
}

const fn capability_for(action: SystemMediaControl) -> CapabilityName {
    match action {
        SystemMediaControl::Play => CapabilityName::Play,
        SystemMediaControl::Pause => CapabilityName::Pause,
        SystemMediaControl::Seek(_) => CapabilityName::Seek,
        SystemMediaControl::Next => CapabilityName::Next,
        SystemMediaControl::Previous => CapabilityName::Previous,
    }
}

const fn capability_enabled(value: &PlaybackStateCapabilities, capability: CapabilityName) -> bool {
    match capability {
        CapabilityName::Play => value.play,
        CapabilityName::Pause => value.pause,
        CapabilityName::Seek => value.seek,
        CapabilityName::Next => value.next,
        CapabilityName::Previous => value.previous,
        CapabilityName::SetQueue => false,
    }
}

fn track_from_snapshot(snapshot: &SystemMediaSnapshot) -> Option<PlaybackStateTrack> {
    let title = snapshot.title.as_ref()?;
    let mut hasher = Sha256::new();
    for field in [
        Some(title.as_str()),
        snapshot.artist.as_deref(),
        snapshot.album.as_deref(),
    ] {
        let value = field.unwrap_or_default().as_bytes();
        hasher.update(value.len().to_le_bytes());
        hasher.update(value);
    }
    hasher.update(snapshot.duration_ms.unwrap_or_default().to_le_bytes());
    let digest = hex::encode(hasher.finalize());
    Some(PlaybackStateTrack {
        track_id: format!("system:{}", &digest[..32]),
        title: title.clone(),
        artist: snapshot.artist.clone(),
        album: snapshot.album.clone(),
        artwork_uri: None,
        origin: PlaybackStateTrackOrigin::SystemSession,
    })
}

fn semantic_event_type(
    previous: &PlaybackState,
    next: &PlaybackState,
) -> Option<PlaybackEventType> {
    if previous.current_track != next.current_track {
        Some(PlaybackEventType::TrackChanged)
    } else if previous.capabilities != next.capabilities {
        Some(PlaybackEventType::CapabilitiesChanged)
    } else if previous.status != next.status {
        Some(PlaybackEventType::StateChanged)
    } else {
        None
    }
}

fn map_error(error: SystemMediaError) -> ApiError {
    match error {
        SystemMediaError::Unavailable | SystemMediaError::Rejected => {
            ApiError::from_reason(InternalReason::SourceUnavailable)
        }
        SystemMediaError::SessionChanged => {
            ApiError::from_reason(InternalReason::SessionIdentityChanged)
        }
        SystemMediaError::CapabilityAbsent(capability) => {
            ApiError::from_reason(InternalReason::CapabilityAbsent).with_capability(capability)
        }
    }
}

#[cfg(windows)]
fn production_backend() -> Box<dyn SystemMediaBackend> {
    Box::new(windows_backend::WindowsSystemMediaBackend::new())
}

#[cfg(not(windows))]
fn production_backend() -> Box<dyn SystemMediaBackend> {
    Box::new(UnavailableSystemMediaBackend)
}

#[cfg(not(windows))]
struct UnavailableSystemMediaBackend;

#[cfg(not(windows))]
impl SystemMediaBackend for UnavailableSystemMediaBackend {
    fn set_event_sink(&mut self, _sink: Arc<dyn SystemMediaEventSink>) {}

    fn refresh(&mut self) -> Result<SystemMediaSnapshot, SystemMediaError> {
        Err(SystemMediaError::Unavailable)
    }

    fn control(
        &mut self,
        _expected_identity: &str,
        _action: SystemMediaControl,
    ) -> Result<SystemMediaSnapshot, SystemMediaError> {
        Err(SystemMediaError::Unavailable)
    }
}

#[cfg(windows)]
mod windows_backend {
    use std::time::{Duration, Instant};

    use super::{
        Arc, CapabilityName, PlaybackStateStatus, SourceCapabilities, SystemMediaBackend,
        SystemMediaControl, SystemMediaError, SystemMediaEventSink, SystemMediaSnapshot, Uuid,
        capability_for,
    };
    use windows::{
        Foundation::TypedEventHandler,
        Media::Control::{
            GlobalSystemMediaTransportControlsSession as Session,
            GlobalSystemMediaTransportControlsSessionManager as SessionManager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
            MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs,
            SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
        },
    };

    const APPLE_AUMID_PREFIX: &str = "AppleInc.AppleMusicWin_";
    const APPLE_AUMID_SUFFIX: &str = "!App";
    const TICKS_PER_MILLISECOND: i64 = 10_000;
    const CONTROL_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(2);
    const CONTROL_CONFIRMATION_POLL: Duration = Duration::from_millis(50);

    struct BoundSession {
        identity: String,
        session: Session,
        playback_token: i64,
        media_token: i64,
        timeline_token: i64,
    }

    pub(super) struct WindowsSystemMediaBackend {
        manager: Option<SessionManager>,
        manager_token: Option<i64>,
        bound: Option<BoundSession>,
        sink: Option<Arc<dyn SystemMediaEventSink>>,
    }

    impl WindowsSystemMediaBackend {
        pub(super) const fn new() -> Self {
            Self {
                manager: None,
                manager_token: None,
                bound: None,
                sink: None,
            }
        }

        fn ensure_manager(&mut self) -> Result<(), SystemMediaError> {
            if self.manager.is_some() {
                return Ok(());
            }
            let manager = SessionManager::RequestAsync()
                .and_then(|operation| operation.join())
                .map_err(|_| SystemMediaError::Unavailable)?;
            if let Some(sink) = &self.sink {
                let sink = Arc::clone(sink);
                let handler = TypedEventHandler::<SessionManager, SessionsChangedEventArgs>::new(
                    move |_, _| {
                        sink.changed();
                        Ok(())
                    },
                );
                self.manager_token = manager.SessionsChanged(&handler).ok();
            }
            self.manager = Some(manager);
            Ok(())
        }

        fn unique_apple_session(&mut self) -> Result<Session, SystemMediaError> {
            self.ensure_manager()?;
            let manager = self.manager.as_ref().ok_or(SystemMediaError::Unavailable)?;
            let sessions = manager
                .GetSessions()
                .map_err(|_| SystemMediaError::Unavailable)?;
            let size = sessions.Size().map_err(|_| SystemMediaError::Unavailable)?;
            let mut candidate = None;
            for index in 0..size {
                let session = sessions
                    .GetAt(index)
                    .map_err(|_| SystemMediaError::Unavailable)?;
                let app_id = session
                    .SourceAppUserModelId()
                    .map_err(|_| SystemMediaError::Unavailable)?
                    .to_string_lossy();
                if is_apple_aumid(&app_id) {
                    if candidate.is_some() {
                        return Err(SystemMediaError::Unavailable);
                    }
                    candidate = Some(session);
                }
            }
            candidate.ok_or(SystemMediaError::Unavailable)
        }

        fn bind(&mut self, session: Session) -> Result<(), SystemMediaError> {
            if self
                .bound
                .as_ref()
                .is_some_and(|bound| bound.session == session)
            {
                return Ok(());
            }
            self.unbind();
            let sink = self.sink.clone().ok_or(SystemMediaError::Unavailable)?;
            let playback_sink = Arc::clone(&sink);
            let playback =
                TypedEventHandler::<Session, PlaybackInfoChangedEventArgs>::new(move |_, _| {
                    playback_sink.changed();
                    Ok(())
                });
            let media_sink = Arc::clone(&sink);
            let media =
                TypedEventHandler::<Session, MediaPropertiesChangedEventArgs>::new(move |_, _| {
                    media_sink.changed();
                    Ok(())
                });
            let timeline = TypedEventHandler::<Session, TimelinePropertiesChangedEventArgs>::new(
                move |_, _| {
                    sink.changed();
                    Ok(())
                },
            );
            let playback_token = session
                .PlaybackInfoChanged(&playback)
                .map_err(|_| SystemMediaError::Unavailable)?;
            let media_token = session.MediaPropertiesChanged(&media).map_err(|_| {
                let _ = session.RemovePlaybackInfoChanged(playback_token);
                SystemMediaError::Unavailable
            })?;
            let timeline_token = session.TimelinePropertiesChanged(&timeline).map_err(|_| {
                let _ = session.RemovePlaybackInfoChanged(playback_token);
                let _ = session.RemoveMediaPropertiesChanged(media_token);
                SystemMediaError::Unavailable
            })?;
            self.bound = Some(BoundSession {
                identity: Uuid::now_v7().to_string(),
                session,
                playback_token,
                media_token,
                timeline_token,
            });
            Ok(())
        }

        fn unbind(&mut self) {
            if let Some(bound) = self.bound.take() {
                let _ = bound
                    .session
                    .RemovePlaybackInfoChanged(bound.playback_token);
                let _ = bound
                    .session
                    .RemoveMediaPropertiesChanged(bound.media_token);
                let _ = bound
                    .session
                    .RemoveTimelinePropertiesChanged(bound.timeline_token);
            }
        }

        fn reset_manager(&mut self) {
            self.unbind();
            if let (Some(manager), Some(token)) = (&self.manager, self.manager_token.take()) {
                let _ = manager.RemoveSessionsChanged(token);
            }
            self.manager = None;
        }

        fn read_bound(&self) -> Result<SystemMediaSnapshot, SystemMediaError> {
            let bound = self.bound.as_ref().ok_or(SystemMediaError::Unavailable)?;
            read_snapshot(&bound.session, &bound.identity)
        }
    }

    impl Drop for WindowsSystemMediaBackend {
        fn drop(&mut self) {
            self.reset_manager();
        }
    }

    impl SystemMediaBackend for WindowsSystemMediaBackend {
        fn set_event_sink(&mut self, sink: Arc<dyn SystemMediaEventSink>) {
            self.sink = Some(sink);
        }

        fn refresh(&mut self) -> Result<SystemMediaSnapshot, SystemMediaError> {
            let session = self.unique_apple_session().inspect_err(|_| {
                self.reset_manager();
            })?;
            self.bind(session)?;
            self.read_bound()
        }

        fn control(
            &mut self,
            expected_identity: &str,
            action: SystemMediaControl,
        ) -> Result<SystemMediaSnapshot, SystemMediaError> {
            let session = self.unique_apple_session()?;
            let bound = self
                .bound
                .as_ref()
                .ok_or(SystemMediaError::SessionChanged)?;
            if bound.identity != expected_identity || bound.session != session {
                return Err(SystemMediaError::SessionChanged);
            }
            let before = read_snapshot(&bound.session, &bound.identity)?;
            let capability = capability_for(action);
            if !source_capability_enabled(before.capabilities, capability) {
                return Err(SystemMediaError::CapabilityAbsent(capability));
            }
            let accepted = match action {
                SystemMediaControl::Play => bound.session.TryPlayAsync(),
                SystemMediaControl::Pause => bound.session.TryPauseAsync(),
                SystemMediaControl::Seek(position_ms) => {
                    let ticks = i64::try_from(position_ms)
                        .ok()
                        .and_then(|value| value.checked_mul(TICKS_PER_MILLISECOND))
                        .ok_or(SystemMediaError::Rejected)?;
                    bound.session.TryChangePlaybackPositionAsync(ticks)
                }
                SystemMediaControl::Next => bound.session.TrySkipNextAsync(),
                SystemMediaControl::Previous => bound.session.TrySkipPreviousAsync(),
            }
            .and_then(|operation| operation.join())
            .map_err(|_| SystemMediaError::Rejected)?;
            if !accepted {
                return Err(SystemMediaError::Rejected);
            }
            let deadline = Instant::now() + CONTROL_CONFIRMATION_TIMEOUT;
            loop {
                if let Ok(after) = self.read_bound()
                    && action_confirmed(action, &before, &after)
                {
                    return Ok(after);
                }
                if Instant::now() >= deadline {
                    return Err(SystemMediaError::Rejected);
                }
                std::thread::sleep(CONTROL_CONFIRMATION_POLL);
            }
        }
    }

    fn is_apple_aumid(value: &str) -> bool {
        value.starts_with(APPLE_AUMID_PREFIX) && value.ends_with(APPLE_AUMID_SUFFIX)
    }

    fn read_snapshot(
        session: &Session,
        identity: &str,
    ) -> Result<SystemMediaSnapshot, SystemMediaError> {
        let playback = session
            .GetPlaybackInfo()
            .map_err(|_| SystemMediaError::Unavailable)?;
        let status = match playback
            .PlaybackStatus()
            .map_err(|_| SystemMediaError::Unavailable)?
        {
            PlaybackStatus::Closed => PlaybackStateStatus::Disconnected,
            PlaybackStatus::Changing => PlaybackStateStatus::Loading,
            PlaybackStatus::Stopped => PlaybackStateStatus::Stopped,
            PlaybackStatus::Playing => PlaybackStateStatus::Playing,
            PlaybackStatus::Paused => PlaybackStateStatus::Paused,
            _ => PlaybackStateStatus::Idle,
        };
        let controls = playback
            .Controls()
            .map_err(|_| SystemMediaError::Unavailable)?;
        let capabilities = SourceCapabilities {
            play: controls.IsPlayEnabled().unwrap_or(false),
            pause: controls.IsPauseEnabled().unwrap_or(false),
            seek: controls.IsPlaybackPositionEnabled().unwrap_or(false),
            next: controls.IsNextEnabled().unwrap_or(false),
            previous: controls.IsPreviousEnabled().unwrap_or(false),
            set_queue: false,
        };
        let media = session
            .TryGetMediaPropertiesAsync()
            .and_then(|operation| operation.join())
            .ok();
        let title = media
            .as_ref()
            .and_then(|value| value.Title().ok())
            .and_then(|value| clean_text(&value.to_string_lossy()));
        let artist = media
            .as_ref()
            .and_then(|value| value.Artist().ok())
            .and_then(|value| clean_text(&value.to_string_lossy()));
        let album = media
            .as_ref()
            .and_then(|value| value.AlbumTitle().ok())
            .and_then(|value| clean_text(&value.to_string_lossy()));
        let timeline = session.GetTimelineProperties().ok();
        let position_ms = timeline
            .as_ref()
            .and_then(|value| value.Position().ok())
            .map_or(0, |value| ticks_to_ms(value.Duration));
        let duration_ms = timeline.as_ref().and_then(|value| {
            let start = value.StartTime().ok()?.Duration;
            let end = value.EndTime().ok()?.Duration;
            (end > start).then(|| ticks_to_ms(end.saturating_sub(start)))
        });
        Ok(SystemMediaSnapshot {
            identity: identity.to_owned(),
            status,
            capabilities,
            title,
            artist,
            album,
            position_ms,
            duration_ms,
        })
    }

    fn clean_text(value: &str) -> Option<String> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        Some(
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(512)
                .collect(),
        )
        .filter(|value: &String| !value.is_empty())
    }

    fn ticks_to_ms(value: i64) -> u64 {
        u64::try_from(value.max(0) / TICKS_PER_MILLISECOND).unwrap_or_default()
    }

    fn action_confirmed(
        action: SystemMediaControl,
        before: &SystemMediaSnapshot,
        after: &SystemMediaSnapshot,
    ) -> bool {
        match action {
            SystemMediaControl::Play => after.status == PlaybackStateStatus::Playing,
            SystemMediaControl::Pause => after.status == PlaybackStateStatus::Paused,
            SystemMediaControl::Seek(position_ms) => after.position_ms.abs_diff(position_ms) <= 500,
            SystemMediaControl::Next | SystemMediaControl::Previous => {
                after.title != before.title
                    || after.artist != before.artist
                    || after.album != before.album
                    || after.duration_ms != before.duration_ms
            }
        }
    }

    const fn source_capability_enabled(
        value: SourceCapabilities,
        capability: CapabilityName,
    ) -> bool {
        match capability {
            CapabilityName::Play => value.play,
            CapabilityName::Pause => value.pause,
            CapabilityName::Seek => value.seek,
            CapabilityName::Next => value.next,
            CapabilityName::Previous => value.previous,
            CapabilityName::SetQueue => false,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::is_apple_aumid;

        #[test]
        fn system_media_source_matches_only_apple_music_windows_app_aumid() {
            assert!(is_apple_aumid("AppleInc.AppleMusicWin_nzyj5cx40ttqa!App"));
            assert!(!is_apple_aumid("chrome.exe"));
            assert!(!is_apple_aumid("msedge.exe"));
            assert!(!is_apple_aumid("music.apple.com"));
            assert!(!is_apple_aumid(
                "AppleInc.AppleMusicWin_nzyj5cx40ttqa!Background"
            ));
        }
    }
}
