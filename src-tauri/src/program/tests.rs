use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};

use super::*;
use crate::contracts::{
    ProgramPlan, ProgramPlanMode, ProgramPlanSegmentsItem, ProgramPlanTrackSegment,
    ProgramPlanVoiceSegment, ProgramPlanVoiceSegmentTrigger,
};

const NOW_MS: i64 = 1_788_399_723_000;
const CREATED_AT: &str = "2026-09-03T01:02:03.000Z";

fn track(track_id: uuid::Uuid, tag: &str) -> CandidateTrack {
    CandidateTrack {
        track_id: track_id.to_string(),
        title: Some(format!("Track {tag}")),
        artist: Some("Fixture Artist".to_owned()),
        album: None,
        duration_ms: 180_000,
        normalized_tags: vec![tag.to_owned()],
        time_tags: Vec::new(),
        last_played_at_ms: None,
        like_count: 0,
        skip_count: 0,
        playable: true,
    }
}

fn selection_request() -> CandidateSelectionRequest {
    CandidateSelectionRequest {
        now_ms: NOW_MS,
        local_hour: 20,
        profile_tags: vec!["ambient".to_owned()],
        approved_memory_tags: vec!["focus".to_owned()],
        recently_played_track_ids: Vec::new(),
        cooldown_ms: 3_600_000,
        desired_count: 6,
        limit: MAX_PROGRAM_CANDIDATES,
        allow_cooldown_relaxation: false,
        selection_seed: 42,
    }
}

#[test]
fn candidate_scoring_combines_time_profile_memory_feedback_and_recency() {
    let preferred_id = uuid::Uuid::now_v7();
    let recent_id = uuid::Uuid::now_v7();
    let skipped_id = uuid::Uuid::now_v7();
    let mut preferred = track(preferred_id, "ambient");
    preferred.normalized_tags.push("focus".to_owned());
    preferred.time_tags.push(TimeTag::Evening);
    preferred.like_count = 3;
    let mut recent = track(recent_id, "ambient");
    recent.like_count = 2;
    let mut skipped = track(skipped_id, "other");
    skipped.skip_count = 4;
    let mut request = selection_request();
    request.desired_count = 3;
    request.recently_played_track_ids = vec![recent_id.to_string()];

    let selected = select_candidates(vec![skipped, recent, preferred], &request)
        .expect("valid deterministic selection");
    assert_eq!(selected.candidates.len(), 3);
    assert_eq!(selected.candidates[0].track_id, preferred_id.to_string());
    assert_eq!(selected.candidates[1].track_id, skipped_id.to_string());
    assert_eq!(selected.candidates[2].track_id, recent_id.to_string());
    assert_eq!(selected.candidates[2].recent_play_penalty, -500);
}

#[test]
fn candidate_cooldown_requires_explicit_scarcity_relaxation() {
    let fresh_id = uuid::Uuid::now_v7();
    let cooled_id = uuid::Uuid::now_v7();
    let fresh = track(fresh_id, "ambient");
    let mut cooled = track(cooled_id, "focus");
    cooled.last_played_at_ms = Some(NOW_MS - 500);
    cooled.like_count = 20;
    let mut request = selection_request();
    request.desired_count = 2;

    let strict = select_candidates(vec![cooled.clone(), fresh.clone()], &request)
        .expect("fresh candidate remains");
    assert_eq!(strict.candidates.len(), 1);
    assert_eq!(strict.candidates[0].track_id, fresh_id.to_string());
    assert!(!strict.cooldown_relaxed);

    request.allow_cooldown_relaxation = true;
    let degraded =
        select_candidates(vec![cooled, fresh], &request).expect("explicit relaxation is allowed");
    assert_eq!(degraded.candidates.len(), 2);
    assert!(degraded.cooldown_relaxed);
    assert_eq!(degraded.candidates[1].track_id, cooled_id.to_string());
}

