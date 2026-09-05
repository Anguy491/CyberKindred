use std::{
    collections::HashMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    DataCategoryInventory, DataDeletionCategory, DataInventoryCategory, DataStorageClass,
    DeleteAllUserDataRequest, DeleteAllUserDataResponse, DeleteDataCategoryRequest,
    DeleteDataCategoryResponse, DeletionControl, ExportUserDataRequest, GetDataInventoryResponse,
    PreviewDataDeletionRequest, PreviewDataDeletionResponse,
};
use crate::{
    ipc::{
        ApiError, EmptyRequest, EventEnvelope, InternalReason, ProcessSequence, PublicField,
        RequestHash, canonical_request_hash,
    },
    playback::PlaybackService,
    providers::{
        CancelOperationRequest, CancelOperationResponse, CancelOperationState, OperationAccepted,
        OperationKind,
        events::{OPERATION_CANCELLED_EVENT, OPERATION_COMPLETED_EVENT, OPERATION_FAILED_EVENT},
    },
    storage::{
        AppPaths, CacheArea, InventoryCounts, Repository, SecretError, SecretVault, Storage,
        StorageError, StorageReason,
    },
};

const PREVIEW_LIFETIME: Duration = Duration::from_mins(5);
const IDEMPOTENCY_LIFETIME: Duration = Duration::from_mins(10);
const IDEMPOTENCY_CAPACITY: usize = 256;
const CATEGORY_CONFIRMATION: &str = "DELETE SELECTED DATA";
const RESET_CONFIRMATION: &str = "DELETE CYBERKINDRED DATA";

pub type DataExportPickerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<PathBuf>, ApiError>> + Send + 'a>>;

pub trait DataExportPicker: Send + Sync {
    fn pick_destination(&self) -> DataExportPickerFuture<'_>;
}

pub struct TauriDataExportPicker {
    app_handle: AppHandle,
}

impl TauriDataExportPicker {
    #[must_use]
    pub const fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }
}

impl DataExportPicker for TauriDataExportPicker {
    fn pick_destination(&self) -> DataExportPickerFuture<'_> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.app_handle
            .dialog()
            .file()
            .set_title("导出 CyberKindred 数据")
            .set_file_name("CyberKindred-data-export.jsonl")
            .add_filter("JSON Lines", &["jsonl"])
            .save_file(move |selection| {
                let selected = selection
                    .map(|path| path.into_path().map_err(|_| path_error()))
                    .transpose();
                let _ = sender.send(selected);
            });
        Box::pin(async move { receiver.await.map_err(|_| path_error())? })
    }
}

pub trait DataExportEventSink: Send + Sync {
    fn completed(&self, operation_id: Uuid) -> Result<(), ApiError>;
    fn failed(&self, operation_id: Uuid, error: ApiError) -> Result<(), ApiError>;
    fn cancelled(&self, operation_id: Uuid) -> Result<(), ApiError>;
}

pub trait DataResetIntegration: Send + Sync {
    /// Disables app-owned Windows integrations before persistent data removal.
    ///
    /// # Errors
    ///
    /// Returns a stable error when an integration cannot be removed.
    fn reset(&self) -> Result<(), ApiError>;
}

pub struct TauriDataExportEventSink<R: Runtime> {
    app_handle: AppHandle<R>,
    sequence: Arc<ProcessSequence>,
}

impl<R: Runtime> TauriDataExportEventSink<R> {
    #[must_use]
    pub fn new(app_handle: AppHandle<R>, sequence: Arc<ProcessSequence>) -> Self {
        Self {
            app_handle,
            sequence,
        }
    }

