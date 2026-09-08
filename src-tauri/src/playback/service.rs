use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use tokio::sync::{mpsc, oneshot};

use crate::{
    contracts::PlaybackState,
    ipc::{
        ApiError, EmptyRequest, InternalReason, ProcessSequence, SourceCapabilities, SourceKind,
        SourceSummary, canonical_request_hash,
    },
};

use super::{
    actor::{ActorMessage, spawn_actor},
    artwork::ArtworkAssetStore,
    dto::{
        ListMusicSourcesResponse, PlaybackControlRequest, SeekPlaybackRequest,
        SelectMusicSourceRequest, SelectMusicSourceResponse,
    },
    engine::{
        LocalAudioEngine, LocalAudioEngineEvent, LocalTrackResolver, PlaybackStartAuthorization,
    },
    events::SharedPlaybackEventSink,
    idempotency::AsyncIdempotency,
    system_media::{
        APPLE_SOURCE_ID, SystemMediaControl, SystemMediaSource, apple_music_app_installed,
    },
};

pub(crate) use super::system_media::SystemInterruptionToken;

#[cfg(test)]
use super::system_media::SystemMediaBackend;

const STANDARD_RETENTION: Duration = Duration::from_mins(10);
const MEDIA_RETENTION: Duration = Duration::from_secs(30);
const IDEMPOTENCY_CAPACITY: usize = 256;
const LOCAL_SOURCE_ID: &str = "local";
const LOCAL_SOURCE_NAME: &str = "本地曲库";

