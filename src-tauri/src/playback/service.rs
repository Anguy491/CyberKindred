use std::{sync::Arc, time::Duration};

use chrono::{SecondsFormat, Utc};
use tokio::sync::{mpsc, oneshot};

use crate::{
    contracts::PlaybackState,
    ipc::{
        ApiError, EmptyRequest, ProcessSequence, SourceCapabilities, SourceKind, SourceSummary,
        canonical_request_hash,
    },
};

use super::{
    actor::{ActorMessage, spawn_actor},
    dto::{
        ListMusicSourcesResponse, PlaybackControlRequest, SeekPlaybackRequest,
        SelectMusicSourceRequest, SelectMusicSourceResponse,
    },
    engine::{
        LocalAudioEngine, LocalAudioEngineEvent, LocalTrackResolver, PlaybackStartAuthorization,
    },
    events::SharedPlaybackEventSink,
    idempotency::AsyncIdempotency,
};

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

impl Drop for PlaybackClient {
    fn drop(&mut self) {
        let (response, _receiver) = oneshot::channel();
        let _ = self.sender.try_send(ActorMessage::Shutdown { response });
    }
}

#[derive(Clone)]
pub struct PlaybackService {
    client: Arc<PlaybackClient>,
    source: SourceSummary,
    select_requests: Arc<AsyncIdempotency<SelectMusicSourceResponse>>,
    control_requests: Arc<AsyncIdempotency<PlaybackState>>,
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
    ) -> Result<Self, ApiError> {
        if !capabilities.set_queue {
            return Err(crate::ipc::ApiError::from_reason(
                crate::ipc::InternalReason::RequestInvalid,
            ));
        }
        let source = SourceSummary::new(
            LOCAL_SOURCE_ID,
            SourceKind::Local,
            LOCAL_SOURCE_NAME,
            true,
            capabilities,
        )?;
        let sender = spawn_actor(
            source.source_id.clone(),
            capabilities,
            resolver,
            engine,
            events,
            clock,
            sequence,
        )?;
        Ok(Self {
            client: Arc::new(PlaybackClient { sender }),
            source,
            select_requests: Arc::new(AsyncIdempotency::new(IDEMPOTENCY_CAPACITY)),
            control_requests: Arc::new(AsyncIdempotency::new(IDEMPOTENCY_CAPACITY)),
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

    /// API-016. This is a pure in-memory read.
    #[must_use]
    pub fn list_music_sources(&self, _request: EmptyRequest) -> ListMusicSourcesResponse {
        ListMusicSourcesResponse {
            sources: vec![self.source.clone()],
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
                    let state = self
                        .request_state(|response| ActorMessage::SelectSource {
                            source_id: request.source_id,
                            response,
                        })
                        .await?;
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
        self.request_state(|response| ActorMessage::GetState { response })
            .await
    }

    /// API-019.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn play(&self, request: PlaybackControlRequest) -> Result<PlaybackState, ApiError> {
        self.control(
            "api_v1_play",
            request.client_request_id,
            &request,
            |response| ActorMessage::Play {
                expected_revision: request.expected_state_revision,
                response,
            },
        )
        .await
    }

    /// API-020.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn pause(&self, request: PlaybackControlRequest) -> Result<PlaybackState, ApiError> {
        self.control(
            "api_v1_pause",
            request.client_request_id,
            &request,
            |response| ActorMessage::Pause {
                expected_revision: request.expected_state_revision,
                response,
            },
        )
        .await
    }

    /// API-021.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn seek(&self, request: SeekPlaybackRequest) -> Result<PlaybackState, ApiError> {
        self.control(
            "api_v1_seek",
            request.client_request_id,
            &request,
            |response| ActorMessage::Seek {
                expected_revision: request.expected_state_revision,
                position_ms: request.position_ms,
                response,
            },
        )
        .await
    }

    /// API-022.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, capability, source, or actor error.
    pub async fn next(&self, request: PlaybackControlRequest) -> Result<PlaybackState, ApiError> {
        self.control(
            "api_v1_next",
            request.client_request_id,
            &request,
            |response| ActorMessage::Next {
                expected_revision: request.expected_state_revision,
                response,
            },
        )
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
        self.control(
            "api_v1_previous",
            request.client_request_id,
            &request,
            |response| ActorMessage::Previous {
                expected_revision: request.expected_state_revision,
                response,
            },
        )
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
        self.request_state(|response| ActorMessage::SetQueue {
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
        self.client
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
        self.request_state(|response| ActorMessage::Suspend { response })
            .await
    }

    /// Rebuilds a suspended/device-lost output only in paused state.
    ///
    /// # Errors
    ///
    /// Returns a safe source, media, output, or actor-availability error.
    pub async fn resume_silent(&self) -> Result<PlaybackState, ApiError> {
        self.request_state(|response| ActorMessage::ResumeSilent { response })
            .await
    }

    /// Stops only CyberKindred-owned local output and clears its queue.
    ///
    /// # Errors
    ///
    /// Returns a safe actor-availability error.
    pub async fn stop_local(&self) -> Result<PlaybackState, ApiError> {
        self.request_state(|response| ActorMessage::Stop { response })
            .await
    }

    /// Performs an orderly actor shutdown and waits for output release.
    ///
    /// # Errors
    ///
    /// Returns a safe actor-availability error.
    pub async fn shutdown(&self) -> Result<(), ApiError> {
        let (response, receiver) = oneshot::channel();
        self.client
            .sender
            .send(ActorMessage::Shutdown { response })
            .await
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())
    }

    async fn control<R, F>(
        &self,
        command: &'static str,
        request_id: uuid::Uuid,
        request: &R,
        message: F,
    ) -> Result<PlaybackState, ApiError>
    where
        R: serde::Serialize,
        F: FnOnce(oneshot::Sender<Result<PlaybackState, ApiError>>) -> ActorMessage,
    {
        let hash = canonical_request_hash(request)?;
        self.control_requests
            .execute(command, request_id, hash, MEDIA_RETENTION, || async {
                self.request_state(message).await
            })
            .await
    }

    async fn request_state<F>(&self, message: F) -> Result<PlaybackState, ApiError>
    where
        F: FnOnce(oneshot::Sender<Result<PlaybackState, ApiError>>) -> ActorMessage,
    {
        let (response, receiver) = oneshot::channel();
        self.client
            .sender
            .send(message(response))
            .await
            .map_err(|_| ApiError::unexpected())?;
        receiver.await.map_err(|_| ApiError::unexpected())?
    }
}