    fn envelope(&self) -> Result<EventEnvelope, ApiError> {
        Ok(EventEnvelope::now(self.sequence.next()?))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletedEvent {
    #[serde(flatten)]
    envelope: EventEnvelope,
    operation_id: Uuid,
    kind: OperationKind,
    output_label: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailedEvent {
    #[serde(flatten)]
    envelope: EventEnvelope,
    operation_id: Uuid,
    error: ApiError,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelledEvent {
    #[serde(flatten)]
    envelope: EventEnvelope,
    operation_id: Uuid,
    kind: OperationKind,
}

impl<R: Runtime> DataExportEventSink for TauriDataExportEventSink<R> {
    fn completed(&self, operation_id: Uuid) -> Result<(), ApiError> {
        self.app_handle
            .emit(
                OPERATION_COMPLETED_EVENT,
                CompletedEvent {
                    envelope: self.envelope()?,
                    operation_id,
                    kind: OperationKind::DataExport,
                    output_label: Some("CyberKindred-data-export.jsonl".to_owned()),
                },
            )
            .map_err(|_| ApiError::unexpected())
    }

    fn failed(&self, operation_id: Uuid, error: ApiError) -> Result<(), ApiError> {
        self.app_handle
            .emit(
                OPERATION_FAILED_EVENT,
                FailedEvent {
                    envelope: self.envelope()?,
                    operation_id,
                    error,
                },
            )
            .map_err(|_| ApiError::unexpected())
    }

    fn cancelled(&self, operation_id: Uuid) -> Result<(), ApiError> {
        self.app_handle
            .emit(
                OPERATION_CANCELLED_EVENT,
                CancelledEvent {
                    envelope: self.envelope()?,
                    operation_id,
                    kind: OperationKind::DataExport,
                },
            )
            .map_err(|_| ApiError::unexpected())
    }
}

struct PreviewEntry {
    category: DataDeletionCategory,
    expires_at: Instant,
}

struct IdempotencyEntry<T> {
    request_hash: RequestHash,
    expires_at: Instant,
    result: Mutex<Option<Result<T, ApiError>>>,
}

struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<Uuid, Arc<IdempotencyEntry<T>>>>,
}

impl<T: Clone> AsyncIdempotency<T> {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(IDEMPOTENCY_CAPACITY)),
        }
    }

    async fn execute<F, Fut>(
        &self,
        request_id: Uuid,
        request_hash: RequestHash,
        operation: F,
    ) -> Result<T, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let now = Instant::now();
        let entry = {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, value| value.expires_at > now);
            if let Some(value) = entries.get(&request_id) {
                if value.request_hash != request_hash {
                    return Err(
                        ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                            .with_field(PublicField::ClientRequestId),
                    );
                }
                Arc::clone(value)
            } else {
                if entries.len() >= IDEMPOTENCY_CAPACITY {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let value = Arc::new(IdempotencyEntry {
                    request_hash,
                    expires_at: now + IDEMPOTENCY_LIFETIME,
                    result: Mutex::new(None),
                });
                entries.insert(request_id, Arc::clone(&value));
                value
            }
        };
        let mut result = entry.result.lock().await;
        if let Some(value) = result.as_ref() {
            return value.clone();
        }
        let value = operation().await;
        *result = Some(value.clone());
        value
    }
}

/// Implements the approved local data inventory, export, deletion, and reset boundaries.
pub struct DataControlService {
    repository: Repository,
    storage: Arc<Storage>,
    paths: AppPaths,
    vault: Arc<Mutex<Box<dyn SecretVault>>>,
    playback: PlaybackService,
    picker: Arc<dyn DataExportPicker>,
    events: Arc<dyn DataExportEventSink>,
    reset_integration: Arc<dyn DataResetIntegration>,
    previews: Mutex<HashMap<Uuid, PreviewEntry>>,
    exports: Arc<Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,
    export_requests: AsyncIdempotency<OperationAccepted>,
    cancel_requests: AsyncIdempotency<CancelOperationResponse>,
    category_requests: AsyncIdempotency<DeleteDataCategoryResponse>,
    reset_requests: AsyncIdempotency<DeleteAllUserDataResponse>,
}

pub struct DataControlServiceDependencies {
    pub repository: Repository,
    pub storage: Arc<Storage>,
    pub paths: AppPaths,
    pub vault: Box<dyn SecretVault>,
    pub playback: PlaybackService,
    pub picker: Arc<dyn DataExportPicker>,
    pub events: Arc<dyn DataExportEventSink>,
    pub reset_integration: Arc<dyn DataResetIntegration>,
}

