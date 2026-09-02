use std::collections::HashSet;

use serde::Serialize;
use uuid::Uuid;

use super::ProgramError;

pub const MAX_PROGRAM_CANDIDATES: usize = 200;
pub const MAX_CANDIDATE_SOURCE_ROWS: usize = 10_000;
const MAX_CONTEXT_TAGS: usize = 100;
const MAX_TRACK_TAGS: usize = 100;
const MAX_TAG_CHARS: usize = 100;
const MAX_LABEL_CHARS: usize = 300;
const MAX_TRACK_DURATION_MS: u64 = 86_400_000;
const MAX_COOLDOWN_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const RECENT_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// Repository projection used for deterministic local candidate selection.
///
/// It intentionally has no path, audio, provider body, or user-text field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateTrack {
    pub track_id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub normalized_tags: Vec<String>,
    pub time_tags: Vec<TimeTag>,
    pub last_played_at_ms: Option<i64>,
    pub like_count: u32,
    pub skip_count: u32,
    pub playable: bool,
}

/// Coarse local-time buckets that may be attached to a track preference fact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TimeTag {
    Morning,
    Day,
    Evening,
    Night,
}

impl TimeTag {
    #[must_use]
    pub const fn from_local_hour(hour: u8) -> Option<Self> {
        match hour {
            5..=10 => Some(Self::Morning),
            11..=16 => Some(Self::Day),
            17..=21 => Some(Self::Evening),
            0..=4 | 22..=23 => Some(Self::Night),
            _ => None,
        }
    }
}

/// Fixed inputs controlling local candidate scoring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateSelectionRequest {
    pub now_ms: i64,
    pub local_hour: u8,
    pub profile_tags: Vec<String>,
    pub approved_memory_tags: Vec<String>,
    pub recently_played_track_ids: Vec<String>,
    pub cooldown_ms: i64,
    pub desired_count: usize,
    pub limit: usize,
    pub allow_cooldown_relaxation: bool,
    pub selection_seed: u64,
}

/// Minimal provider-safe track record selected by the local engine.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramCandidate {
    pub track_id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub normalized_tags: Vec<String>,
    pub recent_play_penalty: i32,
}

/// Selected candidates and whether the explicit scarcity degradation was used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateSelection {
    pub candidates: Vec<ProgramCandidate>,
    pub cooldown_relaxed: bool,
}

struct ScoredCandidate {
    candidate: ProgramCandidate,
    score: i64,
    last_played_at_ms: Option<i64>,
    tie_breaker: u64,
    cooling_down: bool,
}

/// Scores and selects a bounded candidate set without network, randomness, or
/// filesystem access.
///
/// # Errors
///
/// Rejects invalid bounds, corrupt/duplicate records, invalid `UUIDv7` IDs, or a
/// pool with no selectable playable track.
pub fn select_candidates(
    tracks: Vec<CandidateTrack>,
    request: &CandidateSelectionRequest,
) -> Result<CandidateSelection, ProgramError> {
    validate_selection_request(request)?;
    if tracks.len() > MAX_CANDIDATE_SOURCE_ROWS {
        return Err(ProgramError::CandidatePoolTooLarge);
    }
    let profile_tags = normalize_context_tags(&request.profile_tags)?;
    let memory_tags = normalize_context_tags(&request.approved_memory_tags)?;
    let recent_ids = validate_recent_ids(&request.recently_played_track_ids)?;
    let time_tag = TimeTag::from_local_hour(request.local_hour)
        .ok_or(ProgramError::InvalidSelectionRequest)?;
    let mut seen_ids = HashSet::with_capacity(tracks.len());
    let mut scored = Vec::with_capacity(tracks.len());
    for track in tracks {
        let validated = validate_candidate(track, request.now_ms, &mut seen_ids)?;
        if !validated.playable {
            continue;
        }
        scored.push(score_candidate(
            validated,
            request,
            &profile_tags,
            &memory_tags,
            &recent_ids,
            time_tag,
        ));
    }
    scored.sort_by(compare_candidates);
    build_selection(scored, request)
}

fn validate_selection_request(request: &CandidateSelectionRequest) -> Result<(), ProgramError> {
    if request.now_ms < 0
        || request.local_hour > 23
        || request.cooldown_ms < 0
        || request.cooldown_ms > MAX_COOLDOWN_MS
        || request.limit == 0
        || request.limit > MAX_PROGRAM_CANDIDATES
        || request.desired_count == 0
        || request.desired_count > request.limit
    {
        return Err(ProgramError::InvalidSelectionRequest);
    }
    Ok(())
}

fn normalize_context_tags(values: &[String]) -> Result<HashSet<String>, ProgramError> {
    if values.len() > MAX_CONTEXT_TAGS {
        return Err(ProgramError::InvalidSelectionRequest);
    }
    let mut normalized = HashSet::with_capacity(values.len());
    for value in values {
        normalized.insert(normalize_tag(value).ok_or(ProgramError::InvalidSelectionRequest)?);
    }
    Ok(normalized)
}

fn validate_recent_ids(values: &[String]) -> Result<HashSet<String>, ProgramError> {
    if values.len() > MAX_PROGRAM_CANDIDATES {
        return Err(ProgramError::InvalidSelectionRequest);
    }
    values
        .iter()
        .map(|value| {
            validate_uuid_v7(value)
                .then(|| value.clone())
                .ok_or(ProgramError::InvalidSelectionRequest)
        })
        .collect()
}

