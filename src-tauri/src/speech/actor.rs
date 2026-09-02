use super::{
    SpeechArtifact, SpeechCache, SpeechInput, SpeechOwner, TtsProvider, invalid_response,
    valid_operation_id,
};
use crate::providers::{
    CancellationFlag, Clock, ProviderCallContext, ProviderFailure, ProviderFailureCategory,
};
use crate::speech::provider::{SpeechFuture, SpeechSink, wait_for_cancellation};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub trait SpeechPlayback: Send + Sync {
    /// Plays only if `cancellation` is still clear at the output side-effect.
    ///
    /// # Errors
    ///
    /// Returns a redacted audio/output failure. Implementations must not retry.
    fn play(
        &self,
        operation_id: Uuid,
        bytes: Arc<[u8]>,
        operation_cancellation: CancellationFlag,
        caller_cancellation: CancellationFlag,
    ) -> SpeechFuture<'_, Result<(), ProviderFailure>>;

    /// Idempotently stops output owned by this exact operation.
    fn stop(&self, operation_id: Uuid);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechOperationState {
    Preparing,
    Playing,
    Completed,
    Cancelled,
    Failed,
    TextOnly,
}

impl SpeechOperationState {
    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::TextOnly
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechCancelState {
    Cancelled,
    AlreadyTerminal,
    NotFound,
}

/// Explicit paid/sound authority supplied by the owning user action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechAuthorization {
    ConfirmedProgram(Uuid),
    UserSubmittedChat(Uuid),
    UserRequestedPreview(Uuid),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechFallbackReason {
    Disabled,
    Provider(ProviderFailureCategory),
    Playback(ProviderFailureCategory),
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpeechRunOutcome {
    Spoken {
        text: String,
        artifact: SpeechArtifact,
    },
    TextOnly {
        text: String,
        reason: SpeechFallbackReason,
    },
}

struct OperationEntry {
    state: SpeechOperationState,
    cancellation: CancellationFlag,
}

#[derive(Default)]
struct ActorState {
    operations: HashMap<Uuid, OperationEntry>,
    terminal_order: VecDeque<Uuid>,
}

pub struct SpeechActor {
    provider: Arc<dyn TtsProvider>,
    cache: SpeechCache,
    playback: Arc<dyn SpeechPlayback>,
    clock: Arc<dyn Clock>,
    terminal_capacity: usize,
    state: Mutex<ActorState>,
}

impl SpeechActor {
    #[must_use]
    pub fn new(
        provider: Arc<dyn TtsProvider>,
        cache: SpeechCache,
        playback: Arc<dyn SpeechPlayback>,
        clock: Arc<dyn Clock>,
        terminal_capacity: usize,
    ) -> Self {
        Self {
            provider,
            cache,
            playback,
            clock,
            terminal_capacity: terminal_capacity.clamp(1, 4_096),
            state: Mutex::new(ActorState::default()),
        }
    }

    /// Synthesizes and plays at most once, otherwise returns the same visible
    /// text as a deterministic fallback.
    ///
    /// # Errors
    ///
    /// Rejects invalid/duplicate operation ownership. Provider and playback
    /// failures are represented as text fallback outcomes, not retried errors.
    #[allow(
        clippy::too_many_lines,
        reason = "the ordered cancellation, cache-promotion, and playback transitions form one auditable operation"
    )]
    pub async fn run(
        &self,
        operation_id: Uuid,
        input: SpeechInput,
        owner: SpeechOwner,
        tts_enabled: bool,
        authorization: Option<SpeechAuthorization>,
        caller_context: &ProviderCallContext,
    ) -> Result<SpeechRunOutcome, ProviderFailure> {
        validate_owner(operation_id, input.provenance(), owner)?;
        if tts_enabled {
            validate_authorization(operation_id, input.provenance(), authorization)?;
        }
        let cancellation = self.begin(operation_id)?;
        let text = input.text().to_owned();
        if !tts_enabled {
            self.finish(operation_id, SpeechOperationState::TextOnly);
            return Ok(SpeechRunOutcome::TextOnly {
                text,
                reason: SpeechFallbackReason::Disabled,
            });
        }
        if caller_context.cancellation.is_cancelled() {
            let _ = self.cancel(operation_id);
            return Ok(cancelled_outcome(text));
        }

        let key = input.cache_key();
        let lease = if let Some(lease) = self.cache.acquire(key, owner, self.clock.now_ms()) {
            lease
        } else {
            let provider_context = ProviderCallContext {
                correlation_id: caller_context.correlation_id,
                locale: caller_context.locale,
                deadline: caller_context.deadline,
                cancellation: cancellation.clone(),
            };
            let mut sink = VecSpeechSink::default();
            let result = {
                let synthesis =
                    self.provider
                        .synthesize(input.clone(), &mut sink, &provider_context);
                tokio::pin!(synthesis);
                tokio::select! {
                    result = &mut synthesis => Some(result),
                    () = wait_for_cancellation(&caller_context.cancellation) => None,
                }
            };
            let Some(result) = result else {
                cancellation.cancel();
                let _ = self.cancel(operation_id);
                return Ok(cancelled_outcome(text));
            };
            if self.operation_state(operation_id) == Some(SpeechOperationState::Cancelled) {
                return Ok(cancelled_outcome(text));
            }
            let artifact = match result {
                Ok(artifact) => artifact,
                Err(failure) => {
                    self.finish(operation_id, SpeechOperationState::Failed);
                    return Ok(SpeechRunOutcome::TextOnly {
                        text,
                        reason: SpeechFallbackReason::Provider(failure.category),
                    });
                }
            };
            let lease =
                match self.promote_and_acquire(operation_id, &input, artifact, sink.bytes, owner) {
                    Ok(Some(lease)) => lease,
                    Ok(None) => return Ok(cancelled_outcome(text)),
                    Err(failure) => {
                        self.finish(operation_id, SpeechOperationState::Failed);
                        return Ok(SpeechRunOutcome::TextOnly {
                            text,
                            reason: SpeechFallbackReason::Provider(failure.category),
                        });
                    }
                };
            if self.operation_state(operation_id) == Some(SpeechOperationState::Cancelled) {
                return Ok(cancelled_outcome(text));
            }
            if lease.artifact().content_hash != key.to_hex() {
                self.finish(operation_id, SpeechOperationState::Failed);
                return Ok(SpeechRunOutcome::TextOnly {
                    text,
                    reason: SpeechFallbackReason::Provider(
                        ProviderFailureCategory::InvalidResponse,
                    ),
                });
            }
            lease
        };

        if !self.transition_to_playing(operation_id) {
            return Ok(cancelled_outcome(text));
        }
        let artifact = lease.artifact().clone();
        let playback = self.playback.play(
            operation_id,
            lease.bytes(),
            cancellation.clone(),
            caller_context.cancellation.clone(),
        );
        tokio::pin!(playback);
        let playback_result = tokio::select! {
            result = &mut playback => Some(result),
            () = wait_for_cancellation(&cancellation) => None,
            () = wait_for_cancellation(&caller_context.cancellation) => None,
        };
        if playback_result.is_none() {
            cancellation.cancel();
            let _ = self.cancel(operation_id);
            return Ok(cancelled_outcome(text));
        }
        if self.operation_state(operation_id) == Some(SpeechOperationState::Cancelled) {
            return Ok(cancelled_outcome(text));
        }
        match playback_result.and_then(Result::err) {
            Some(failure) => {
                self.finish(operation_id, SpeechOperationState::Failed);
                Ok(SpeechRunOutcome::TextOnly {
                    text,
                    reason: SpeechFallbackReason::Playback(failure.category),
                })
            }
            None => {
                if self.operation_state(operation_id) == Some(SpeechOperationState::Cancelled) {
                    Ok(cancelled_outcome(text))
                } else {
                    self.finish(operation_id, SpeechOperationState::Completed);
                    Ok(SpeechRunOutcome::Spoken { text, artifact })
                }
            }
        }
    }

    #[must_use]
    pub fn cancel(&self, operation_id: Uuid) -> SpeechCancelState {
        let should_stop = {
            let mut state = self.lock();
            let Some(entry) = state.operations.get_mut(&operation_id) else {
                return SpeechCancelState::NotFound;
            };
            if entry.state.is_terminal() {
                return SpeechCancelState::AlreadyTerminal;
            }
            let should_stop = entry.state == SpeechOperationState::Playing;
            entry.cancellation.cancel();
            entry.state = SpeechOperationState::Cancelled;
            push_terminal(&mut state, operation_id, self.terminal_capacity);
            should_stop
        };
        if should_stop {
            self.playback.stop(operation_id);
        }
        SpeechCancelState::Cancelled
    }

    #[must_use]
    pub fn operation_state(&self, operation_id: Uuid) -> Option<SpeechOperationState> {
        self.lock()
            .operations
            .get(&operation_id)
            .map(|entry| entry.state)
    }

    fn begin(&self, operation_id: Uuid) -> Result<CancellationFlag, ProviderFailure> {
        if !valid_operation_id(operation_id) {
            return Err(invalid_response());
        }
        let mut state = self.lock();
        if state.operations.contains_key(&operation_id) {
            return Err(invalid_response());
        }
        let cancellation = CancellationFlag::default();
        state.operations.insert(
            operation_id,
            OperationEntry {
                state: SpeechOperationState::Preparing,
                cancellation: cancellation.clone(),
            },
        );
        Ok(cancellation)
    }

    fn transition_to_playing(&self, operation_id: Uuid) -> bool {
        let mut state = self.lock();
        let Some(entry) = state.operations.get_mut(&operation_id) else {
            return false;
        };
        if entry.state != SpeechOperationState::Preparing || entry.cancellation.is_cancelled() {
            return false;
        }
        entry.state = SpeechOperationState::Playing;
        true
    }

    fn promote_and_acquire(
        &self,
        operation_id: Uuid,
        input: &SpeechInput,
        artifact: SpeechArtifact,
        bytes: Vec<u8>,
        owner: SpeechOwner,
    ) -> Result<Option<super::SpeechLease>, ProviderFailure> {
        let state = self.lock();
        let Some(entry) = state.operations.get(&operation_id) else {
            return Ok(None);
        };
        if entry.state != SpeechOperationState::Preparing || entry.cancellation.is_cancelled() {
            return Ok(None);
        }
        let now_ms = self.clock.now_ms();
        self.cache.insert(input, artifact, bytes, owner, now_ms)?;
        Ok(self.cache.acquire(input.cache_key(), owner, now_ms))
    }

    fn finish(&self, operation_id: Uuid, terminal: SpeechOperationState) {
        let mut state = self.lock();
        let Some(entry) = state.operations.get_mut(&operation_id) else {
            return;
        };
        if entry.state == SpeechOperationState::Cancelled {
            return;
        }
        entry.state = terminal;
        push_terminal(&mut state, operation_id, self.terminal_capacity);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ActorState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn push_terminal(state: &mut ActorState, operation_id: Uuid, capacity: usize) {
    state.terminal_order.push_back(operation_id);
    while state.terminal_order.len() > capacity {
        let Some(oldest) = state.terminal_order.pop_front() else {
            break;
        };
        if state
            .operations
            .get(&oldest)
            .is_some_and(|entry| entry.state.is_terminal())
        {
            state.operations.remove(&oldest);
        }
    }
}

fn validate_owner(
    operation_id: Uuid,
    provenance: super::SpeechProvenance,
    owner: SpeechOwner,
) -> Result<(), ProviderFailure> {
    let valid = match (provenance, owner) {
        (super::SpeechProvenance::VoicePreview, SpeechOwner::Preview(owner_id)) => {
            owner_id == operation_id
        }
        (
            super::SpeechProvenance::LocalProgram
            | super::SpeechProvenance::Chat
            | super::SpeechProvenance::SystemSessionGeneric,
            SpeechOwner::Segment(_),
        ) => true,
        _ => false,
    };
    if valid && owner.is_valid() {
        Ok(())
    } else {
        Err(invalid_response())
    }
}

fn validate_authorization(
    operation_id: Uuid,
    provenance: super::SpeechProvenance,
    authorization: Option<SpeechAuthorization>,
) -> Result<(), ProviderFailure> {
    let valid = match (provenance, authorization) {
        (
            super::SpeechProvenance::LocalProgram | super::SpeechProvenance::SystemSessionGeneric,
            Some(SpeechAuthorization::ConfirmedProgram(program_id)),
        ) => valid_operation_id(program_id),
        (
            super::SpeechProvenance::Chat,
            Some(SpeechAuthorization::UserSubmittedChat(chat_operation_id)),
        ) => valid_operation_id(chat_operation_id),
        (
            super::SpeechProvenance::VoicePreview,
            Some(SpeechAuthorization::UserRequestedPreview(preview_operation_id)),
        ) => preview_operation_id == operation_id,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid_response())
    }
}

fn cancelled_outcome(text: String) -> SpeechRunOutcome {
    SpeechRunOutcome::TextOnly {
        text,
        reason: SpeechFallbackReason::Cancelled,
    }
}

#[derive(Default)]
struct VecSpeechSink {
    bytes: Vec<u8>,
}

impl SpeechSink for VecSpeechSink {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ProviderFailure> {
        if self.bytes.len().saturating_add(bytes.len()) > super::MAX_SPEECH_BYTES {
            return Err(invalid_response());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
