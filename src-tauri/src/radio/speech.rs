use std::{sync::Arc, time::Duration};

use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    ipc::{ApiError, InternalReason},
    providers::ProviderCallContext,
    speech::{
        SpeechActor, SpeechAuthorization, SpeechInput, SpeechOwner, SpeechProvenance,
        SpeechRunOutcome,
    },
};

use super::{ConfirmedProgramStart, ProgramSpeech, ProgramSpeechOutcome, traits::RadioFuture};

#[derive(Clone, Debug, PartialEq)]
pub struct SpeechRuntimeConfig {
    pub enabled: bool,
    pub voice_id: String,
    pub model_id: String,
    pub speed: f32,
    pub locale: String,
}

/// TASK-017 adapter that preserves visible text when speech is disabled or
/// unavailable and carries confirmed-program authority into the actor.
pub struct SpeechActorProgramAdapter {
    actor: Arc<SpeechActor>,
    config: SpeechRuntimeConfig,
}

impl SpeechActorProgramAdapter {
    #[must_use]
    pub fn new(actor: Arc<SpeechActor>, config: SpeechRuntimeConfig) -> Self {
        Self { actor, config }
    }
}

impl ProgramSpeech for SpeechActorProgramAdapter {
    fn present<'a>(
        &'a self,
        program_id: Uuid,
        segment_id: Uuid,
        text: &'a str,
        _authorization: ConfirmedProgramStart,
        mut cancellation: watch::Receiver<bool>,
    ) -> RadioFuture<'a, Result<ProgramSpeechOutcome, ApiError>> {
        Box::pin(async move {
            if *cancellation.borrow() {
                return Err(ApiError::from_reason(InternalReason::OperationCancelled));
            }
            let input = SpeechInput::segment(
                text,
                self.config.voice_id.clone(),
                self.config.model_id.clone(),
                self.config.speed,
                self.config.locale.clone(),
                SpeechProvenance::LocalProgram,
            )
            .map_err(|failure| failure.into_api_error(false))?;
            let context = ProviderCallContext::new(Duration::from_secs(60));
            let caller_cancellation = context.cancellation.clone();
            let run = self.actor.run(
                segment_id,
                input,
                SpeechOwner::Segment(segment_id),
                self.config.enabled,
                Some(SpeechAuthorization::ConfirmedProgram(program_id)),
                &context,
            );
            tokio::pin!(run);
            tokio::select! {
                result = &mut run => match result.map_err(|failure| failure.into_api_error(false))? {
                    SpeechRunOutcome::Spoken { .. } => Ok(ProgramSpeechOutcome::Spoken),
                    SpeechRunOutcome::TextOnly { .. } => Ok(ProgramSpeechOutcome::TextOnly),
                },
                _ = cancellation.changed() => {
                    caller_cancellation.cancel();
                    let _ = self.actor.cancel(segment_id);
                    Err(ApiError::from_reason(InternalReason::OperationCancelled))
                }
            }
        })
    }

    fn cancel(&self, segment_id: Uuid) {
        let _ = self.actor.cancel(segment_id);
    }
}

/// Explicitly disabled adapter: returns visible text without provider or audio.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextOnlyProgramSpeech;

impl ProgramSpeech for TextOnlyProgramSpeech {
    fn present<'a>(
        &'a self,
        _program_id: Uuid,
        _segment_id: Uuid,
        _text: &'a str,
        _authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
    ) -> RadioFuture<'a, Result<ProgramSpeechOutcome, ApiError>> {
        Box::pin(async move {
            if *cancellation.borrow() {
                Err(ApiError::from_reason(InternalReason::OperationCancelled))
            } else {
                Ok(ProgramSpeechOutcome::TextOnly)
            }
        })
    }

    fn cancel(&self, _segment_id: Uuid) {}
}
