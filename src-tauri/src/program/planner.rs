use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::contracts::{
    ProgramPlan, ProgramPlanMode, ProgramPlanSegmentsItem, ProgramPlanTrackSegment,
    ProgramPlanVoiceSegment, ProgramPlanVoiceSegmentTrigger,
};

use super::{
    CandidateSelection, CandidateSelectionRequest, CandidateTrack, LocalPlanExpectation,
    MAX_CANDIDATE_SOURCE_ROWS, MAX_PROGRAM_CANDIDATES, ProgramCandidate, ProgramError,
    select_candidates, validate_local_program_plan,
};

const LOCAL_SOURCE_ID: &str = "local";
const FALLBACK_TRACK_COUNT: usize = 6;
const OPENING_TEXT: &str = "欢迎回来。今天先从你的本地曲库开始。";
const BETWEEN_TRACKS_TEXT: &str = "继续听下去，接下来还是你的本地收藏。";

pub type ProgramFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Bounded request issued to the repository adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateLoadRequest {
    /// Includes one overflow sentinel row so the domain can fail closed.
    pub maximum_rows: usize,
}

/// Injectable, path-free repository projection for candidate facts.
pub trait ProgramRepository: Send + Sync {
    fn load_candidate_tracks(
        &self,
        request: CandidateLoadRequest,
    ) -> ProgramFuture<'_, Result<Vec<CandidateTrack>, ProgramError>>;
}

/// Closed, provider-safe local program-generation input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProgramInput {
    pub program_id: String,
    pub source_id: String,
    pub created_at: String,
    pub local_hour: u8,
    pub profile_tags: Vec<String>,
    pub approved_memory_tags: Vec<String>,
    pub candidates: Vec<ProgramCandidate>,
}

/// Redacted provider outcome. No provider response body is retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramProviderError {
    Cancelled,
    Authentication,
    RateLimited,
    Timeout,
    Unavailable,
    InvalidResponse,
}

/// Injectable structured-plan provider boundary.
pub trait ProgramPlanProvider: Send + Sync {
    fn generate_program<'a>(
        &'a self,
        input: &'a ProviderProgramInput,
    ) -> ProgramFuture<'a, Result<ProgramPlan, ProgramProviderError>>;
}

pub trait ProgramClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProgramClock;

impl ProgramClock for SystemProgramClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub trait ProgramIdFactory: Send + Sync {
    fn next_id(&self) -> Uuid;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProgramIdFactory;

impl ProgramIdFactory for SystemProgramIdFactory {
    fn next_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// User-confirmed local planning facts. Construction itself never starts audio.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProgramRequest {
    pub program_id: Uuid,
    pub local_hour: u8,
    pub profile_tags: Vec<String>,
    pub approved_memory_tags: Vec<String>,
    pub recently_played_track_ids: Vec<String>,
    pub cooldown_ms: i64,
    pub allow_cooldown_relaxation: bool,
    pub selection_seed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanOrigin {
    Provider,
    Deterministic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanDegradation {
    ProviderUnavailable,
    ProviderFailed,
    InvalidProviderPlan,
}

/// Valid plan plus bounded evidence needed by the later persistence/runner layer.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedProgram {
    pub plan: ProgramPlan,
    pub candidates: CandidateSelection,
    pub origin: PlanOrigin,
    pub degradation: Option<PlanDegradation>,
}

/// Candidate and plan orchestration with injected storage/provider/time/ID
/// boundaries. It performs no playback or persistence side effect.
pub struct ProgramPlanner {
    repository: Arc<dyn ProgramRepository>,
    provider: Option<Arc<dyn ProgramPlanProvider>>,
    clock: Arc<dyn ProgramClock>,
    id_factory: Arc<dyn ProgramIdFactory>,
}

impl ProgramPlanner {
    #[must_use]
    pub fn new(
        repository: Arc<dyn ProgramRepository>,
        provider: Option<Arc<dyn ProgramPlanProvider>>,
        clock: Arc<dyn ProgramClock>,
        id_factory: Arc<dyn ProgramIdFactory>,
    ) -> Self {
        Self {
            repository,
            provider,
            clock,
            id_factory,
        }
    }

