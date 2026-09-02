use std::{collections::HashMap, sync::Arc};

use tokio::sync::{broadcast, mpsc, watch};
use uuid::Uuid;

use crate::{
    contracts::{PlaybackEvent, PlaybackState, PlaybackStateStatus},
    ipc::{ApiError, InternalReason},
    playback::{PlaybackEventSink, PlaybackService},
    playback_repository::RepositoryTrackResolver,
    storage::{StorageError, StorageReason},
};

use super::{ConfirmedProgramStart, ProgramPlayback, ProgramPlaybackSignal, traits::RadioFuture};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramTrack {
    pub segment_id: Uuid,
    pub track_id: String,
}

/// Forwards each EVT-001 once and also exposes a bounded cloned observation
/// stream to the local program runner.
#[derive(Clone)]
pub struct PlaybackEventHub {
    downstream: Arc<dyn PlaybackEventSink>,
    sender: broadcast::Sender<PlaybackEvent>,
}

impl PlaybackEventHub {
    #[must_use]
    pub fn new(downstream: Arc<dyn PlaybackEventSink>, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.clamp(16, 1_024));
        Self { downstream, sender }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<PlaybackEvent> {
        self.sender.subscribe()
    }
}

impl PlaybackEventSink for PlaybackEventHub {
    fn publish(&self, event: PlaybackEvent) -> Result<(), ApiError> {
        let downstream_result = self.downstream.publish(event.clone());
        let _ = self.sender.send(event);
        downstream_result
    }
}

/// Production local playback adapter. Main must construct `PlaybackService`
/// with the same resolver and this hub as its EVT-001 sink.
pub struct LocalProgramPlayback {
    service: PlaybackService,
    resolver: RepositoryTrackResolver,
    events: PlaybackEventHub,
}

impl LocalProgramPlayback {
    #[must_use]
    pub(crate) fn new(
        service: PlaybackService,
        resolver: RepositoryTrackResolver,
        events: PlaybackEventHub,
    ) -> Self {
        Self {
            service,
            resolver,
            events,
        }
    }

    async fn play_batch_inner(
        &self,
        tracks: &[ProgramTrack],
        authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<ProgramPlaybackSignal>,
    ) -> Result<(), ApiError> {
        validate_batch(tracks)?;
        if *cancellation.borrow() {
            return Err(cancelled());
        }
        let mut events = self.events.subscribe();
        let track_ids = tracks
            .iter()
            .map(|track| track.track_id.clone())
            .collect::<Vec<_>>();
        self.resolver
            .refresh(&track_ids)
            .await
            .map_err(|error| map_storage_error(&error))?;
        let initial = match self
            .service
            .set_local_queue(track_ids, Some(authorization.playback()))
            .await
        {
            Ok(state) => state,
            Err(error) => {
                self.resolver.clear();
                return Err(error);
            }
        };
        let lookup = tracks
            .iter()
            .map(|track| (track.track_id.as_str(), track.segment_id))
            .collect::<HashMap<_, _>>();
        let mut current = None;
        let mut paused = false;
        apply_playback_state(&initial, &lookup, &signals, &mut current).await?;
        apply_playback_status(&initial, &signals, &mut paused).await?;
        loop {
            tokio::select! {
                changed = cancellation.changed() => {
                    if changed.is_err() || *cancellation.borrow() {
                        let _ = self.service.stop_local().await;
                        self.resolver.clear();
                        return Err(cancelled());
                    }
                }
                event = events.recv() => {
                    let event = event.map_err(|_| ApiError::unexpected())?;
                    let Some(state) = event.state else {
                        continue;
                    };
                    if state.status == PlaybackStateStatus::Error {
                        if let Some(segment_id) = current.take() {
                            send_signal(&signals, ProgramPlaybackSignal::Failed(segment_id)).await?;
                        }
                        let _ = self.service.stop_local().await;
                        self.resolver.clear();
                        return Err(ApiError::from_reason(InternalReason::SourceUnavailable));
                    }
                    apply_playback_state(&state, &lookup, &signals, &mut current).await?;
                    apply_playback_status(&state, &signals, &mut paused).await?;
                    if matches!(state.status, PlaybackStateStatus::Idle | PlaybackStateStatus::Stopped)
                        && state.current_track.is_none()
                    {
                        self.resolver.clear();
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn apply_playback_status(
    state: &PlaybackState,
    signals: &mpsc::Sender<ProgramPlaybackSignal>,
    paused: &mut bool,
) -> Result<(), ApiError> {
    match state.status {
        PlaybackStateStatus::Paused if !*paused => {
            send_signal(signals, ProgramPlaybackSignal::Paused).await?;
            *paused = true;
        }
        PlaybackStateStatus::Playing if *paused => {
            send_signal(signals, ProgramPlaybackSignal::Resumed).await?;
            *paused = false;
        }
        _ => {}
    }
    Ok(())
}

impl ProgramPlayback for LocalProgramPlayback {
    fn play_batch<'a>(
        &'a self,
        _program_id: Uuid,
        tracks: &'a [ProgramTrack],
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<ProgramPlaybackSignal>,
    ) -> RadioFuture<'a, Result<(), ApiError>> {
        Box::pin(self.play_batch_inner(tracks, authorization, cancellation, signals))
    }

    fn stop(&self) -> RadioFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let result = self.service.stop_local().await.map(|_| ());
            self.resolver.clear();
            result
        })
    }
}

async fn apply_playback_state(
    state: &PlaybackState,
    lookup: &HashMap<&str, Uuid>,
    signals: &mpsc::Sender<ProgramPlaybackSignal>,
    current: &mut Option<Uuid>,
) -> Result<(), ApiError> {
    let next = match &state.current_track {
        Some(track) => Some(
            lookup
                .get(track.track_id.as_str())
                .copied()
                .ok_or_else(ApiError::unexpected)?,
        ),
        None => None,
    };
    if next == *current {
        return Ok(());
    }
    if let Some(segment_id) = current.take() {
        send_signal(signals, ProgramPlaybackSignal::Completed(segment_id)).await?;
    }
    if let Some(segment_id) = next {
        send_signal(signals, ProgramPlaybackSignal::Started(segment_id)).await?;
        *current = Some(segment_id);
    }
    Ok(())
}

async fn send_signal(
    sender: &mpsc::Sender<ProgramPlaybackSignal>,
    signal: ProgramPlaybackSignal,
) -> Result<(), ApiError> {
    sender
        .send(signal)
        .await
        .map_err(|_| ApiError::unexpected())
}

fn validate_batch(tracks: &[ProgramTrack]) -> Result<(), ApiError> {
    if !(1..=3).contains(&tracks.len())
        || tracks.iter().any(|track| {
            track.segment_id.get_version_num() != 7
                || !Uuid::parse_str(&track.track_id).is_ok_and(|track_id| {
                    track_id.get_version_num() == 7 && track_id.to_string() == track.track_id
                })
        })
    {
        return Err(ApiError::from_reason(InternalReason::RequestInvalid));
    }
    Ok(())
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        _ => InternalReason::StorageWriteFailed,
    };
    ApiError::from_reason(reason)
}

fn cancelled() -> ApiError {
    ApiError::from_reason(InternalReason::OperationCancelled)
}
