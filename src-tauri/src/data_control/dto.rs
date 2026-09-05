use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataDeletionCategory {
    ProfileAndMemories,
    ConversationsAndSummaries,
    PlaybackHistory,
    MetadataCache,
    LibraryIndex,
}

impl DataDeletionCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProfileAndMemories => "profile_and_memories",
            Self::ConversationsAndSummaries => "conversations_and_summaries",
            Self::PlaybackHistory => "playback_history",
            Self::MetadataCache => "metadata_cache",
            Self::LibraryIndex => "library_index",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataInventoryCategory {
    Credentials,
    ProfileAndPreferences,
    WeatherLocationAndCache,
    LibraryRootsAndIdentity,
    EmbeddedMusicTags,
    MetadataMatches,
    ArtworkAndTtsCache,
    SystemMediaRuntime,
    PlaybackHistoryAndFeedback,
    ChatMessages,
    VoiceSegmentText,
    SessionSummaries,
    MemoryProposals,
    ApprovedMemoriesAndRevisions,
    SchedulesAndNotifications,
    ProviderUsageFacts,
    OperationOutbox,
    DiagnosticLogs,
    MigrationBackups,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataStorageClass {
    Memory,
    CredentialManager,
    Sqlite,
    AppData,
    AppCache,
    WindowsTask,
    WindowsNotification,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionControl {
    Category,
    Credential,
    Automatic,
    ResetOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCategoryInventory {
    pub category: DataInventoryCategory,
    pub item_count: u64,
    pub storage_classes: Vec<DataStorageClass>,
    pub retention_summary: String,
    pub external_recipients: Vec<String>,
    pub deletion_control: DeletionControl,
    pub deletion_category: Option<DataDeletionCategory>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetDataInventoryResponse {
    pub generated_at: String,
    pub categories: Vec<DataCategoryInventory>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewDataDeletionRequest {
    pub category: DataDeletionCategory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewDataDeletionResponse {
    pub preview_token: Uuid,
    pub expires_at: String,
    pub category: DataDeletionCategory,
    pub item_count: u64,
    pub consequences: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteDataCategoryRequest {
    pub client_request_id: Uuid,
    pub preview_token: Uuid,
    pub category: DataDeletionCategory,
    pub confirmation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteDataCategoryResponse {
    pub request_id: Uuid,
    pub category: DataDeletionCategory,
    pub deleted_count: u64,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportUserDataRequest {
    pub client_request_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAllUserDataRequest {
    pub client_request_id: Uuid,
    pub confirmation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAllUserDataResponse {
    pub request_id: Uuid,
    pub restart_required: bool,
}
