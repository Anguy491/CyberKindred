use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use uuid::Uuid;

const MAX_DURATION_MS: u64 = 86_400_000;

/// One path-authorized local track. It deliberately implements neither
/// `Debug` nor serialization because it contains a canonical local path.
#[derive(Clone)]
pub struct LocalTrack {
    track_id: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    artwork_uri: Option<String>,
    canonical_path: PathBuf,
    duration_ms: u64,
}

impl LocalTrack {
    /// Builds a resolver-owned track after repository authorization and
    /// canonical containment have been checked.
    ///
    /// # Errors
    ///
    /// Returns a path-free validation error for unsafe contract fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track_id: impl Into<String>,
        title: impl Into<String>,
        artist: Option<String>,
        album: Option<String>,
        artwork_uri: Option<String>,
        canonical_path: PathBuf,
        duration_ms: u64,
    ) -> Result<Self, LocalSourceError> {
        let track_id = track_id.into();
        let title = title.into();
        if !is_safe_id(&track_id)
            || !is_safe_text(&title)
            || artist.as_deref().is_some_and(|value| !is_safe_text(value))
            || album.as_deref().is_some_and(|value| !is_safe_text(value))
            || artwork_uri
                .as_deref()
                .is_some_and(|value| !is_safe_artwork_uri(value))
            || !canonical_path.is_absolute()
            || !(1..=MAX_DURATION_MS).contains(&duration_ms)
        {
            return Err(LocalSourceError::MediaInvalid);
        }
        Ok(Self {
            track_id,
            title,
            artist,
            album,
            artwork_uri,
            canonical_path,
            duration_ms,
        })
    }

    #[must_use]
    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    #[must_use]
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }

    pub(super) fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    pub(super) fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }

    pub(super) fn artwork_uri(&self) -> Option<&str> {
        self.artwork_uri.as_deref()
    }
}

/// Resolves an opaque indexed track ID and re-checks canonical containment at
/// every load/recovery boundary. Implementations must not cache authorization.
pub trait LocalTrackResolver: Send + Sync + 'static {
    /// Resolves and reauthorizes an opaque track identifier.
    ///
    /// # Errors
    ///
    /// Returns a path-free source error when the track cannot be authorized.
    fn resolve(&self, track_id: &str) -> Result<LocalTrack, LocalSourceError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalSourceError {
    NotFound,
    PathDenied,
    MediaInvalid,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioEngineError {
    OutputUnavailable,
    MediaInvalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSnapshot {
    pub position_ms: u64,
    pub duration_ms: u64,
}

/// Minimal output boundary owned exclusively by the playback actor. A
/// production implementation may hold native handles but must never expose
/// device identifiers or paths through errors.
pub trait LocalAudioEngine: Send + 'static {
    fn set_event_sink(&mut self, sink: Arc<dyn LocalAudioEngineEventSink>);

    /// Replaces the owned output session with a validated local track.
    ///
    /// # Errors
    ///
    /// Returns a path-free media or output error.
    fn load(
        &mut self,
        track: &LocalTrack,
        session_id: Uuid,
        position_ms: u64,
        autoplay: bool,
    ) -> Result<AudioSnapshot, AudioEngineError>;

    /// Starts or resumes the current session.
    ///
    /// # Errors
    ///
    /// Returns a path-free media or output error.
    fn play(&mut self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError>;

    /// Pauses the current session.
    ///
    /// # Errors
    ///
    /// Returns a path-free media or output error.
    fn pause(&mut self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError>;

    /// Seeks the current session to a bounded position.
    ///
    /// # Errors
    ///
    /// Returns a path-free media or output error.
    fn seek(
        &mut self,
        session_id: Uuid,
        position_ms: u64,
    ) -> Result<AudioSnapshot, AudioEngineError>;

    /// Reads the current session position without changing playback intent.
    ///
    /// # Errors
    ///
    /// Returns a path-free error when the session is no longer current.
    fn snapshot(&self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError>;

    fn stop(&mut self);
}

pub trait LocalAudioEngineEventSink: Send + Sync + 'static {
    fn publish(&self, event: LocalAudioEngineEvent);
}

/// Events from the current engine session. The actor discards any event whose
/// opaque session ID is no longer current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAudioEngineEvent {
    Position {
        session_id: Uuid,
        position_ms: u64,
    },
    Ended {
        session_id: Uuid,
    },
    Failed {
        session_id: Uuid,
        error: AudioEngineError,
    },
    OutputLost {
        session_id: Uuid,
        position_ms: u64,
    },
    OutputRestored {
        recovery_session_id: Uuid,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackStartAuthorization {
    Manual,
    ConfirmedNotification,
}

fn is_safe_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn is_safe_text(value: &str) -> bool {
    let length = value.chars().count();
    (1..=300).contains(&length) && value.chars().all(|character| !character.is_control())
}

fn is_safe_artwork_uri(value: &str) -> bool {
    value.len() <= 512
        && value.strip_prefix("asset://").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
                })
        })
}
