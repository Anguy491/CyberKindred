use std::collections::HashSet;

use chrono::DateTime;
use uuid::Uuid;

use crate::contracts::{
    ContractRegistry, ProgramPlan, ProgramPlanMode, ProgramPlanSegmentsItem,
    ProgramPlanVoiceSegmentTrigger,
};

use super::ProgramError;

/// Expected immutable facts used to validate an untrusted provider plan.
pub struct LocalPlanExpectation<'a> {
    pub program_id: Uuid,
    pub source_id: &'a str,
    pub created_at: &'a str,
    pub candidate_track_ids: &'a HashSet<String>,
}

/// Applies the checked-in schema and local planning invariants to an untrusted
/// `ProgramPlan`.
///
/// # Errors
///
/// Rejects malformed schema fields, mismatched immutable facts, unknown or
/// duplicate track IDs, non-UUIDv7 identifiers, and invalid narration spacing.
pub fn validate_local_program_plan(
    plan: &ProgramPlan,
    expected: &LocalPlanExpectation<'_>,
) -> Result<(), ProgramError> {
    let registry = ContractRegistry::new().map_err(|_| ProgramError::InvalidProgramPlan)?;
    let document = serde_json::to_value(plan).map_err(|_| ProgramError::InvalidProgramPlan)?;
    registry
        .validate("program-plan", &document)
        .map_err(|_| ProgramError::InvalidProgramPlan)?;
    validate_plan_header(plan, expected)?;
    validate_segments(plan, expected.candidate_track_ids)
}

fn validate_plan_header(
    plan: &ProgramPlan,
    expected: &LocalPlanExpectation<'_>,
) -> Result<(), ProgramError> {
    if plan.schema_version != "1.0.0"
        || plan.mode != ProgramPlanMode::Local
        || plan.program_id != expected.program_id.to_string()
        || plan.source_id != expected.source_id
        || plan.created_at != expected.created_at
        || !valid_uuid_v7(&plan.program_id)
        || DateTime::parse_from_rfc3339(&plan.created_at).is_err()
    {
        return Err(ProgramError::InvalidProgramPlan);
    }
    Ok(())
}

fn validate_segments(
    plan: &ProgramPlan,
    candidate_track_ids: &HashSet<String>,
) -> Result<(), ProgramError> {
    let mut segment_ids = HashSet::with_capacity(plan.segments.len());
    let mut track_ids = HashSet::with_capacity(plan.segments.len());
    let mut tracks_since_voice = 0_usize;
    let mut opening_count = 0_usize;
    for (index, segment) in plan.segments.iter().enumerate() {
        match segment {
            ProgramPlanSegmentsItem::TrackSegment(track) => {
                if track.r#type != "track"
                    || !valid_uuid_v7(&track.segment_id)
                    || !segment_ids.insert(track.segment_id.as_str())
                    || !valid_uuid_v7(&track.track_id)
                    || !candidate_track_ids.contains(&track.track_id)
                    || !track_ids.insert(track.track_id.as_str())
                {
                    return Err(ProgramError::InvalidProgramPlan);
                }
                tracks_since_voice += 1;
                if tracks_since_voice > 3 {
                    return Err(ProgramError::InvalidProgramPlan);
                }
            }
            ProgramPlanSegmentsItem::VoiceSegment(voice) => {
                if voice.r#type != "voice"
                    || !valid_uuid_v7(&voice.segment_id)
                    || !segment_ids.insert(voice.segment_id.as_str())
                {
                    return Err(ProgramError::InvalidProgramPlan);
                }
                match voice.trigger {
                    ProgramPlanVoiceSegmentTrigger::Opening if index == 0 => {
                        opening_count += 1;
                        tracks_since_voice = 0;
                    }
                    ProgramPlanVoiceSegmentTrigger::BetweenTracks
                        if (2..=3).contains(&tracks_since_voice) =>
                    {
                        tracks_since_voice = 0;
                    }
                    _ => return Err(ProgramError::InvalidProgramPlan),
                }
            }
        }
    }
    if opening_count != 1 || tracks_since_voice == 0 {
        return Err(ProgramError::InvalidProgramPlan);
    }
    Ok(())
}

fn valid_uuid_v7(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}
