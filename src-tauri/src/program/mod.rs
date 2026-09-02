//! Deterministic, path-free local program candidate and planning domain.

mod candidate;
mod planner;
mod validation;

pub use candidate::{
    CandidateSelection, CandidateSelectionRequest, CandidateTrack, MAX_CANDIDATE_SOURCE_ROWS,
    MAX_PROGRAM_CANDIDATES, ProgramCandidate, TimeTag, select_candidates,
};
pub use planner::{
    CandidateLoadRequest, LocalProgramRequest, PlanDegradation, PlanOrigin, PlannedProgram,
    ProgramClock, ProgramFuture, ProgramIdFactory, ProgramPlanProvider, ProgramPlanner,
    ProgramProviderError, ProgramRepository, ProviderProgramInput, SystemProgramClock,
    SystemProgramIdFactory,
};
pub use validation::{LocalPlanExpectation, validate_local_program_plan};

/// Stable, payload-free domain/application failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProgramError {
    #[error("program selection request is invalid")]
    InvalidSelectionRequest,
    #[error("candidate data failed validation")]
    InvalidCandidateData,
    #[error("candidate source exceeded its bounded read")]
    CandidatePoolTooLarge,
    #[error("no playable candidate is available")]
    NoPlayableCandidates,
    #[error("program repository is unavailable")]
    RepositoryUnavailable,
    #[error("program plan failed validation")]
    InvalidProgramPlan,
    #[error("program plan identifiers could not be constructed")]
    PlanConstructionFailed,
}

#[cfg(test)]
mod tests;
