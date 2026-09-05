//! Serialized local playback and the API-016..023 command surface.

mod actor;
mod artwork;
pub mod commands;
mod dto;
mod engine;
mod events;
mod idempotency;
mod output;
mod service;
mod system_media;

pub use artwork::ArtworkAssetStore;
pub use dto::{
    ListMusicSourcesResponse, PlaybackControlRequest, SeekPlaybackRequest,
    SelectMusicSourceRequest, SelectMusicSourceResponse,
};
pub use engine::{
    AudioEngineError, AudioSnapshot, LocalAudioEngine, LocalAudioEngineEvent,
    LocalAudioEngineEventSink, LocalSourceError, LocalTrack, LocalTrackResolver,
    PlaybackStartAuthorization,
};
pub use events::{PLAYBACK_EVENT, PlaybackEventSink, TauriPlaybackEventSink};
pub use output::RodioAudioEngine;
pub use service::{PlaybackClock, PlaybackService, SystemPlaybackClock};

#[cfg(test)]
mod tests;
