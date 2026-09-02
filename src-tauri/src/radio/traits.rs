use std::{future::Future, pin::Pin, sync::Arc};

use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::{
    ipc::{ApiError, InternalReason},
    playback::PlaybackStartAuthorization,
    program::{LocalProgramRequest, PlannedProgram, ProgramError, ProgramPlanner},
};

use super::{StartProgramRequest, StartProgramTrigger, playback::ProgramTrack};

pub type RadioFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Authority proven by the command boundary before any planning/provider work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmedProgramStart {
    Manual,
    ConfirmedNotification,
}

impl ConfirmedProgramStart {
    pub(crate) const fn playback(self) -> PlaybackStartAuthorization {
        match self {
            Self::Manual => PlaybackStartAuthorization::Manual,
            Self::ConfirmedNotification => PlaybackStartAuthorization::ConfirmedNotification,
        }
    }
}

/// Validates that API-024 carries a direct user action or a previously
/// confirmed notification occurrence.
pub trait ProgramStartAuthorizer: Send + Sync {
    fn authorize<'a>(
        &'a self,
        request: &'a StartProgramRequest,
    ) -> RadioFuture<'a, Result<ConfirmedProgramStart, ApiError>>;
}

/// Production-safe baseline until the scheduler supplies notification proof.
/// Manual starts are accepted; bare notification claims are rejected.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManualProgramStartAuthorizer;

impl ProgramStartAuthorizer for ManualProgramStartAuthorizer {
    fn authorize<'a>(
        &'a self,
        request: &'a StartProgramRequest,
    ) -> RadioFuture<'a, Result<ConfirmedProgramStart, ApiError>> {
        Box::pin(async move {
            match request.trigger {
                StartProgramTrigger::Manual => Ok(ConfirmedProgramStart::Manual),
                StartProgramTrigger::Notification => {
                    Err(ApiError::from_reason(InternalReason::RequestInvalid))
                }
            }
        })
    }
}

/// Non-path planning context loaded from approved local facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadioPlanningContext {
    pub local_hour: u8,
    pub profile_tags: Vec<String>,
    pub approved_memory_tags: Vec<String>,
    pub recently_played_track_ids: Vec<String>,
    pub cooldown_ms: i64,
    pub allow_cooldown_relaxation: bool,
    pub selection_seed: u64,
}

pub trait ProgramPlannerContextSource: Send + Sync {
    fn load_context(
        &self,
        program_id: Uuid,
    ) -> RadioFuture<'_, Result<RadioPlanningContext, ApiError>>;
}

pub trait ProgramRadioPlanner: Send + Sync {
    fn plan_local(&self, program_id: Uuid) -> RadioFuture<'_, Result<PlannedProgram, ApiError>>;
}

/// Adapter from the TASK-015 domain planner to the runner's minimal boundary.
pub struct DomainProgramPlanner {
    planner: Arc<ProgramPlanner>,
    context: Arc<dyn ProgramPlannerContextSource>,
}

impl DomainProgramPlanner {
    #[must_use]
    pub fn new(
        planner: Arc<ProgramPlanner>,
        context: Arc<dyn ProgramPlannerContextSource>,
    ) -> Self {
        Self { planner, context }
    }
}

impl ProgramRadioPlanner for DomainProgramPlanner {
    fn plan_local(&self, program_id: Uuid) -> RadioFuture<'_, Result<PlannedProgram, ApiError>> {
        Box::pin(async move {
            let context = self.context.load_context(program_id).await?;
            self.planner
                .plan_local(LocalProgramRequest {
                    program_id,
                    local_hour: context.local_hour,
                    profile_tags: context.profile_tags,
                    approved_memory_tags: context.approved_memory_tags,
                    recently_played_track_ids: context.recently_played_track_ids,
                    cooldown_ms: context.cooldown_ms,
                    allow_cooldown_relaxation: context.allow_cooldown_relaxation,
                    selection_seed: context.selection_seed,
                })
                .await
                .map_err(map_program_error)
        })
    }
}

fn map_program_error(error: ProgramError) -> ApiError {
    let reason = match error {
        ProgramError::InvalidSelectionRequest | ProgramError::InvalidCandidateData => {
            InternalReason::InvalidCandidate
        }
        ProgramError::CandidatePoolTooLarge => InternalReason::ResourceBusy,
        ProgramError::NoPlayableCandidates => InternalReason::SourceUnavailable,
        ProgramError::RepositoryUnavailable => InternalReason::StorageReadFailed,
        ProgramError::InvalidProgramPlan | ProgramError::PlanConstructionFailed => {
            InternalReason::ProviderInvalidResponse
        }
    };
    ApiError::from_reason(reason)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramPlaybackSignal {
    Started(Uuid),
    Completed(Uuid),
    Failed(Uuid),
    Paused,
    Resumed,
}

pub trait ProgramPlayback: Send + Sync {
    fn play_batch<'a>(
        &'a self,
        program_id: Uuid,
        tracks: &'a [ProgramTrack],
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
        signals: mpsc::Sender<ProgramPlaybackSignal>,
    ) -> RadioFuture<'a, Result<(), ApiError>>;

    fn stop(&self) -> RadioFuture<'_, Result<(), ApiError>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramSpeechOutcome {
    Spoken,
    TextOnly,
}

pub trait ProgramSpeech: Send + Sync {
    fn present<'a>(
        &'a self,
        program_id: Uuid,
        segment_id: Uuid,
        text: &'a str,
        authorization: ConfirmedProgramStart,
        cancellation: watch::Receiver<bool>,
    ) -> RadioFuture<'a, Result<ProgramSpeechOutcome, ApiError>>;

    fn cancel(&self, segment_id: Uuid);
}