    /// Builds a provider plan when valid, otherwise a deterministic playable
    /// local plan. This method never starts playback or performs network work
    /// when no provider was injected.
    ///
    /// # Errors
    ///
    /// Returns a stable domain failure for invalid input, unavailable storage,
    /// no playable tracks, or invalid generated `UUIDv7` segment IDs.
    pub async fn plan_local(
        &self,
        request: LocalProgramRequest,
    ) -> Result<PlannedProgram, ProgramError> {
        validate_local_request(&request)?;
        let now = self.clock.now();
        let tracks = self
            .repository
            .load_candidate_tracks(CandidateLoadRequest {
                maximum_rows: MAX_CANDIDATE_SOURCE_ROWS + 1,
            })
            .await?;
        let candidates = select_candidates(
            tracks,
            &CandidateSelectionRequest {
                now_ms: now.timestamp_millis(),
                local_hour: request.local_hour,
                profile_tags: request.profile_tags.clone(),
                approved_memory_tags: request.approved_memory_tags.clone(),
                recently_played_track_ids: request.recently_played_track_ids.clone(),
                cooldown_ms: request.cooldown_ms,
                desired_count: FALLBACK_TRACK_COUNT,
                limit: MAX_PROGRAM_CANDIDATES,
                allow_cooldown_relaxation: request.allow_cooldown_relaxation,
                selection_seed: request.selection_seed,
            },
        )?;
        let created_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        let provider_input = ProviderProgramInput {
            program_id: request.program_id.to_string(),
            source_id: LOCAL_SOURCE_ID.to_owned(),
            created_at: created_at.clone(),
            local_hour: request.local_hour,
            profile_tags: request.profile_tags,
            approved_memory_tags: request.approved_memory_tags,
            candidates: candidates.candidates.clone(),
        };
        let candidate_ids = candidates
            .candidates
            .iter()
            .map(|candidate| candidate.track_id.clone())
            .collect::<HashSet<_>>();
        if let Some(provider) = &self.provider {
            match provider.generate_program(&provider_input).await {
                Ok(plan)
                    if validate_local_program_plan(
                        &plan,
                        &LocalPlanExpectation {
                            program_id: request.program_id,
                            source_id: LOCAL_SOURCE_ID,
                            created_at: &created_at,
                            candidate_track_ids: &candidate_ids,
                        },
                    )
                    .is_ok() =>
                {
                    return Ok(PlannedProgram {
                        plan,
                        candidates,
                        origin: PlanOrigin::Provider,
                        degradation: None,
                    });
                }
                Ok(_) => {
                    return self.fallback(
                        request.program_id,
                        created_at,
                        candidates,
                        PlanDegradation::InvalidProviderPlan,
                    );
                }
                Err(ProgramProviderError::Unavailable) => {
                    return self.fallback(
                        request.program_id,
                        created_at,
                        candidates,
                        PlanDegradation::ProviderUnavailable,
                    );
                }
                Err(_) => {
                    return self.fallback(
                        request.program_id,
                        created_at,
                        candidates,
                        PlanDegradation::ProviderFailed,
                    );
                }
            }
        }
        self.fallback(
            request.program_id,
            created_at,
            candidates,
            PlanDegradation::ProviderUnavailable,
        )
    }

    fn fallback(
        &self,
        program_id: Uuid,
        created_at: String,
        candidates: CandidateSelection,
        degradation: PlanDegradation,
    ) -> Result<PlannedProgram, ProgramError> {
        let plan = deterministic_fallback_plan(
            program_id,
            created_at,
            &candidates.candidates,
            self.id_factory.as_ref(),
        )?;
        let candidate_ids = candidates
            .candidates
            .iter()
            .map(|candidate| candidate.track_id.clone())
            .collect::<HashSet<_>>();
        validate_local_program_plan(
            &plan,
            &LocalPlanExpectation {
                program_id,
                source_id: LOCAL_SOURCE_ID,
                created_at: &plan.created_at,
                candidate_track_ids: &candidate_ids,
            },
        )?;
        Ok(PlannedProgram {
            plan,
            candidates,
            origin: PlanOrigin::Deterministic,
            degradation: Some(degradation),
        })
    }
}

fn validate_local_request(request: &LocalProgramRequest) -> Result<(), ProgramError> {
    if request.program_id.get_version_num() != 7 || request.local_hour > 23 {
        return Err(ProgramError::InvalidSelectionRequest);
    }
    Ok(())
}

fn deterministic_fallback_plan(
    program_id: Uuid,
    created_at: String,
    candidates: &[ProgramCandidate],
    id_factory: &dyn ProgramIdFactory,
) -> Result<ProgramPlan, ProgramError> {
    let selected = candidates
        .iter()
        .take(FALLBACK_TRACK_COUNT)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(ProgramError::NoPlayableCandidates);
    }
    let mut used_segment_ids = HashSet::with_capacity(selected.len() + 3);
    let mut segments = Vec::with_capacity(selected.len() + 3);
    segments.push(ProgramPlanSegmentsItem::VoiceSegment(
        ProgramPlanVoiceSegment {
            r#type: "voice".to_owned(),
            segment_id: next_segment_id(id_factory, &mut used_segment_ids)?,
            text: OPENING_TEXT.to_owned(),
            trigger: ProgramPlanVoiceSegmentTrigger::Opening,
        },
    ));
    let mut group_size = 2_usize;
    let mut tracks_since_voice = 0_usize;
    for (index, candidate) in selected.into_iter().enumerate() {
        if tracks_since_voice == group_size {
            segments.push(ProgramPlanSegmentsItem::VoiceSegment(
                ProgramPlanVoiceSegment {
                    r#type: "voice".to_owned(),
                    segment_id: next_segment_id(id_factory, &mut used_segment_ids)?,
                    text: BETWEEN_TRACKS_TEXT.to_owned(),
                    trigger: ProgramPlanVoiceSegmentTrigger::BetweenTracks,
                },
            ));
            tracks_since_voice = 0;
            group_size = if group_size == 2 { 3 } else { 2 };
        }
        segments.push(ProgramPlanSegmentsItem::TrackSegment(
            ProgramPlanTrackSegment {
                r#type: "track".to_owned(),
                segment_id: next_segment_id(id_factory, &mut used_segment_ids)?,
                track_id: candidate.track_id.clone(),
                segue_text: None,
            },
        ));
        tracks_since_voice += 1;
        if index + 1 == FALLBACK_TRACK_COUNT {
            break;
        }
    }
    Ok(ProgramPlan {
        schema_version: "1.0.0".to_owned(),
        program_id: program_id.to_string(),
        source_id: LOCAL_SOURCE_ID.to_owned(),
        mode: ProgramPlanMode::Local,
        created_at,
        segments,
    })
}

fn next_segment_id(
    id_factory: &dyn ProgramIdFactory,
    used: &mut HashSet<Uuid>,
) -> Result<String, ProgramError> {
    let id = id_factory.next_id();
    if id.get_version_num() != 7 || !used.insert(id) {
        return Err(ProgramError::PlanConstructionFailed);
    }
    Ok(id.to_string())
}
