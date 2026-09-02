use super::{SpeechAuthorization, SpeechPlayback, invalid_response, valid_operation_id};
use crate::providers::{CancellationFlag, ProviderFailure, ProviderFailureCategory};
use crate::speech::provider::SpeechFuture;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

const PREVIEW_VOLUME: f32 = 0.7;
const COMPLETION_POLL: Duration = Duration::from_millis(10);

struct RodioSpeechSession {
    player: Player,
    _device_sink: MixerDeviceSink,
}

/// Safe production audio boundary for synthesized MP3 bytes.
///
/// Construction is silent. The default device is opened only inside an
/// explicitly authorized `play` call, and the operation/caller cancellation
/// flags are checked again immediately before `Player::play`.
#[derive(Default)]
pub struct RodioSpeechPlayback {
    sessions: Arc<Mutex<HashMap<Uuid, Arc<RodioSpeechSession>>>>,
}

impl RodioSpeechPlayback {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn stop_exact(sessions: &Mutex<HashMap<Uuid, Arc<RodioSpeechSession>>>, operation_id: Uuid) {
        let session = match sessions.lock() {
            Ok(mut guard) => guard.remove(&operation_id),
            Err(poisoned) => poisoned.into_inner().remove(&operation_id),
        };
        if let Some(session) = session {
            session.player.stop();
        }
    }
}

impl SpeechPlayback for RodioSpeechPlayback {
    fn play(
        &self,
        operation_id: Uuid,
        bytes: Arc<[u8]>,
        authorization: SpeechAuthorization,
        operation_cancellation: CancellationFlag,
        caller_cancellation: CancellationFlag,
    ) -> SpeechFuture<'_, Result<(), ProviderFailure>> {
        let sessions = Arc::clone(&self.sessions);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                if !authorization_allows(operation_id, authorization) {
                    return Err(invalid_response());
                }
                if operation_cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
                    return Err(unavailable());
                }
                let decoder = Decoder::new(Cursor::new(bytes)).map_err(|_| invalid_response())?;
                let builder =
                    DeviceSinkBuilder::from_default_device().map_err(|_| unavailable())?;
                let device_sink = builder.open_sink_or_fallback().map_err(|_| unavailable())?;
                let player = Player::connect_new(device_sink.mixer());
                player.pause();
                player.set_volume(PREVIEW_VOLUME);
                player.append(decoder);
                let session = Arc::new(RodioSpeechSession {
                    player,
                    _device_sink: device_sink,
                });
                {
                    let mut active = sessions.lock().map_err(|_| unavailable())?;
                    if operation_cancellation.is_cancelled()
                        || caller_cancellation.is_cancelled()
                        || active.contains_key(&operation_id)
                    {
                        return Err(unavailable());
                    }
                    active.insert(operation_id, Arc::clone(&session));
                    // This is the sole audio-start side effect. Cancellation
                    // flags are checked while holding the same ownership lock
                    // used by `stop`, so a concurrent cancel either prevents
                    // start or immediately stops this exact player.
                    if operation_cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
                        active.remove(&operation_id);
                        return Err(unavailable());
                    }
                    session.player.play();
                }
                while !session.player.empty() {
                    if operation_cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
                        Self::stop_exact(sessions.as_ref(), operation_id);
                        return Err(unavailable());
                    }
                    std::thread::sleep(COMPLETION_POLL);
                }
                let mut active = sessions.lock().map_err(|_| unavailable())?;
                if active
                    .get(&operation_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &session))
                {
                    active.remove(&operation_id);
                }
                Ok(())
            })
            .await
            .map_err(|_| unavailable())?
        })
    }

    fn stop(&self, operation_id: Uuid) {
        Self::stop_exact(self.sessions.as_ref(), operation_id);
    }
}

fn authorization_allows(operation_id: Uuid, authorization: SpeechAuthorization) -> bool {
    if !valid_operation_id(operation_id) {
        return false;
    }
    match authorization {
        SpeechAuthorization::UserRequestedPreview(owner) => owner == operation_id,
        SpeechAuthorization::ConfirmedProgram(owner)
        | SpeechAuthorization::UserSubmittedChat(owner) => valid_operation_id(owner),
    }
}

fn unavailable() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::Unavailable)
}