impl DataControlService {
    #[must_use]
    pub fn new(dependencies: DataControlServiceDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            storage: dependencies.storage,
            paths: dependencies.paths,
            vault: Arc::new(Mutex::new(dependencies.vault)),
            playback: dependencies.playback,
            picker: dependencies.picker,
            events: dependencies.events,
            reset_integration: dependencies.reset_integration,
            previews: Mutex::new(HashMap::new()),
            exports: Arc::new(Mutex::new(HashMap::new())),
            export_requests: AsyncIdempotency::new(),
            cancel_requests: AsyncIdempotency::new(),
            category_requests: AsyncIdempotency::new(),
            reset_requests: AsyncIdempotency::new(),
        }
    }

    /// Returns one path-free entry for every approved lifecycle data class.
    ///
    /// # Errors
    ///
    /// Returns a stable credential, storage, or path error.
    pub async fn get_inventory(
        &self,
        _request: EmptyRequest,
    ) -> Result<GetDataInventoryResponse, ApiError> {
        let counts = self
            .repository
            .load_inventory_counts()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let credential_count = u64::from(
            self.vault
                .lock()
                .await
                .count_cyberkindred_namespace()
                .map_err(map_secret_error)?,
        );
        let system_count = u64::from(
            self.playback
                .list_music_sources(EmptyRequest {})
                .sources
                .iter()
                .any(|source| source.source_id == "apple_music" && source.connected),
        );
        let cache_count = count_regular_files(self.paths.cache_root()).map_err(|_| path_error())?;
        let log_count =
            count_regular_files(self.paths.log_directory()).map_err(|_| path_error())?;
        let backup_count =
            count_regular_files(&self.paths.backups_dir()).map_err(|_| path_error())?;
        Ok(GetDataInventoryResponse {
            generated_at: now_rfc3339(),
            categories: inventory_rows(
                counts,
                credential_count,
                system_count,
                cache_count,
                log_count,
                backup_count,
            ),
        })
    }

    /// Creates one five-minute token for a fixed deletion category.
    ///
    /// # Errors
    ///
    /// Returns a stable storage error when the preview count cannot be read.
    pub async fn preview_deletion(
        &self,
        request: PreviewDataDeletionRequest,
    ) -> Result<PreviewDataDeletionResponse, ApiError> {
        let item_count = self
            .repository
            .category_count(request.category.as_str())
            .await
            .map_err(|error| map_storage_error(&error))?;
        let preview_token = Uuid::now_v7();
        let expires_at = Utc::now() + chrono::Duration::minutes(5);
        let mut previews = self.previews.lock().await;
        let now = Instant::now();
        previews.retain(|_, entry| entry.expires_at > now);
        previews.insert(
            preview_token,
            PreviewEntry {
                category: request.category,
                expires_at: now + PREVIEW_LIFETIME,
            },
        );
        Ok(PreviewDataDeletionResponse {
            preview_token,
            expires_at: expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
            category: request.category,
            item_count,
            consequences: consequences(request.category),
        })
    }

    /// Deletes one previewed category with exact confirmation and idempotency.
    ///
    /// # Errors
    ///
    /// Returns a stable confirmation, token, storage, path, or conflict error.
    pub async fn delete_category(
        &self,
        request: DeleteDataCategoryRequest,
    ) -> Result<DeleteDataCategoryResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        self.category_requests
            .execute(request.client_request_id, hash, || async {
                self.delete_category_once(request).await
            })
            .await
    }

    async fn delete_category_once(
        &self,
        request: DeleteDataCategoryRequest,
    ) -> Result<DeleteDataCategoryResponse, ApiError> {
        require_confirmation(&request.confirmation, CATEGORY_CONFIRMATION)?;
        let entry = self
            .previews
            .lock()
            .await
            .remove(&request.preview_token)
            .ok_or_else(|| ApiError::from_reason(InternalReason::PreviewTokenStale))?;
        if entry.expires_at <= Instant::now() || entry.category != request.category {
            return Err(ApiError::from_reason(InternalReason::PreviewTokenStale));
        }
        let deleted_count = self
            .repository
            .delete_category(request.category.as_str())
            .await
            .map_err(|error| map_storage_error(&error))?;
        if request.category == DataDeletionCategory::MetadataCache {
            clear_directory_contents(self.paths.cache_root()).map_err(|_| path_error())?;
            for area in [CacheArea::Tts, CacheArea::Covers, CacheArea::Staging] {
                fs::create_dir_all(self.paths.cache_dir(area)).map_err(|_| path_error())?;
            }
        }
        clear_directory_contents(&self.paths.backups_dir()).map_err(|_| path_error())?;
        Ok(DeleteDataCategoryResponse {
            request_id: request.client_request_id,
            category: request.category,
            deleted_count,
            restart_required: false,
        })
    }

    /// Accepts a cancellable export and returns before the native picker completes.
    ///
    /// # Errors
    ///
    /// Returns a stable validation, idempotency, or capacity error.
    pub async fn export_user_data(
        &self,
        request: ExportUserDataRequest,
    ) -> Result<OperationAccepted, ApiError> {
        let hash = canonical_request_hash(&request)?;
        self.export_requests
            .execute(request.client_request_id, hash, || async {
                let accepted = OperationAccepted {
                    operation_id: Uuid::now_v7(),
                    accepted_at: now_rfc3339(),
                };
                let cancelled = Arc::new(AtomicBool::new(false));
                self.exports
                    .lock()
                    .await
                    .insert(accepted.operation_id, Arc::clone(&cancelled));
                let worker = ExportWorker {
                    repository: self.repository.clone(),
                    paths: self.paths.clone(),
                    picker: Arc::clone(&self.picker),
                    events: Arc::clone(&self.events),
                    exports: Arc::clone(&self.exports),
                };
                let operation_id = accepted.operation_id;
                tauri::async_runtime::spawn(async move {
                    worker.run(operation_id, cancelled).await;
                });
                Ok(accepted)
            })
            .await
    }

    /// Cancels an accepted export if it has not reached a terminal state.
    ///
    /// # Errors
    ///
    /// Returns a stable capability error for a mismatched operation kind.
    pub async fn cancel_export(
        &self,
        request: CancelOperationRequest,
    ) -> Result<CancelOperationResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        self.cancel_requests
            .execute(request.client_request_id, hash, || async {
                if request.expected_kind != OperationKind::DataExport {
                    return Err(ApiError::from_reason(InternalReason::CapabilityAbsent));
                }
                let exports = self.exports.lock().await;
                let state = if let Some(cancelled) = exports.get(&request.operation_id) {
                    cancelled.store(true, Ordering::Release);
                    CancelOperationState::Cancelled
                } else {
                    CancelOperationState::AlreadyTerminal
                };
                Ok(CancelOperationResponse {
                    request_id: request.client_request_id,
                    operation_id: request.operation_id,
                    state,
                })
            })
            .await
    }

    /// Removes app-owned user data after the independent exact confirmation.
    ///
    /// # Errors
    ///
    /// Returns a stable confirmation, credential, storage, or path error and
    /// never reports success after a partial failure.
    pub async fn delete_all_user_data(
        &self,
        request: DeleteAllUserDataRequest,
    ) -> Result<DeleteAllUserDataResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        self.reset_requests
            .execute(request.client_request_id, hash, || async {
                self.delete_all_once(request).await
            })
            .await
    }

    async fn delete_all_once(
        &self,
        request: DeleteAllUserDataRequest,
    ) -> Result<DeleteAllUserDataResponse, ApiError> {
        require_confirmation(&request.confirmation, RESET_CONFIRMATION)?;
        for cancelled in self.exports.lock().await.values() {
            cancelled.store(true, Ordering::Release);
        }
        self.reset_integration.reset()?;
        self.vault
            .lock()
            .await
            .delete_cyberkindred_namespace()
            .map_err(map_secret_error)?;
        self.storage.close_ref().await;
        let database = self.paths.database_path();
        for path in [
            database.clone(),
            sqlite_sidecar(&database, "-wal"),
            sqlite_sidecar(&database, "-shm"),
        ] {
            remove_file_if_present(&path).map_err(|_| path_error())?;
        }
        clear_directory_contents(&self.paths.backups_dir()).map_err(|_| path_error())?;
        clear_directory_contents(&self.paths.exports_dir()).map_err(|_| path_error())?;
        clear_directory_contents(self.paths.cache_root()).map_err(|_| path_error())?;
        clear_directory_contents(self.paths.log_directory()).map_err(|_| path_error())?;
        Ok(DeleteAllUserDataResponse {
            request_id: request.client_request_id,
            restart_required: true,
        })
    }
}

