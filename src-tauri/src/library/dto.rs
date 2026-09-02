use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_TRACK_PAGE_SIZE: u32 = 200;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryRoot {
    pub root_id: Uuid,
    pub display_name: String,
    pub available: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryRootsResponse {
    pub roots: Vec<LibraryRoot>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickAndAddLibraryRootRequest {
    pub client_request_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickAndAddLibraryRootResponse {
    pub request_id: Uuid,
    pub root: Option<LibraryRoot>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveLibraryRootRequest {
    pub client_request_id: Uuid,
    pub root_id: Uuid,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryRootAck {
    pub request_id: Uuid,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackSort {
    Title,
    Artist,
    Album,
    Recent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackAvailabilityFilter {
    Playable,
    Missing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackMatchStatus {
    Matched,
    Unmatched,
    Review,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackFilters {
    pub availability: Option<TrackAvailabilityFilter>,
    pub match_status: Option<TrackMatchStatus>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListTracksRequest {
    pub cursor: Option<String>,
    pub limit: u32,
    pub query: Option<String>,
    pub sort: TrackSort,
    pub filters: TrackFilters,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackAvailability {
    Playable,
    Missing,
    Corrupt,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackTagView {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrichedTrackTagView {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub provider: TrackMetadataProvider,
    pub confidence: f64,
    pub fetched_at: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackMetadataProvider {
    Musicbrainz,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackView {
    pub track_id: Uuid,
    pub availability: TrackAvailability,
    pub duration_ms: u64,
    pub artwork_available: bool,
    pub original: TrackTagView,
    pub enriched: Option<EnrichedTrackTagView>,
    pub match_status: TrackMatchStatus,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TracksPage {
    pub items: Vec<TrackView>,
    pub next_cursor: Option<String>,
}