pub trait PlaybackClock: Send + Sync + 'static {
    fn now_rfc3339(&self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPlaybackClock;

impl PlaybackClock for SystemPlaybackClock {
    fn now_rfc3339(&self) -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

struct PlaybackClient {
    sender: mpsc::Sender<ActorMessage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveSource {
    Local,
    System,
}

#[derive(Clone, Copy)]
enum ControlAction {
    Play,
    Pause,
    Seek(u64),
    Next,
    Previous,
}

impl Drop for PlaybackClient {
    fn drop(&mut self) {
        let (response, _receiver) = oneshot::channel();
        let _ = self.sender.try_send(ActorMessage::Shutdown { response });
    }
}

#[derive(Clone)]
pub struct PlaybackService {
    local_client: Arc<PlaybackClient>,
    local_source: SourceSummary,
    system_source: SystemMediaSource,
    active_source: Arc<Mutex<ActiveSource>>,
    select_requests: Arc<AsyncIdempotency<SelectMusicSourceResponse>>,
    control_requests: Arc<AsyncIdempotency<PlaybackState>>,
    accepting_sound: Arc<AtomicBool>,
}

impl PlaybackService {
    /// Creates the serialized local actor. Construction performs no output or
    /// filesystem side effect.
    ///
    /// # Errors
    ///
    /// Returns a safe validation/actor-start error.
    pub fn new(
        resolver: Arc<dyn LocalTrackResolver>,
        engine: Box<dyn LocalAudioEngine>,
        events: SharedPlaybackEventSink,
        clock: Arc<dyn PlaybackClock>,
        sequence: Arc<ProcessSequence>,
        capabilities: SourceCapabilities,
        artwork: ArtworkAssetStore,
    ) -> Result<Self, ApiError> {
        let system_source = SystemMediaSource::production(
            Arc::clone(&events),
            Arc::clone(&clock),
            Arc::clone(&sequence),
            artwork,
        )?;
        Self::new_with_system_source(
            resolver,
            engine,
            events,
            clock,
            sequence,
            capabilities,
            system_source,
        )
    }

    #[cfg(test)]
    pub(super) fn new_with_system_backend(
        resolver: Arc<dyn LocalTrackResolver>,
        engine: Box<dyn LocalAudioEngine>,
        events: SharedPlaybackEventSink,
        clock: Arc<dyn PlaybackClock>,
        sequence: Arc<ProcessSequence>,
        capabilities: SourceCapabilities,
        backend: Box<dyn SystemMediaBackend>,
    ) -> Result<Self, ApiError> {
        let system_source = SystemMediaSource::new(
            backend,
            Arc::clone(&events),
            Arc::clone(&clock),
            Arc::clone(&sequence),
        )?;
        Self::new_with_system_source(
            resolver,
            engine,
            events,
            clock,
            sequence,
            capabilities,
            system_source,
        )
    }

    fn new_with_system_source(
        resolver: Arc<dyn LocalTrackResolver>,
        engine: Box<dyn LocalAudioEngine>,
        events: SharedPlaybackEventSink,
        clock: Arc<dyn PlaybackClock>,
        sequence: Arc<ProcessSequence>,
        capabilities: SourceCapabilities,
        system_source: SystemMediaSource,
    ) -> Result<Self, ApiError> {
        if !capabilities.set_queue {
            return Err(crate::ipc::ApiError::from_reason(
                crate::ipc::InternalReason::RequestInvalid,
            ));
        }
        let local_source = SourceSummary::new(
            LOCAL_SOURCE_ID,
            SourceKind::Local,
            LOCAL_SOURCE_NAME,
            true,
            capabilities,
        )?;
        let sender = spawn_actor(
            local_source.source_id.clone(),
            capabilities,
            resolver,
            engine,
            events,
            clock,
            sequence,
        )?;
        Ok(Self {
            local_client: Arc::new(PlaybackClient { sender }),
            local_source,
            system_source,
            active_source: Arc::new(Mutex::new(ActiveSource::Local)),
            select_requests: Arc::new(AsyncIdempotency::new(IDEMPOTENCY_CAPACITY)),
            control_requests: Arc::new(AsyncIdempotency::new(IDEMPOTENCY_CAPACITY)),
            accepting_sound: Arc::new(AtomicBool::new(true)),
        })
    }

    #[must_use]
    pub fn local_capabilities() -> SourceCapabilities {
        SourceCapabilities {
            play: true,
            pause: true,
            seek: true,
            next: true,
            previous: true,
            set_queue: true,
        }
    }

    #[must_use]
    pub fn apple_music_app_installed(&self) -> Option<bool> {
        apple_music_app_installed()
    }

    /// Applies the persisted v1 default without issuing a media command or
    /// starting audio. An unavailable Apple session remains the selected,
    /// disconnected source so the UI can show the recovery guidance instead
    /// of silently falling back to local playback.
    ///
    /// # Errors
    ///
    /// Returns a storage-integrity error for a persisted source outside the
    /// two v1 adapters.
    pub(crate) fn apply_initial_source(&self, source_id: Option<&str>) -> Result<(), ApiError> {
        match source_id {
            None | Some(LOCAL_SOURCE_ID) => self.set_active(ActiveSource::Local),
            Some(APPLE_SOURCE_ID) => self.set_active(ActiveSource::System),
            Some(_) => Err(ApiError::from_reason(
                InternalReason::StorageIntegrityFailed,
            )),
        }
    }

    /// API-016. This is a pure in-memory read.
    #[must_use]
    pub fn list_music_sources(&self, _request: EmptyRequest) -> ListMusicSourcesResponse {
        ListMusicSourcesResponse {
            sources: vec![self.local_source.clone(), self.system_source.summary()],
        }
    }

    /// API-017 with a completion-based ten-minute idempotency window.
    ///
    /// # Errors
    ///
    /// Returns a safe source, actor, or idempotency error.
    pub async fn select_music_source(
        &self,
        request: SelectMusicSourceRequest,
    ) -> Result<SelectMusicSourceResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.select_requests
            .execute(
                "api_v1_select_music_source",
                request_id,
                hash,
                STANDARD_RETENTION,
                || async {
                    let state = match request.source_id.as_str() {
                        LOCAL_SOURCE_ID => {
                            let state = self
                                .request_local_state(|response| ActorMessage::SelectSource {
                                    source_id: request.source_id,
                                    response,
                                })
                                .await?;
                            self.set_active(ActiveSource::Local)?;
                            self.system_source.deactivate().await?;
                            state
                        }
                        APPLE_SOURCE_ID => {
                            let state = self.system_source.select().await?;
                            self.set_active(ActiveSource::System)?;
                            state
                        }
                        _ => {
                            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
                        }
                    };
                    Ok(SelectMusicSourceResponse { request_id, state })
                },
            )
            .await
    }

    /// API-018 authoritative snapshot.
    ///
    /// # Errors
    ///
    /// Returns a safe actor/source error.
    pub async fn get_playback_state(
        &self,
        _request: EmptyRequest,
    ) -> Result<PlaybackState, ApiError> {
        match self.active()? {
            ActiveSource::Local => {
                self.request_local_state(|response| ActorMessage::GetState { response })
                    .await
            }
            ActiveSource::System => self.system_source.get_state().await,
        }
    }

    /// API-019.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn play(&self, request: PlaybackControlRequest) -> Result<PlaybackState, ApiError> {
        self.control_active("api_v1_play", &request, ControlAction::Play)
            .await
    }

    /// API-020.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn pause(&self, request: PlaybackControlRequest) -> Result<PlaybackState, ApiError> {
        self.control_active("api_v1_pause", &request, ControlAction::Pause)
            .await
    }

    /// API-021.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn seek(&self, request: SeekPlaybackRequest) -> Result<PlaybackState, ApiError> {
        self.control_active(
            "api_v1_seek",
            &request,
            ControlAction::Seek(request.position_ms),
        )
        .await
    }

    /// API-022.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn next(&self, request: PlaybackControlRequest) -> Result<PlaybackState, ApiError> {
        self.control_active("api_v1_next", &request, ControlAction::Next)
            .await
    }

    /// API-023.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn previous(
        &self,
        request: PlaybackControlRequest,
    ) -> Result<PlaybackState, ApiError> {
        self.control_active("api_v1_previous", &request, ControlAction::Previous)
            .await
    }

    /// Internal local-source queue activation for the program runner. Passing
    /// no authorization always loads paused. Only an explicit manual or
    /// confirmed-notification authorization permits autoplay.
    ///
    /// # Errors
    ///
    /// Returns a safe queue validation, source, media, output, or actor error.
    pub async fn set_local_queue(
        &self,
        track_ids: Vec<String>,
        authorization: Option<PlaybackStartAuthorization>,
    ) -> Result<PlaybackState, ApiError> {
        self.ensure_sound_admission()?;
        self.set_active(ActiveSource::Local)?;
        self.system_source.deactivate().await?;
        self.request_local_state(|response| ActorMessage::SetQueue {
            track_ids,
            authorization,
            response,
        })
        .await
    }

    /// Accepts a path-free callback from the current output session.
    ///
    /// # Errors
    ///
    /// Returns an actor-availability error. Stale session events are accepted
    /// as no-ops by the actor.
    pub async fn handle_engine_event(&self, event: LocalAudioEngineEvent) -> Result<(), ApiError> {
        self.local_client
            .sender
            .send(ActorMessage::EngineEvent(event))
            .await
            .map_err(|_| ApiError::unexpected())
    }

    /// Checkpoints and silences the local output before OS suspend.
    ///
    /// # Errors
    ///
    /// Returns a safe media, output, or actor-availability error.
    pub async fn prepare_suspend(&self) -> Result<PlaybackState, ApiError> {
        self.begin_suspend();
        self.system_source.suspend().await?;
        self.request_local_state(|response| ActorMessage::Suspend { response })
            .await
    }

    pub(crate) fn begin_suspend(&self) {
        self.accepting_sound.store(false, Ordering::Release);
        self.system_source.begin_suspend();
        let (response, _receiver) = oneshot::channel();
        let _ = self
            .local_client
            .sender
            .try_send(ActorMessage::Suspend { response });
    }

    pub(crate) fn resume_after_suspend(&self) {
        self.system_source.resume_after_suspend();
        self.accepting_sound.store(true, Ordering::Release);
    }

    /// Rebuilds a suspended/device-lost output only in paused state.
    ///
    /// # Errors
    ///
    /// Returns a safe source, media, output, or actor-availability error.
    pub async fn resume_silent(&self) -> Result<PlaybackState, ApiError> {
        let local = self
            .request_local_state(|response| ActorMessage::ResumeSilent { response })
            .await?;
        match self.active()? {
            ActiveSource::Local => Ok(local),
            // Refreshing a selected system source reads a new authoritative
            // snapshot but deliberately sends no media command.
            ActiveSource::System => self.system_source.get_state().await,
        }
    }

    /// Stops only CyberKindred-owned local output and clears its queue.
    ///
    /// # Errors
    ///
    /// Returns a safe actor-availability error.
    pub async fn stop_local(&self) -> Result<PlaybackState, ApiError> {
        self.request_local_state(|response| ActorMessage::Stop { response })
            .await
    }

    /// Performs an orderly actor shutdown and waits for output release.
    ///
    /// # Errors
    ///
    /// Returns a safe actor-availability error.
    pub async fn shutdown(&self) -> Result<(), ApiError> {
        self.system_source.deactivate().await?;
        let (response, receiver) = oneshot::channel();
        self.local_client
            .sender
            .send(ActorMessage::Shutdown { response })
            .await
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())
    }

    pub(crate) async fn begin_system_interruption(
        &self,
    ) -> Result<SystemInterruptionToken, ApiError> {
        self.ensure_sound_admission()?;
        if self.active()? != ActiveSource::System {
            return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
        }
        self.system_source.begin_interruption().await
    }

    pub(crate) async fn finish_system_interruption(
        &self,
        token: SystemInterruptionToken,
    ) -> Result<PlaybackState, ApiError> {
        if !self.accepting_sound.load(Ordering::Acquire) {
            return self.system_source.suspend().await;
        }
        if self.active()? != ActiveSource::System {
            return self.system_source.deactivate().await;
        }
        self.system_source.finish_interruption(token).await
    }

    async fn control_active<R>(
        &self,
        command: &'static str,
        request: &R,
        action: ControlAction,
    ) -> Result<PlaybackState, ApiError>
    where
        R: serde::Serialize + ControlRequest,
    {
        let hash = canonical_request_hash(request)?;
        self.control_requests
            .execute(
                command,
                request.request_id(),
                hash,
                MEDIA_RETENTION,
                || async {
                    self.ensure_sound_admission()?;
                    match self.active()? {
                        ActiveSource::Local => {
                            self.control_local(request.expected_revision(), action)
                                .await
                        }
                        ActiveSource::System => {
                            self.system_source
                                .control(request.expected_revision(), system_action(action))
                                .await
                        }
                    }
                },
            )
            .await
    }

    fn ensure_sound_admission(&self) -> Result<(), ApiError> {
        if self.accepting_sound.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ApiError::from_reason(InternalReason::ResourceBusy))
        }
    }

    async fn control_local(
        &self,
        expected_revision: u64,
        action: ControlAction,
    ) -> Result<PlaybackState, ApiError> {
        self.request_local_state(|response| match action {
            ControlAction::Play => ActorMessage::Play {
                expected_revision,
                response,
            },
            ControlAction::Pause => ActorMessage::Pause {
                expected_revision,
                response,
            },
            ControlAction::Seek(position_ms) => ActorMessage::Seek {
                expected_revision,
                position_ms,
                response,
            },
            ControlAction::Next => ActorMessage::Next {
                expected_revision,
                response,
            },
            ControlAction::Previous => ActorMessage::Previous {
                expected_revision,
                response,
            },
        })
        .await
    }

    fn active(&self) -> Result<ActiveSource, ApiError> {
        self.active_source
            .lock()
            .map(|active| *active)
            .map_err(|_| ApiError::unexpected())
    }

    fn set_active(&self, source: ActiveSource) -> Result<(), ApiError> {
        let mut active = self
            .active_source
            .lock()
            .map_err(|_| ApiError::unexpected())?;
        *active = source;
        Ok(())
    }

    async fn request_local_state<F>(&self, message: F) -> Result<PlaybackState, ApiError>
    where
        F: FnOnce(oneshot::Sender<Result<PlaybackState, ApiError>>) -> ActorMessage,
    {
        let (response, receiver) = oneshot::channel();
        self.local_client
            .sender
            .send(message(response))
            .await
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())?
    }
}

trait ControlRequest {
    fn request_id(&self) -> uuid::Uuid;
    fn expected_revision(&self) -> u64;
}

impl ControlRequest for PlaybackControlRequest {
    fn request_id(&self) -> uuid::Uuid {
        self.client_request_id
    }

    fn expected_revision(&self) -> u64 {
        self.expected_state_revision
    }
}

impl ControlRequest for SeekPlaybackRequest {
    fn request_id(&self) -> uuid::Uuid {
        self.client_request_id
    }

    fn expected_revision(&self) -> u64 {
        self.expected_state_revision
    }
}

const fn system_action(action: ControlAction) -> SystemMediaControl {
    match action {
        ControlAction::Play => SystemMediaControl::Play,
        ControlAction::Pause => SystemMediaControl::Pause,
        ControlAction::Seek(position_ms) => SystemMediaControl::Seek(position_ms),
        ControlAction::Next => SystemMediaControl::Next,
        ControlAction::Previous => SystemMediaControl::Previous,
    }
}
