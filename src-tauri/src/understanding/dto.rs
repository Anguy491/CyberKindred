use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{contracts::MemoryRecord, providers::WeatherLocation};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitChatRequest {
    pub client_request_id: Uuid,
    pub program_id: Uuid,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Like,
    Skip,
    LessTalk,
}

impl FeedbackKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Skip => "skip",
            Self::LessTalk => "less_talk",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitFeedbackRequest {
    pub client_request_id: Uuid,
    pub program_id: Uuid,
    pub track_id: Option<Uuid>,
    pub kind: FeedbackKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageRequest {
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryListStatus {
    Proposed,
    Approved,
    Disabled,
}

impl MemoryListStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListMemoriesRequest {
    pub cursor: Option<String>,
    pub limit: u32,
    pub status: Option<MemoryListStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPage {
    pub items: Vec<MemoryRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryMutationRequest {
    pub client_request_id: Uuid,
    pub memory_id: Uuid,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateMemoryRequest {
    pub client_request_id: Uuid,
    pub memory_id: Uuid,
    pub expected_revision: u64,
    pub content: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RejectMemoryResponse {
    pub request_id: Uuid,
    pub memory_id: Uuid,
    pub status: RejectedStatus,
    pub rejected_at: String,
    pub content_delete_at: String,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectedStatus {
    Rejected,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserProfileView {
    pub display_name: String,
    pub companion_style: CompanionStyle,
    pub initial_preferences: Vec<String>,
    pub narration_density: NarrationDensity,
    pub weather_location: Option<WeatherLocation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionStyle {
    QuietWarm,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationDensity {
    Quiet,
    Balanced,
    Frequent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Up,
    Stable,
    Down,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreferenceTrend {
    pub kind: String,
    pub label: String,
    pub direction: TrendDirection,
    pub sample_count: u32,
    pub window_days: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileViewResponse {
    pub profile: UserProfileView,
    pub preference_trends: Vec<PreferenceTrend>,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfilePatch {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub companion_style: Option<CompanionStyle>,
    #[serde(default)]
    pub initial_preferences: Option<Vec<String>>,
    #[serde(default)]
    pub narration_density: Option<NarrationDensity>,
}

impl ProfilePatch {
    pub(crate) const fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.companion_style.is_none()
            && self.initial_preferences.is_none()
            && self.narration_density.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateProfileRequest {
    pub client_request_id: Uuid,
    pub expected_revision: u64,
    pub patch: ProfilePatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionSummaryView {
    pub summary_id: Uuid,
    pub covered_from: String,
    pub covered_to: String,
    pub summary: String,
    pub generation_kind: SummaryGenerationKind,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryGenerationKind {
    Llm,
    Deterministic,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionSummaryPage {
    pub items: Vec<SessionSummaryView>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteSummaryRequest {
    pub client_request_id: Uuid,
    pub summary_id: Uuid,
    pub expected_revision: u64,
}
