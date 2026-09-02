use super::{
    CallScopedOpenAiTtsProvider, ReqwestSpeechTransportFactory, RodioSpeechPlayback, SpeechActor,
    SpeechArtifact, SpeechAuthorization, SpeechCache, SpeechFallbackReason, SpeechInput,
    SpeechOwner, SpeechPlayback, SpeechRunOutcome, SpeechSink, SpeechTransportFactory, TtsProvider,
    VOICE_PREVIEW_TEXT_V1, invalid_response,
};
use crate::providers::{
    ProviderCallContext, ProviderFailure, ProviderFailureCategory, ProviderFuture, SystemClock,
    VoicePreviewCancelDisposition, VoicePreviewInput, VoicePreviewer, VoiceView,
};
use std::sync::Arc;
use uuid::Uuid;

/// API-009 adapter joining the Rust-owned secret, Speech transport, cache,
/// operation ownership, and authorized audio actor without retaining the key.
pub struct SpeechVoicePreviewer {
    actor: Arc<SpeechActor>,
    transports: Arc<dyn SpeechTransportFactory>,
    voices: Vec<VoiceView>,
}

impl SpeechVoicePreviewer {
    /// Builds the production API-009 adapter without opening the network,
    /// credential store, or audio device. Side effects remain gated by the
    /// later explicit preview command and its operation authorization.
    #[must_use]
    pub fn production() -> Self {
        let actor = Arc::new(SpeechActor::new(
            Arc::new(PreviewOnlyProvider),
            SpeechCache::new(128, 64 * 1_024 * 1_024),
            Arc::new(RodioSpeechPlayback::new()) as Arc<dyn SpeechPlayback>,
            Arc::new(SystemClock),
            512,
        ));
        Self::openai_default(actor, Arc::new(ReqwestSpeechTransportFactory))
    }

    /// Creates a previewer with an explicit catalog. Construction performs no
    /// credential read, network call, or audio operation.
    ///
    /// # Errors
    ///
    /// Rejects empty/duplicate/unsafe voices and unavailable entries: this
    /// adapter advertises only voices backed by its playable Speech boundary.
    pub fn new(
        actor: Arc<SpeechActor>,
        transports: Arc<dyn SpeechTransportFactory>,
        voices: Vec<VoiceView>,
    ) -> Result<Self, ProviderFailure> {
        if voices.is_empty()
            || voices.iter().any(|voice| {
                !voice.preview_available
                    || !safe_label(&voice.voice_id, 128)
                    || !safe_label(&voice.display_name, 100)
            })
            || voices.iter().enumerate().any(|(index, voice)| {
                voices[index.saturating_add(1)..]
                    .iter()
                    .any(|other| other.voice_id == voice.voice_id)
            })
        {
            return Err(invalid_response());
        }
        Ok(Self {
            actor,
            transports,
            voices,
        })
    }

    #[must_use]
    pub fn openai_default(
        actor: Arc<SpeechActor>,
        transports: Arc<dyn SpeechTransportFactory>,
    ) -> Self {
        Self {
            actor,
            transports,
            voices: vec![VoiceView {
                voice_id: "alloy".to_owned(),
                display_name: "Alloy".to_owned(),
                preview_available: true,
            }],
        }
    }
}

struct PreviewOnlyProvider;

impl TtsProvider for PreviewOnlyProvider {
    fn synthesize<'a>(
        &'a self,
        _input: SpeechInput,
        _sink: &'a mut dyn SpeechSink,
        _context: &'a ProviderCallContext,
    ) -> super::provider::SpeechFuture<'a, Result<SpeechArtifact, ProviderFailure>> {
        Box::pin(async { Err(ProviderFailure::new(ProviderFailureCategory::Unavailable)) })
    }
}

impl VoicePreviewer for SpeechVoicePreviewer {
    fn voices(&self) -> Vec<VoiceView> {
        self.voices.clone()
    }

    fn preview<'a>(
        &'a self,
        input: VoicePreviewInput<'a>,
        context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<(), ProviderFailure>> {
        Box::pin(async move {
            if input.text != VOICE_PREVIEW_TEXT_V1
                || !self
                    .voices
                    .iter()
                    .any(|voice| voice.voice_id == input.voice_id && voice.preview_available)
            {
                return Err(invalid_response());
            }
            let transport = self.transports.for_origin(input.origin)?;
            let provider = CallScopedOpenAiTtsProvider::new(transport.as_ref(), input.secret);
            let speech = SpeechInput::voice_preview(
                input.voice_id.to_owned(),
                input.model_id.to_owned(),
                1.0,
                context.locale.to_owned(),
            )?;
            match self
                .actor
                .run_with_provider(
                    &provider,
                    input.operation_id,
                    speech,
                    SpeechOwner::Preview(input.operation_id),
                    true,
                    Some(SpeechAuthorization::UserRequestedPreview(
                        input.operation_id,
                    )),
                    context,
                )
                .await?
            {
                SpeechRunOutcome::Spoken { .. } => Ok(()),
                SpeechRunOutcome::TextOnly {
                    reason: SpeechFallbackReason::Cancelled,
                    ..
                } => Err(ProviderFailure::new(ProviderFailureCategory::Unavailable)),
                SpeechRunOutcome::TextOnly { reason, .. } => Err(fallback_failure(reason)),
            }
        })
    }

    fn cancel(&self, operation_id: Uuid) -> VoicePreviewCancelDisposition {
        match self.actor.cancel_or_reserve(operation_id) {
            super::SpeechCancelState::Cancelled => VoicePreviewCancelDisposition::Cancelled,
            super::SpeechCancelState::AlreadyTerminal => {
                VoicePreviewCancelDisposition::AlreadyTerminal
            }
            super::SpeechCancelState::NotFound => VoicePreviewCancelDisposition::NotFound,
        }
    }

    fn is_cancelled(&self, operation_id: Uuid) -> bool {
        self.actor.operation_state(operation_id) == Some(super::SpeechOperationState::Cancelled)
    }
}

fn fallback_failure(reason: SpeechFallbackReason) -> ProviderFailure {
    let category = match reason {
        SpeechFallbackReason::Provider(category) | SpeechFallbackReason::Playback(category) => {
            category
        }
        SpeechFallbackReason::Disabled | SpeechFallbackReason::Cancelled => {
            ProviderFailureCategory::Unavailable
        }
    };
    ProviderFailure::new(category)
}

fn safe_label(value: &str, max_chars: usize) -> bool {
    (1..=max_chars).contains(&value.chars().count())
        && !value.chars().any(char::is_control)
        && !value.trim().is_empty()
}
