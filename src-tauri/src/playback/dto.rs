use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{contracts::PlaybackState, ipc::SourceSummary};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectMusicSourceRequest {
    pub client_request_id: Uuid,
    pub source_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectMusicSourceResponse {
    pub request_id: Uuid,
    pub state: PlaybackState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaybackControlRequest {
    pub client_request_id: Uuid,
    pub expected_state_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeekPlaybackRequest {
    pub client_request_id: Uuid,
    pub expected_state_revision: u64,
    pub position_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListMusicSourcesResponse {
    pub sources: Vec<SourceSummary>,
}