struct ExportWorker {
    repository: Repository,
    paths: AppPaths,
    picker: Arc<dyn DataExportPicker>,
    events: Arc<dyn DataExportEventSink>,
    exports: Arc<Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,
}

impl ExportWorker {
    async fn run(self, operation_id: Uuid, cancelled: Arc<AtomicBool>) {
        let temporary = self
            .paths
            .exports_dir()
            .join(format!("export-{operation_id}.jsonl"));
        let result = self.run_inner(&temporary, &cancelled).await;
        let _cleanup = remove_file_if_present(&temporary);
        self.exports.lock().await.remove(&operation_id);
        match result {
            Ok(ExportDisposition::Completed) => {
                let _published = self.events.completed(operation_id);
            }
            Ok(ExportDisposition::Cancelled) => {
                let _published = self.events.cancelled(operation_id);
            }
            Err(error) => {
                let _published = self.events.failed(operation_id, error);
            }
        }
    }

    async fn run_inner(
        &self,
        temporary: &Path,
        cancelled: &AtomicBool,
    ) -> Result<ExportDisposition, ApiError> {
        if cancelled.load(Ordering::Acquire) {
            return Ok(ExportDisposition::Cancelled);
        }
        let bytes = self
            .repository
            .export_user_data_jsonl()
            .await
            .map_err(|error| map_storage_error(&error))?;
        fs::write(temporary, bytes).map_err(|_| path_error())?;
        if cancelled.load(Ordering::Acquire) {
            return Ok(ExportDisposition::Cancelled);
        }
        let Some(destination) = self.picker.pick_destination().await? else {
            return Ok(ExportDisposition::Cancelled);
        };
        if cancelled.load(Ordering::Acquire) {
            return Ok(ExportDisposition::Cancelled);
        }
        fs::copy(temporary, destination).map_err(|_| path_error())?;
        Ok(ExportDisposition::Completed)
    }
}

