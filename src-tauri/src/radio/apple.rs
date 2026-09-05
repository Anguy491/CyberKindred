use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::{
    contracts::{PlaybackState, PlaybackStateStatus},
    ipc::{ApiError, EmptyRequest, InternalReason},
    playback::{PlaybackService, SelectMusicSourceRequest},
};

use super::{ConfirmedProgramStart, PlaybackEventHub, ProgramSpeechOutcome, traits::RadioFuture};

const APPLE_SOURCE_ID: &str = "apple_music";
const DISCONNECTED_MESSAGE: &str =
    "Apple Music 会话已断开；陪伴模式保持静音并等待 Windows App 会话恢复。";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppleCompanionSignal {
    Reaction(String),
    Disconnected,
}

pub trait SystemProgramSpeech: Send + Sync {
    fn present_opening(
        &self,
        program_id: Uuid,
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
    ) -> RadioFuture<'_, Result<ProgramSpeechOutcome, ApiError>>;

    fn cancel(&self, program_id: Uuid);
}

pub trait AppleCompanion: Send + Sync {
    fn connect(&self) -> RadioFuture<'_, Result<PlaybackState, ApiError>>;

    fn run(
        &self,
        program_id: Uuid,
        initial: PlaybackState,
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> RadioFuture<'_, Result<(), ApiError>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableAppleCompanion;

impl AppleCompanion for UnavailableAppleCompanion {
    fn connect(&self) -> RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async { Err(ApiError::from_reason(InternalReason::SourceUnavailable)) })
    }

    fn run(
        &self,
        _program_id: Uuid,
        _initial: PlaybackState,
        _authorization: ConfirmedProgramStart,
        _cancellation: watch::Receiver<bool>,
        _signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async { Err(ApiError::from_reason(InternalReason::SourceUnavailable)) })
    }
}

pub struct AppleCompanionMonitor {
    playback: PlaybackService,
    events: PlaybackEventHub,
    speech: Arc<dyn SystemProgramSpeech>,
}

impl AppleCompanionMonitor {
    #[must_use]
    pub fn new(
        playback: PlaybackService,
        events: PlaybackEventHub,
        speech: Arc<dyn SystemProgramSpeech>,
    ) -> Self {
        Self {
            playback,
            events,
            speech,
        }
    }
}

impl AppleCompanion for AppleCompanionMonitor {
    fn connect(&self) -> RadioFuture<'_, Result<PlaybackState, ApiError>> {
        Box::pin(async move {
            self.playback
                .select_music_source(SelectMusicSourceRequest {
                    client_request_id: Uuid::now_v7(),
                    source_id: APPLE_SOURCE_ID.to_owned(),
                })
                .await
                .map(|response| response.state)
        })
    }

    fn run(
        &self,
        program_id: Uuid,
        initial: PlaybackState,
        authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<AppleCompanionSignal>,
    ) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let mut events = self.events.subscribe();
            let opening =
                self.speech
                    .present_opening(program_id, authorization, cancellation.clone());
            tokio::pin!(opening);
            tokio::select! {
                result = &mut opening => { let _ = result?; }
                changed = cancellation.changed() => {
                    self.speech.cancel(program_id);
                    if changed.is_err() || *cancellation.borrow() { return Ok(()); }
                }
            }

            let reconciled = self
                .playback
                .get_playback_state(EmptyRequest {})
                .await
                .unwrap_or(initial);
            let mut last_track_id = None;
            observe(&reconciled, &signals, &mut last_track_id).await?;
            loop {
                tokio::select! {
                    changed = cancellation.changed() => {
                        if changed.is_err() || *cancellation.borrow() {
                            self.speech.cancel(program_id);
                            return Ok(());
                        }
                    }
                    event = events.recv() => {
                        let event = event.map_err(|_| ApiError::unexpected())?;
                        if event.source_id != APPLE_SOURCE_ID { continue; }
                        if let Some(state) = event.state {
                            observe(&state, &signals, &mut last_track_id).await?;
                        }
                    }
                }
            }
        })
    }
}

async fn observe(
    state: &PlaybackState,
    signals: &mpsc::Sender<AppleCompanionSignal>,
    last_track_id: &mut Option<String>,
) -> Result<(), ApiError> {
    if state.status == PlaybackStateStatus::Disconnected {
        last_track_id.take();
        return signals
            .send(AppleCompanionSignal::Disconnected)
            .await
            .map_err(|_| ApiError::unexpected());
    }
    let Some(track) = &state.current_track else {
        return Ok(());
    };
    if last_track_id.as_deref() == Some(track.track_id.as_str()) {
        return Ok(());
    }
    *last_track_id = Some(track.track_id.clone());
    let reaction = deterministic_reaction(&track.title, track.artist.as_deref());
    signals
        .send(AppleCompanionSignal::Reaction(reaction))
        .await
        .map_err(|_| ApiError::unexpected())
}

fn deterministic_reaction(title: &str, artist: Option<&str>) -> String {
    match artist {
        Some(artist) => {
            format!("正在播放《{title}》— {artist}。曲目信息来自 Apple Music；这条反应在本机生成。")
        }
        None => format!("正在播放《{title}》。曲目信息来自 Apple Music；这条反应在本机生成。"),
    }
}

#[must_use]
pub(crate) const fn disconnected_message() -> &'static str {
    DISCONNECTED_MESSAGE
}

#[cfg(test)]
mod tests {
    use super::deterministic_reaction;

    #[test]
    fn apple_program_reaction_is_deterministic_and_does_not_infer_missing_artist() {
        assert_eq!(
            deterministic_reaction("Title", None),
            "正在播放《Title》。曲目信息来自 Apple Music；这条反应在本机生成。"
        );
        assert_eq!(
            deterministic_reaction("Title", Some("Artist")),
            deterministic_reaction("Title", Some("Artist"))
        );
    }
}
