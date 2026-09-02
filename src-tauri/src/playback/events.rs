use std::sync::Arc;

use tauri::{Emitter, Runtime};

use crate::{
    contracts::PlaybackEvent,
    ipc::{ApiError, InternalReason},
};

pub const PLAYBACK_EVENT: &str = "cyberkindred://v1/playback/event";

/// At-most-once playback transport. Publish failures are never retried; UI
/// consumers recover from sequence gaps through API-018.
pub trait PlaybackEventSink: Send + Sync + 'static {
    /// Attempts one transport delivery; callers never retry a failed publish.
    ///
    /// # Errors
    ///
    /// Returns a transport-safe error without changing authoritative state.
    fn publish(&self, event: PlaybackEvent) -> Result<(), ApiError>;
}

pub struct TauriPlaybackEventSink<R: Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: Runtime> TauriPlaybackEventSink<R> {
    #[must_use]
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> PlaybackEventSink for TauriPlaybackEventSink<R> {
    fn publish(&self, event: PlaybackEvent) -> Result<(), ApiError> {
        self.app
            .emit(PLAYBACK_EVENT, event)
            .map_err(|_| ApiError::from_reason(InternalReason::UnexpectedInternal))
    }
}

pub(super) type SharedPlaybackEventSink = Arc<dyn PlaybackEventSink>;