fn validate_candidate(
    mut track: CandidateTrack,
    now_ms: i64,
    seen_ids: &mut HashSet<String>,
) -> Result<CandidateTrack, ProgramError> {
    if !validate_uuid_v7(&track.track_id)
        || !seen_ids.insert(track.track_id.clone())
        || track.duration_ms == 0
        || track.duration_ms > MAX_TRACK_DURATION_MS
        || track.normalized_tags.len() > MAX_TRACK_TAGS
        || track
            .last_played_at_ms
            .is_some_and(|value| value < 0 || value > now_ms)
    {
        return Err(ProgramError::InvalidCandidateData);
    }
    validate_optional_label(track.title.as_deref())?;
    validate_optional_label(track.artist.as_deref())?;
    validate_optional_label(track.album.as_deref())?;
    let mut tags = HashSet::with_capacity(track.normalized_tags.len());
    for tag in track.normalized_tags {
        tags.insert(normalize_tag(&tag).ok_or(ProgramError::InvalidCandidateData)?);
    }
    let mut time_tags = HashSet::with_capacity(track.time_tags.len());
    for time_tag in track.time_tags {
        if !time_tags.insert(time_tag) {
            return Err(ProgramError::InvalidCandidateData);
        }
    }
    track.normalized_tags = tags.into_iter().collect();
    track.normalized_tags.sort_unstable();
    track.time_tags = time_tags.into_iter().collect();
    track.time_tags.sort_unstable_by_key(|tag| match tag {
        TimeTag::Morning => 0,
        TimeTag::Day => 1,
        TimeTag::Evening => 2,
        TimeTag::Night => 3,
    });
    Ok(track)
}

fn validate_optional_label(value: Option<&str>) -> Result<(), ProgramError> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.chars().count() > MAX_LABEL_CHARS
            || value.chars().any(char::is_control)
    }) {
        return Err(ProgramError::InvalidCandidateData);
    }
    Ok(())
}

fn normalize_tag(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() || normalized.chars().count() > MAX_TAG_CHARS {
        return None;
    }
    Some(normalized.to_lowercase())
}

fn validate_uuid_v7(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

fn score_candidate(
    track: CandidateTrack,
    request: &CandidateSelectionRequest,
    profile_tags: &HashSet<String>,
    memory_tags: &HashSet<String>,
    recent_ids: &HashSet<String>,
    time_tag: TimeTag,
) -> ScoredCandidate {
    let profile_hits = track
        .normalized_tags
        .iter()
        .filter(|tag| profile_tags.contains(*tag))
        .fold(0_i64, |count, _| count + 1);
    let memory_hits = track
        .normalized_tags
        .iter()
        .filter(|tag| memory_tags.contains(*tag))
        .fold(0_i64, |count, _| count + 1);
    let time_match = i64::from(track.time_tags.contains(&time_tag));
    let age_ms = track
        .last_played_at_ms
        .map_or(i64::MAX, |value| request.now_ms.saturating_sub(value));
    let cooling_down = request.cooldown_ms > 0 && age_ms < request.cooldown_ms;
    let recent_play_penalty = recent_play_penalty(age_ms, recent_ids.contains(&track.track_id));
    let score = profile_hits * 120
        + memory_hits * 100
        + time_match * 75
        + i64::from(track.like_count.min(20)) * 45
        - i64::from(track.skip_count.min(20)) * 70
        + i64::from(recent_play_penalty);
    let tie_breaker = seeded_tie_breaker(request.selection_seed, &track.track_id);
    ScoredCandidate {
        candidate: ProgramCandidate {
            track_id: track.track_id,
            title: track.title,
            artist: track.artist,
            album: track.album,
            duration_ms: track.duration_ms,
            normalized_tags: track.normalized_tags,
            recent_play_penalty,
        },
        score,
        last_played_at_ms: track.last_played_at_ms,
        tie_breaker,
        cooling_down,
    }
}

fn recent_play_penalty(age_ms: i64, explicitly_recent: bool) -> i32 {
    if explicitly_recent {
        return -500;
    }
    if age_ms == i64::MAX || age_ms >= RECENT_WINDOW_MS {
        return 0;
    }
    let remaining = RECENT_WINDOW_MS.saturating_sub(age_ms);
    let scaled = remaining.saturating_mul(300) / RECENT_WINDOW_MS;
    match i32::try_from(scaled) {
        Ok(value) => -value,
        Err(_) => -300,
    }
}

fn compare_candidates(left: &ScoredCandidate, right: &ScoredCandidate) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.last_played_at_ms.cmp(&right.last_played_at_ms))
        .then_with(|| left.tie_breaker.cmp(&right.tie_breaker))
        .then_with(|| left.candidate.track_id.cmp(&right.candidate.track_id))
}

fn build_selection(
    scored: Vec<ScoredCandidate>,
    request: &CandidateSelectionRequest,
) -> Result<CandidateSelection, ProgramError> {
    let mut fresh = Vec::new();
    let mut cooling = Vec::new();
    for candidate in scored {
        if candidate.cooling_down {
            cooling.push(candidate.candidate);
        } else {
            fresh.push(candidate.candidate);
        }
    }
    let mut candidates = fresh;
    let mut cooldown_relaxed = false;
    if request.allow_cooldown_relaxation && candidates.len() < request.desired_count {
        let needed = request.desired_count.saturating_sub(candidates.len());
        let fresh_count = candidates.len();
        candidates.extend(cooling.into_iter().take(needed));
        cooldown_relaxed = candidates.len() > fresh_count;
    }
    candidates.truncate(request.limit);
    if candidates.is_empty() {
        return Err(ProgramError::NoPlayableCandidates);
    }
    Ok(CandidateSelection {
        candidates,
        cooldown_relaxed,
    })
}

fn seeded_tie_breaker(seed: u64, value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