#[test]
fn candidate_pool_is_bounded_and_rejects_duplicate_or_illegal_ids() {
    let tracks = (0..=MAX_PROGRAM_CANDIDATES)
        .map(|index| track(uuid::Uuid::now_v7(), &format!("tag-{index}")))
        .collect::<Vec<_>>();
    let selected = select_candidates(tracks, &selection_request()).expect("bounded selection");
    assert_eq!(selected.candidates.len(), MAX_PROGRAM_CANDIDATES);

    let repeated_id = uuid::Uuid::now_v7();
    let duplicate = vec![track(repeated_id, "one"), track(repeated_id, "two")];
    assert_eq!(
        select_candidates(duplicate, &selection_request()),
        Err(ProgramError::InvalidCandidateData)
    );
    let mut invalid = track(uuid::Uuid::now_v7(), "invalid");
    invalid.track_id = "not-a-real-id".to_owned();
    assert_eq!(
        select_candidates(vec![invalid], &selection_request()),
        Err(ProgramError::InvalidCandidateData)
    );
}

#[test]
fn approved_memory_context_accepts_the_documented_240_character_projection() {
    let mut request = selection_request();
    request.desired_count = 1;
    request.approved_memory_tags = vec!["记".repeat(240)];
    assert!(select_candidates(vec![track(uuid::Uuid::now_v7(), "ambient")], &request).is_ok());

    request.approved_memory_tags = vec!["记".repeat(241)];
    assert_eq!(
        select_candidates(vec![track(uuid::Uuid::now_v7(), "ambient")], &request),
        Err(ProgramError::InvalidSelectionRequest)
    );
}

struct FakeRepository {
    tracks: Mutex<Option<Result<Vec<CandidateTrack>, ProgramError>>>,
    requests: Mutex<Vec<CandidateLoadRequest>>,
}

impl FakeRepository {
    fn new(tracks: Vec<CandidateTrack>) -> Self {
        Self {
            tracks: Mutex::new(Some(Ok(tracks))),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl ProgramRepository for FakeRepository {
    fn load_candidate_tracks(
        &self,
        request: CandidateLoadRequest,
    ) -> ProgramFuture<'_, Result<Vec<CandidateTrack>, ProgramError>> {
        self.requests
            .lock()
            .expect("repository request lock")
            .push(request);
        let result = self
            .tracks
            .lock()
            .expect("repository result lock")
            .take()
            .unwrap_or(Err(ProgramError::RepositoryUnavailable));
        Box::pin(async move { result })
    }
}

struct FakeProvider {
    result: Mutex<Option<Result<ProgramPlan, ProgramProviderError>>>,
    inputs: Mutex<Vec<ProviderProgramInput>>,
}

impl FakeProvider {
    fn new(result: Result<ProgramPlan, ProgramProviderError>) -> Self {
        Self {
            result: Mutex::new(Some(result)),
            inputs: Mutex::new(Vec::new()),
        }
    }
}

impl ProgramPlanProvider for FakeProvider {
    fn generate_program<'a>(
        &'a self,
        input: &'a ProviderProgramInput,
    ) -> ProgramFuture<'a, Result<ProgramPlan, ProgramProviderError>> {
        self.inputs
            .lock()
            .expect("provider input lock")
            .push(input.clone());
        let result = self
            .result
            .lock()
            .expect("provider result lock")
            .take()
            .unwrap_or(Err(ProgramProviderError::Unavailable));
        Box::pin(async move { result })
    }
}

struct FixedClock;

impl ProgramClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(CREATED_AT)
            .expect("fixed RFC3339")
            .with_timezone(&Utc)
    }
}

struct FixedIds(Mutex<VecDeque<uuid::Uuid>>);

impl FixedIds {
    fn new(count: usize) -> Self {
        Self(Mutex::new(
            (0..count).map(|_| uuid::Uuid::now_v7()).collect(),
        ))
    }
}

impl ProgramIdFactory for FixedIds {
    fn next_id(&self) -> uuid::Uuid {
        self.0
            .lock()
            .expect("ID queue lock")
            .pop_front()
            .expect("enough fixed IDs")
    }
}

fn local_request(program_id: uuid::Uuid) -> LocalProgramRequest {
    LocalProgramRequest {
        program_id,
        local_hour: 20,
        profile_tags: vec!["ambient".to_owned()],
        approved_memory_tags: vec!["focus".to_owned()],
        recently_played_track_ids: Vec::new(),
        cooldown_ms: 3_600_000,
        allow_cooldown_relaxation: false,
        selection_seed: 7,
    }
}