enum ExportDisposition {
    Completed,
    Cancelled,
}

#[allow(clippy::too_many_lines)] // The 19 approved lifecycle rows stay visible and ordered.
fn inventory_rows(
    counts: InventoryCounts,
    credentials: u64,
    system_media: u64,
    cache_files: u64,
    log_files: u64,
    backup_files: u64,
) -> Vec<DataCategoryInventory> {
    use DataDeletionCategory as Delete;
    use DataInventoryCategory as Category;
    use DataStorageClass as Store;
    use DeletionControl as Control;

    let row = |category: DataInventoryCategory,
               item_count: u64,
               storage_classes: Vec<DataStorageClass>,
               retention: &str,
               recipients: Vec<&str>,
               control: DeletionControl,
               deletion: Option<DataDeletionCategory>| {
        DataCategoryInventory {
            category,
            item_count,
            storage_classes,
            retention_summary: retention.to_owned(),
            external_recipients: recipients.into_iter().map(str::to_owned).collect(),
            deletion_control: control,
            deletion_category: deletion,
        }
    };
    vec![
        row(
            Category::Credentials,
            credentials,
            vec![Store::CredentialManager],
            "保留到按来源删除或全部重置",
            vec!["用户确认的 OpenAI-compatible origin"],
            Control::Credential,
            None,
        ),
        row(
            Category::ProfileAndPreferences,
            counts.profile_and_preferences,
            vec![Store::Sqlite],
            "保留到编辑、分类删除或全部重置",
            vec!["用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::ProfileAndMemories),
        ),
        row(
            Category::WeatherLocationAndCache,
            counts.weather_location_and_cache,
            vec![Store::Memory, Store::Sqlite],
            "搜索候选 10 分钟；天气缓存最多 7 天",
            vec!["Open-Meteo", "用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::ProfileAndMemories),
        ),
        row(
            Category::LibraryRootsAndIdentity,
            counts.library_roots_and_identity,
            vec![Store::Sqlite],
            "保留到移除曲库索引或全部重置",
            vec![],
            Control::Category,
            Some(Delete::LibraryIndex),
        ),
        row(
            Category::EmbeddedMusicTags,
            counts.embedded_music_tags,
            vec![Store::Sqlite],
            "跟随曲库索引",
            vec!["MusicBrainz", "用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::LibraryIndex),
        ),
        row(
            Category::MetadataMatches,
            counts.metadata_matches,
            vec![Store::Sqlite],
            "成功结果保留到刷新；未匹配 24 小时",
            vec!["MusicBrainz", "Cover Art Archive"],
            Control::Category,
            Some(Delete::MetadataCache),
        ),
        row(
            Category::ArtworkAndTtsCache,
            counts.artwork_and_tts_cache.saturating_add(cache_files),
            vec![Store::Sqlite, Store::AppCache],
            "成功缓存最多 30 天；暂存 24 小时",
            vec!["Cover Art Archive", "用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::MetadataCache),
        ),
        row(
            Category::SystemMediaRuntime,
            system_media,
            vec![Store::Memory],
            "只保留当前进程状态",
            vec![],
            Control::Automatic,
            None,
        ),
        row(
            Category::PlaybackHistoryAndFeedback,
            counts.playback_history_and_feedback,
            vec![Store::Sqlite],
            "保留到分类删除或全部重置",
            vec!["用户确认的 OpenAI-compatible origin（仅本地来源聚合）"],
            Control::Category,
            Some(Delete::PlaybackHistory),
        ),
        row(
            Category::ChatMessages,
            counts.chat_messages,
            vec![Store::Sqlite],
            "创建后固定最多 30 天",
            vec!["用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::ConversationsAndSummaries),
        ),
        row(
            Category::VoiceSegmentText,
            counts.voice_segment_text,
            vec![Store::Sqlite],
            "节目段结束后最多 30 天",
            vec!["用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::PlaybackHistory),
        ),
        row(
            Category::SessionSummaries,
            counts.session_summaries,
            vec![Store::Sqlite],
            "保留到删除对应会话、摘要或全部重置",
            vec!["用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::ConversationsAndSummaries),
        ),
        row(
            Category::MemoryProposals,
            counts.memory_proposals,
            vec![Store::Sqlite],
            "待审 90 天；拒绝正文 30 天",
            vec![],
            Control::Category,
            Some(Delete::ProfileAndMemories),
        ),
        row(
            Category::ApprovedMemoriesAndRevisions,
            counts.approved_memories_and_revisions,
            vec![Store::Sqlite],
            "保留到禁用、删除或全部重置",
            vec!["用户确认的 OpenAI-compatible origin"],
            Control::Category,
            Some(Delete::ProfileAndMemories),
        ),
        row(
            Category::SchedulesAndNotifications,
            counts.schedules_and_notifications,
            vec![
                Store::Sqlite,
                Store::WindowsTask,
                Store::WindowsNotification,
            ],
            "保留到删除规则或全部重置",
            vec![],
            Control::ResetOnly,
            None,
        ),
        row(
            Category::ProviderUsageFacts,
            counts.provider_usage_facts,
            vec![Store::Sqlite],
            "最多保留 13 个月",
            vec![],
            Control::Automatic,
            None,
        ),
        row(
            Category::OperationOutbox,
            counts.operation_outbox,
            vec![Store::Sqlite],
            "已投递 24 小时；未投递最多 7 天",
            vec![],
            Control::Automatic,
            None,
        ),
        row(
            Category::DiagnosticLogs,
            log_files,
            vec![Store::AppData],
            "滚动保留最多 14 天",
            vec![],
            Control::ResetOnly,
            None,
        ),
        row(
            Category::MigrationBackups,
            backup_files,
            vec![Store::AppData],
            "最近 3 个成功备份且最多 30 天",
            vec![],
            Control::Automatic,
            None,
        ),
    ]
}

fn consequences(category: DataDeletionCategory) -> Vec<String> {
    let messages = match category {
        DataDeletionCategory::ProfileAndMemories => [
            "画像、位置、偏好与全部记忆将被删除。",
            "旧迁移备份会一并删除。",
        ],
        DataDeletionCategory::ConversationsAndSummaries => {
            ["对话、摘要及其来源映射将被删除。", "旧迁移备份会一并删除。"]
        }
        DataDeletionCategory::PlaybackHistory => [
            "节目、串场文字、播放历史与反馈将被删除。",
            "音乐文件不会被删除。",
        ],
        DataDeletionCategory::MetadataCache => [
            "元数据、封面、TTS、天气及临时缓存将被删除。",
            "音乐文件不会被删除。",
        ],
        DataDeletionCategory::LibraryIndex => [
            "曲库授权、扫描记录、标签索引与文件身份将被删除。",
            "源音乐文件不会被删除。",
        ],
    };
    messages.into_iter().map(str::to_owned).collect()
}

fn require_confirmation(value: &str, expected: &str) -> Result<(), ApiError> {
    if value.is_empty() {
        return Err(ApiError::from_reason(
            InternalReason::DestructiveConfirmationMissing,
        ));
    }
    if value != expected {
        return Err(ApiError::from_reason(
            InternalReason::DestructiveConfirmationMismatch,
        ));
    }
    Ok(())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn map_storage_error(error: &StorageError) -> ApiError {
    let reason = match error.reason() {
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
    };
    ApiError::from_reason(reason)
}

fn map_secret_error(error: SecretError) -> ApiError {
    match error {
        SecretError::Unavailable => ApiError::from_reason(InternalReason::ResourceBusy),
        SecretError::InvalidOrigin | SecretError::InvalidValue => {
            ApiError::from_reason(InternalReason::RequestInvalid)
        }
        SecretError::OperationFailed
        | SecretError::ReplacementFailedRestored
        | SecretError::CredentialStateUnknown => {
            ApiError::from_reason(InternalReason::StorageWriteFailed)
        }
    }
}

fn path_error() -> ApiError {
    ApiError::from_reason(InternalReason::PathDenied)
}

fn count_regular_files(root: &Path) -> std::io::Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0_u64;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if is_link_like(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            count = count.saturating_add(count_regular_files(&entry.path())?);
        } else if metadata.is_file() {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

fn clear_directory_contents(root: &Path) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        remove_owned_entry(&entry?.path())?;
    }
    Ok(())
}

fn remove_owned_entry(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link_like(&metadata) {
        return fs::remove_file(path).or_else(|_| fs::remove_dir(path));
    }
    if metadata.is_dir() {
        clear_directory_contents(path)?;
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(windows)]
fn is_link_like(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link_like(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn remove_file_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sqlite_sidecar(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_inventory_has_all_nineteen_ordered_path_free_classes() {
        let rows = inventory_rows(InventoryCounts::default(), 0, 0, 0, 0, 0);
        assert_eq!(rows.len(), 19);
        assert_eq!(rows[0].category, DataInventoryCategory::Credentials);
        assert_eq!(rows[18].category, DataInventoryCategory::MigrationBackups);
        assert!(rows.iter().all(|row| !row.storage_classes.is_empty()));
        let encoded = serde_json::to_string(&rows).expect("inventory JSON");
        assert!(!encoded.contains(r"C:\"));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn data_deletion_confirmations_are_exact_and_independent() {
        assert!(require_confirmation(CATEGORY_CONFIRMATION, CATEGORY_CONFIRMATION).is_ok());
        assert!(require_confirmation(RESET_CONFIRMATION, RESET_CONFIRMATION).is_ok());
        assert!(require_confirmation(RESET_CONFIRMATION, CATEGORY_CONFIRMATION).is_err());
        assert!(require_confirmation("delete cyberkindred data", RESET_CONFIRMATION).is_err());
        assert!(require_confirmation("", RESET_CONFIRMATION).is_err());
    }

    #[test]
    fn data_reset_cleanup_removes_only_the_explicit_owned_root() {
        let temp = tempfile::tempdir().expect("temporary root");
        let owned = temp.path().join("owned-cache");
        let source_music = temp.path().join("source-music.mp3");
        fs::create_dir_all(owned.join("nested")).expect("cache tree");
        fs::write(owned.join("nested").join("private-cache"), b"cache").expect("cache file");
        fs::write(&source_music, b"source music").expect("source music");

        clear_directory_contents(&owned).expect("clear owned cache");

        assert_eq!(count_regular_files(&owned).expect("count"), 0);
        assert!(source_music.exists(), "source music is out of reset scope");
    }
}
