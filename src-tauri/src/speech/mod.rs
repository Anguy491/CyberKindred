//! Bounded speech synthesis and playback coordination.
//!
//! Construction is side-effect free. Network and audio work occur only after
//! an explicit actor run with TTS enabled, and both boundaries are injectable.

mod actor;
mod cache;
mod playback;
mod preview;
mod provider;

pub use actor::{
    SpeechActor, SpeechAuthorization, SpeechCancelState, SpeechFallbackReason,
    SpeechOperationState, SpeechPlayback, SpeechRunOutcome,
};
pub use cache::{SpeechCache, SpeechCacheMetadata, SpeechLease, SpeechOwner};
pub use playback::RodioSpeechPlayback;
pub use preview::SpeechVoicePreviewer;
pub(crate) use provider::CallScopedOpenAiTtsProvider;
pub use provider::{
    OpenAiTtsProvider, ReqwestSpeechTransport, ReqwestSpeechTransportFactory,
    SpeechCredentialSource, SpeechSink, SpeechTransport, SpeechTransportFactory,
    SpeechTransportRequest, SpeechTransportResponse, TtsProvider,
};

use crate::providers::{ProviderFailure, ProviderFailureCategory};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const VOICE_PREVIEW_TEXT_V1: &str = "你好，我是 CyberKindred，很高兴陪你听一会儿。";
pub(crate) const MAX_SPEECH_BYTES: usize = 20 * 1_024 * 1_024;
pub(crate) const SPEECH_CACHE_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

pub(crate) fn invalid_response() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::InvalidResponse)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Mp3,
}

impl AudioFormat {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechProvenance {
    LocalProgram,
    Chat,
    VoicePreview,
    SystemSessionGeneric,
}

/// Validated, normalized input for the closed `OpenAI` Speech request.
#[derive(Clone)]
pub struct SpeechInput {
    text: String,
    voice_id: String,
    model_id: String,
    format: AudioFormat,
    speed_millis: u16,
    locale: String,
    provenance: SpeechProvenance,
}

impl SpeechInput {
    /// Creates a local-program or chat segment.
    ///
    /// # Errors
    ///
    /// Rejects empty/oversized text, unsafe identifiers, invalid speed/locale,
    /// and provenance values that require a dedicated trusted constructor.
    pub fn segment(
        text: impl AsRef<str>,
        voice_id: String,
        model_id: String,
        speed: f32,
        locale: String,
        provenance: SpeechProvenance,
    ) -> Result<Self, ProviderFailure> {
        if !matches!(
            provenance,
            SpeechProvenance::LocalProgram | SpeechProvenance::Chat
        ) {
            return Err(invalid_response());
        }
        Self::validated(text.as_ref(), voice_id, model_id, speed, locale, provenance)
    }

    /// Creates the versioned fixed preview phrase. No caller text is accepted.
    ///
    /// # Errors
    ///
    /// Rejects unsafe identifiers, speed, or locale.
    pub fn voice_preview(
        voice_id: String,
        model_id: String,
        speed: f32,
        locale: String,
    ) -> Result<Self, ProviderFailure> {
        Self::validated(
            VOICE_PREVIEW_TEXT_V1,
            voice_id,
            model_id,
            speed,
            locale,
            SpeechProvenance::VoicePreview,
        )
    }

    fn validated(
        text: &str,
        voice_id: String,
        model_id: String,
        speed: f32,
        locale: String,
        provenance: SpeechProvenance,
    ) -> Result<Self, ProviderFailure> {
        let text = normalize_text(text)?;
        let speed_scaled = f64::from(speed) * 1_000.0;
        if !speed.is_finite()
            || !(750.0..=1_250.0).contains(&speed_scaled)
            || (speed_scaled.round() - speed_scaled).abs() > 0.001
            || !safe_identifier(&voice_id)
            || !safe_identifier(&model_id)
            || !safe_locale(&locale)
        {
            return Err(invalid_response());
        }
        let speed_millis = format!("{speed_scaled:.0}")
            .parse::<u16>()
            .map_err(|_| invalid_response())?;
        Ok(Self {
            text,
            voice_id,
            model_id,
            format: AudioFormat::Mp3,
            speed_millis,
            locale,
            provenance,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn voice_id(&self) -> &str {
        &self.voice_id
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    #[must_use]
    pub const fn format(&self) -> AudioFormat {
        self.format
    }

    #[must_use]
    pub fn speed(&self) -> f32 {
        f32::from(self.speed_millis) / 1_000.0
    }

    #[must_use]
    pub fn locale(&self) -> &str {
        &self.locale
    }

    #[must_use]
    pub const fn provenance(&self) -> SpeechProvenance {
        self.provenance
    }

    #[must_use]
    pub fn cache_key(&self) -> SpeechCacheKey {
        let mut hasher = Sha256::new();
        let speed_millis = self.speed_millis.to_string();
        for value in [
            self.model_id.as_bytes(),
            self.voice_id.as_bytes(),
            speed_millis.as_bytes(),
            self.format.as_str().as_bytes(),
            self.locale.as_bytes(),
            self.text.as_bytes(),
        ] {
            hasher.update(value.len().to_string().as_bytes());
            hasher.update([0]);
            hasher.update(value);
        }
        SpeechCacheKey(hasher.finalize().into())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SpeechCacheKey([u8; 32]);

impl SpeechCacheKey {
    #[must_use]
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechArtifact {
    pub content_hash: String,
    pub format: AudioFormat,
    pub byte_length: u64,
}

impl SpeechArtifact {
    pub(crate) fn new(input: &SpeechInput, byte_length: usize) -> Result<Self, ProviderFailure> {
        if byte_length == 0 || byte_length > MAX_SPEECH_BYTES {
            return Err(invalid_response());
        }
        Ok(Self {
            content_hash: input.cache_key().to_hex(),
            format: input.format(),
            byte_length: u64::try_from(byte_length).map_err(|_| invalid_response())?,
        })
    }
}

fn normalize_text(value: &str) -> Result<String, ProviderFailure> {
    if value
        .chars()
        .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err(invalid_response());
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if !(1..=500).contains(&normalized.chars().count()) {
        return Err(invalid_response());
    }
    Ok(normalized)
}

fn safe_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn safe_locale(value: &str) -> bool {
    (2..=35).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
}

pub(crate) fn valid_operation_id(value: Uuid) -> bool {
    !value.is_nil()
}

#[cfg(test)]
mod tests;