fn six_tracks() -> Vec<CandidateTrack> {
    (0..6)
        .map(|index| track(uuid::Uuid::now_v7(), &format!("ambient-{index}")))
        .collect()
}

fn planner(repository: Arc<FakeRepository>, provider: Option<Arc<FakeProvider>>) -> ProgramPlanner {
    ProgramPlanner::new(
        repository,
        provider.map(|value| value as Arc<dyn ProgramPlanProvider>),
        Arc::new(FixedClock),
        Arc::new(FixedIds::new(16)),
    )
}

#[tokio::test]
async fn program_plan_provider_failure_falls_back_to_six_real_tracks_and_safe_text() {
    let tracks = six_tracks();
    let expected_ids = tracks
        .iter()
        .map(|value| value.track_id.clone())
        .collect::<std::collections::HashSet<_>>();
    let repository = Arc::new(FakeRepository::new(tracks));
    let provider = Arc::new(FakeProvider::new(Err(ProgramProviderError::Timeout)));
    let program_id = uuid::Uuid::now_v7();
    let planned = planner(repository.clone(), Some(provider.clone()))
        .plan_local(local_request(program_id))
        .await
        .expect("fallback plan");

    assert_eq!(planned.origin, PlanOrigin::Deterministic);
    assert_eq!(planned.degradation, Some(PlanDegradation::ProviderFailed));
    assert_eq!(planned.plan.schema_version, "1.0.0");
    assert_eq!(planned.plan.source_id, "local");
    assert_eq!(planned.plan.mode, ProgramPlanMode::Local);
    assert_eq!(planned.plan.created_at, CREATED_AT);
    let track_ids = planned
        .plan
        .segments
        .iter()
        .filter_map(|segment| match segment {
            ProgramPlanSegmentsItem::TrackSegment(value) => Some(value.track_id.clone()),
            ProgramPlanSegmentsItem::VoiceSegment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(track_ids.len(), 6);
    assert!(track_ids.iter().all(|id| expected_ids.contains(id)));
    let voice_positions = planned
        .plan
        .segments
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| {
            matches!(segment, ProgramPlanSegmentsItem::VoiceSegment(_)).then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(voice_positions, vec![0, 3, 7]);
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 1);
    let serialized_input = serde_json::to_string(
        &provider
            .inputs
            .lock()
            .expect("provider input serialization")[0],
    )
    .expect("provider input JSON");
    assert!(!serialized_input.to_lowercase().contains("path"));
    assert_eq!(
        repository.requests.lock().expect("requests").as_slice(),
        [CandidateLoadRequest {
            maximum_rows: MAX_CANDIDATE_SOURCE_ROWS + 1,
        }]
    );
}

#[tokio::test]
async fn program_plan_fallback_uses_every_track_when_fewer_than_six_exist() {
    let tracks = (0..3)
        .map(|index| track(uuid::Uuid::now_v7(), &format!("short-{index}")))
        .collect::<Vec<_>>();
    let planned = planner(Arc::new(FakeRepository::new(tracks)), None)
        .plan_local(local_request(uuid::Uuid::now_v7()))
        .await
        .expect("short fallback plan");
    let track_count = planned
        .plan
        .segments
        .iter()
        .filter(|segment| matches!(segment, ProgramPlanSegmentsItem::TrackSegment(_)))
        .count();
    assert_eq!(track_count, 3);
    assert_eq!(planned.origin, PlanOrigin::Deterministic);
    assert_eq!(
        planned.degradation,
        Some(PlanDegradation::ProviderUnavailable)
    );
}

#[tokio::test]
async fn program_plan_unknown_provider_id_is_rejected_and_replaced() {
    let tracks = six_tracks();
    let program_id = uuid::Uuid::now_v7();
    let provider_plan = valid_plan(
        program_id,
        uuid::Uuid::now_v7().to_string(),
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
    );
    let provider = Arc::new(FakeProvider::new(Ok(provider_plan)));
    let planned = planner(Arc::new(FakeRepository::new(tracks)), Some(provider))
        .plan_local(local_request(program_id))
        .await
        .expect("invalid provider output degrades");
    assert_eq!(planned.origin, PlanOrigin::Deterministic);
    assert_eq!(
        planned.degradation,
        Some(PlanDegradation::InvalidProviderPlan)
    );
}

#[tokio::test]
async fn program_plan_valid_provider_output_is_retained() {
    let track_id = uuid::Uuid::now_v7();
    let program_id = uuid::Uuid::now_v7();
    let provider_plan = valid_plan(
        program_id,
        track_id.to_string(),
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
    );
    let provider = Arc::new(FakeProvider::new(Ok(provider_plan.clone())));
    let planned = planner(
        Arc::new(FakeRepository::new(vec![track(track_id, "ambient")])),
        Some(provider),
    )
    .plan_local(local_request(program_id))
    .await
    .expect("valid provider plan");
    assert_eq!(planned.origin, PlanOrigin::Provider);
    assert_eq!(planned.degradation, None);
    assert_eq!(planned.plan, provider_plan);
}

#[test]
fn program_plan_domain_rejects_duplicate_illegal_and_unknown_ids() {
    let program_id = uuid::Uuid::now_v7();
    let known_track = uuid::Uuid::now_v7();
    let candidate_ids = [known_track.to_string()].into_iter().collect();
    let opening_id = uuid::Uuid::now_v7();
    let track_segment_id = uuid::Uuid::now_v7();
    let expected = LocalPlanExpectation {
        program_id,
        source_id: "local",
        created_at: CREATED_AT,
        candidate_track_ids: &candidate_ids,
    };
    let valid = valid_plan(
        program_id,
        known_track.to_string(),
        opening_id,
        track_segment_id,
    );
    validate_local_program_plan(&valid, &expected).expect("valid domain plan");

    let mut unknown = valid.clone();
    let ProgramPlanSegmentsItem::TrackSegment(track) = &mut unknown.segments[1] else {
        panic!("fixture track segment");
    };
    track.track_id = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        validate_local_program_plan(&unknown, &expected),
        Err(ProgramError::InvalidProgramPlan)
    );

    let mut illegal = valid.clone();
    let ProgramPlanSegmentsItem::TrackSegment(track) = &mut illegal.segments[1] else {
        panic!("fixture track segment");
    };
    track.segment_id = uuid::Uuid::nil().to_string();
    assert_eq!(
        validate_local_program_plan(&illegal, &expected),
        Err(ProgramError::InvalidProgramPlan)
    );

    let mut duplicate = valid;
    duplicate
        .segments
        .push(ProgramPlanSegmentsItem::TrackSegment(
            ProgramPlanTrackSegment {
                r#type: "track".to_owned(),
                segment_id: uuid::Uuid::now_v7().to_string(),
                track_id: known_track.to_string(),
                segue_text: None,
            },
        ));
    assert_eq!(
        validate_local_program_plan(&duplicate, &expected),
        Err(ProgramError::InvalidProgramPlan)
    );
}

fn valid_plan(
    program_id: uuid::Uuid,
    track_id: String,
    opening_id: uuid::Uuid,
    track_segment_id: uuid::Uuid,
) -> ProgramPlan {
    ProgramPlan {
        schema_version: "1.0.0".to_owned(),
        program_id: program_id.to_string(),
        source_id: "local".to_owned(),
        mode: ProgramPlanMode::Local,
        created_at: CREATED_AT.to_owned(),
        segments: vec![
            ProgramPlanSegmentsItem::VoiceSegment(ProgramPlanVoiceSegment {
                r#type: "voice".to_owned(),
                segment_id: opening_id.to_string(),
                text: "欢迎回来。".to_owned(),
                trigger: ProgramPlanVoiceSegmentTrigger::Opening,
            }),
            ProgramPlanSegmentsItem::TrackSegment(ProgramPlanTrackSegment {
                r#type: "track".to_owned(),
                segment_id: track_segment_id.to_string(),
                track_id,
                segue_text: None,
            }),
        ],
    }
}
