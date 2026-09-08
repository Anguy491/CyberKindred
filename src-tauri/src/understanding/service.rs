use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, Utc};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

use crate::{
    contracts::{MemoryRecord, MemoryRecordKind, MemoryRecordStatus},
    ipc::{ApiError, InternalReason, PublicField, RequestHash, canonical_request_hash},
    providers::{
        Ack, CancelOperationResponse, CancelOperationState, CancellationFlag, Clock,
        OperationAccepted, ProviderCallContext, WeatherLocation,
    },
    storage::{
        NewMemoryProposal, Repository, StorageError, StorageReason, StoredMemory, StoredMemoryKind,
        StoredMemoryStatus,
    },
};

use super::{
    ChatEventSink, ChatProvider, ChatProviderError, CompanionStyle, DeleteSummaryRequest,
    FeedbackKind, ListMemoriesRequest, MemoryMutationRequest, MemoryPage, NarrationDensity,
    PageRequest, PreferenceTrend, ProfileViewResponse, RejectMemoryResponse, RejectedStatus,
    SessionSummaryPage, SessionSummaryView, SubmitChatRequest, SubmitFeedbackRequest,
    SummaryGenerationKind, TrendDirection, UpdateMemoryRequest, UpdateProfileRequest,
    UserProfileView,
};

pub const DEGRADED_CHAT_MESSAGE: &str = "现在无法使用 AI 对话；本地电台、播放控制和反馈仍然可用。";
const CHAT_TIMEOUT: Duration = Duration::from_secs(60);
const REJECTED_CONTENT_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const IDEMPOTENCY_WINDOW: Duration = Duration::from_mins(10);
const IDEMPOTENCY_CAPACITY: usize = 256;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct IdempotencyKey {
    command: &'static str,
    client_request_id: Uuid,
}

struct IdempotencyEntry<T> {
    request_hash: RequestHash,
    expires_at: Instant,
    result: Mutex<Option<Result<T, ApiError>>>,
}

struct AsyncIdempotency<T> {
    entries: Mutex<HashMap<IdempotencyKey, Arc<IdempotencyEntry<T>>>>,
}

impl<T: Clone> AsyncIdempotency<T> {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(IDEMPOTENCY_CAPACITY)),
        }
    }

    async fn execute<F, Fut>(
        &self,
        command: &'static str,
        client_request_id: Uuid,
        request_hash: RequestHash,
        operation: F,
    ) -> Result<T, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let now = Instant::now();
        let key = IdempotencyKey {
            command,
            client_request_id,
        };
        let entry = {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, stored| stored.expires_at > now);
            if let Some(stored) = entries.get(&key) {
                if stored.request_hash != request_hash {
                    return Err(
                        ApiError::from_reason(InternalReason::IdempotencyPayloadConflict)
                            .with_field(PublicField::ClientRequestId),
                    );
                }
                Arc::clone(stored)
            } else {
                if entries.len() >= IDEMPOTENCY_CAPACITY {
                    return Err(ApiError::from_reason(InternalReason::ResourceBusy));
                }
                let stored = Arc::new(IdempotencyEntry {
                    request_hash,
                    expires_at: now + IDEMPOTENCY_WINDOW,
                    result: Mutex::new(None),
                });
                entries.insert(key, Arc::clone(&stored));
                stored
            }
        };
        let mut stored_result = entry.result.lock().await;
        if let Some(result) = stored_result.as_ref() {
            return result.clone();
        }
        let result = operation().await;
        *stored_result = Some(result.clone());
        result
    }
}

enum ChatState {
    Active,
    Terminal,
}

struct ChatOperation {
    program_id: Uuid,
    cancellation: CancellationFlag,
    state: Mutex<ChatState>,
    completion: Arc<OperationCompletion>,
}

struct OperationCompletion {
    finished: AtomicBool,
    notify: Notify,
}

impl OperationCompletion {
    fn new() -> Self {
        Self {
            finished: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.finished.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

struct CompletionGuard(Arc<OperationCompletion>);

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }
}

struct ServiceState {
    operations: Mutex<HashMap<Uuid, Arc<ChatOperation>>>,
    admission: Mutex<()>,
    submit_requests: AsyncIdempotency<OperationAccepted>,
    cancel_requests: AsyncIdempotency<CancelOperationResponse>,
    ack_requests: AsyncIdempotency<Ack>,
    memory_requests: AsyncIdempotency<MemoryRecord>,
    reject_requests: AsyncIdempotency<RejectMemoryResponse>,
    accepting: AtomicBool,
}

/// Rust-owned M4 service. Construction and read methods perform no provider call.
pub struct UnderstandingService {
    repository: Repository,
    provider: Option<Arc<dyn ChatProvider>>,
    events: Arc<dyn ChatEventSink>,
    clock: Arc<dyn Clock>,
    state: Arc<ServiceState>,
}

impl UnderstandingService {
    pub(crate) fn new(
        repository: Repository,
        provider: Option<Arc<dyn ChatProvider>>,
        events: Arc<dyn ChatEventSink>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            provider,
            events,
            clock,
            state: Arc::new(ServiceState {
                operations: Mutex::new(HashMap::new()),
                admission: Mutex::new(()),
                submit_requests: AsyncIdempotency::new(),
                cancel_requests: AsyncIdempotency::new(),
                ack_requests: AsyncIdempotency::new(),
                memory_requests: AsyncIdempotency::new(),
                reject_requests: AsyncIdempotency::new(),
                accepting: AtomicBool::new(true),
            }),
        }
    }

    pub async fn submit_chat(
        &self,
        request: SubmitChatRequest,
    ) -> Result<OperationAccepted, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .submit_requests
            .execute("api_v1_submit_chat", request_id, request_hash, || {
                self.submit_chat_once(request)
            })
            .await
    }

    async fn submit_chat_once(
        &self,
        request: SubmitChatRequest,
    ) -> Result<OperationAccepted, ApiError> {
        let _admission = self.state.admission.lock().await;
        if !self.state.accepting.load(Ordering::Acquire) {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        validate_chat_text(&request.text)?;
        let now = self.clock.now();
        let accepted_chat = self
            .repository
            .accept_chat_message(
                request.program_id,
                &request.text,
                requests_less_talk(&request.text),
                now.timestamp_millis(),
            )
            .await
            .map_err(map_storage_error)?;
        let operation_id = Uuid::now_v7();
        let accepted = OperationAccepted {
            operation_id,
            accepted_at: format_time(now),
        };
        let operation = Arc::new(ChatOperation {
            program_id: request.program_id,
            cancellation: CancellationFlag::default(),
            state: Mutex::new(ChatState::Active),
            completion: Arc::new(OperationCompletion::new()),
        });
        self.state
            .operations
            .lock()
            .await
            .insert(operation_id, Arc::clone(&operation));
        self.events.user_message(
            operation_id,
            request.program_id,
            &request.text,
            &accepted.accepted_at,
        )?;

        let repository = self.repository.clone();
        let provider = self.provider.clone();
        let events = Arc::clone(&self.events);
        let clock = Arc::clone(&self.clock);
        let completion = Arc::clone(&operation.completion);
        tokio::spawn(async move {
            let _completion = CompletionGuard(completion);
            run_chat_operation(
                repository,
                provider,
                events,
                clock,
                operation_id,
                accepted_chat.session_id,
                accepted_chat.message_id,
                operation,
            )
            .await;
        });
        Ok(accepted)
    }

    pub async fn cancel_chat(
        &self,
        client_request_id: Uuid,
        operation_id: Uuid,
    ) -> Result<CancelOperationResponse, ApiError> {
        let request = (client_request_id, operation_id, "chat");
        let request_hash = canonical_request_hash(&request)?;
        self.state
            .cancel_requests
            .execute(
                "api_v1_cancel_operation",
                client_request_id,
                request_hash,
                || self.cancel_chat_once(client_request_id, operation_id),
            )
            .await
    }

    pub(crate) async fn quiesce_for_reset(&self) -> Result<(), ApiError> {
        self.suspend_chat_operations().await
    }

    pub(crate) async fn prepare_suspend(&self) -> Result<(), ApiError> {
        self.suspend_chat_operations().await
    }

    pub(crate) fn begin_suspend(&self) {
        self.state.accepting.store(false, Ordering::Release);
    }

    pub(crate) fn resume_after_suspend(&self) {
        self.state.accepting.store(true, Ordering::Release);
    }

    async fn suspend_chat_operations(&self) -> Result<(), ApiError> {
        self.begin_suspend();
        let operations = {
            let _admission = self.state.admission.lock().await;
            self.state
                .operations
                .lock()
                .await
                .values()
                .cloned()
                .collect::<Vec<_>>()
        };
        for operation in operations {
            let mut state = operation.state.lock().await;
            if matches!(*state, ChatState::Active) {
                operation.cancellation.cancel();
                *state = ChatState::Terminal;
            }
            drop(state);
            operation.completion.wait().await;
        }
        Ok(())
    }

    async fn cancel_chat_once(
        &self,
        client_request_id: Uuid,
        operation_id: Uuid,
    ) -> Result<CancelOperationResponse, ApiError> {
        let operation = self
            .state
            .operations
            .lock()
            .await
            .get(&operation_id)
            .cloned()
            .ok_or_else(|| ApiError::from_reason(InternalReason::OperationNotFound))?;
        let mut state = operation.state.lock().await;
        let response_state = match *state {
            ChatState::Active => {
                operation.cancellation.cancel();
                *state = ChatState::Terminal;
                self.events
                    .cancelled(operation_id, &self.clock.now_rfc3339())?;
                CancelOperationState::Cancelled
            }
            ChatState::Terminal => CancelOperationState::AlreadyTerminal,
        };
        Ok(CancelOperationResponse {
            request_id: client_request_id,
            operation_id,
            state: response_state,
        })
    }

    pub async fn submit_feedback(&self, request: SubmitFeedbackRequest) -> Result<Ack, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .ack_requests
            .execute(
                "api_v1_submit_feedback",
                request_id,
                request_hash,
                || async {
                    if matches!(request.kind, FeedbackKind::LessTalk) != request.track_id.is_none()
                    {
                        return Err(ApiError::from_reason(InternalReason::RequestInvalid));
                    }
                    let revision = self
                        .repository
                        .record_feedback(
                            request.program_id,
                            request.track_id,
                            request.kind.as_str(),
                            self.clock.now_ms(),
                        )
                        .await
                        .map_err(map_storage_error)?;
                    Ok(Ack {
                        request_id,
                        revision,
                    })
                },
            )
            .await
    }

    pub async fn list_memories(
        &self,
        request: ListMemoriesRequest,
    ) -> Result<MemoryPage, ApiError> {
        let offset = decode_cursor(request.cursor.as_deref())?;
        let (items, next) = self
            .repository
            .list_memories(
                request.status.map(super::MemoryListStatus::as_str),
                offset,
                request.limit,
            )
            .await
            .map_err(map_storage_error)?;
        Ok(MemoryPage {
            items: items
                .into_iter()
                .map(memory_to_dto)
                .collect::<Result<_, _>>()?,
            next_cursor: next.map(encode_cursor),
        })
    }

    pub async fn approve_memory(
        &self,
        request: MemoryMutationRequest,
    ) -> Result<MemoryRecord, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .memory_requests
            .execute(
                "api_v1_approve_memory",
                request_id,
                request_hash,
                || async {
                    self.repository
                        .approve_memory(
                            request.memory_id,
                            request.expected_revision,
                            self.clock.now_ms(),
                        )
                        .await
                        .map_err(map_storage_error)
                        .and_then(memory_to_dto)
                },
            )
            .await
    }

    pub async fn update_memory(
        &self,
        request: UpdateMemoryRequest,
    ) -> Result<MemoryRecord, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .memory_requests
            .execute("api_v1_update_memory", request_id, request_hash, || async {
                validate_memory_content(&request.content)?;
                self.repository
                    .update_memory(
                        request.memory_id,
                        request.expected_revision,
                        &request.content,
                        request.enabled,
                        self.clock.now_ms(),
                    )
                    .await
                    .map_err(map_storage_error)
                    .and_then(memory_to_dto)
            })
            .await
    }

    pub async fn delete_memory(&self, request: MemoryMutationRequest) -> Result<Ack, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .ack_requests
            .execute("api_v1_delete_memory", request_id, request_hash, || async {
                let revision = self
                    .repository
                    .delete_memory(
                        request.memory_id,
                        request.expected_revision,
                        self.clock.now_ms(),
                    )
                    .await
                    .map_err(map_storage_error)?;
                Ok(Ack {
                    request_id,
                    revision,
                })
            })
            .await
    }

    pub async fn reject_memory(
        &self,
        request: MemoryMutationRequest,
    ) -> Result<RejectMemoryResponse, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .reject_requests
            .execute(
                "api_v1_reject_memory_proposal",
                request_id,
                request_hash,
                || async {
                    let rejected_at_ms = self.clock.now_ms();
                    let revision = self
                        .repository
                        .reject_memory_proposal(
                            request.memory_id,
                            request.expected_revision,
                            rejected_at_ms,
                        )
                        .await
                        .map_err(map_storage_error)?;
                    let content_delete_at_ms = rejected_at_ms
                        .checked_add(REJECTED_CONTENT_RETENTION_MS)
                        .ok_or_else(ApiError::unexpected)?;
                    Ok(RejectMemoryResponse {
                        request_id,
                        memory_id: request.memory_id,
                        status: RejectedStatus::Rejected,
                        rejected_at: timestamp(rejected_at_ms)?,
                        content_delete_at: timestamp(content_delete_at_ms)?,
                        revision,
                    })
                },
            )
            .await
    }

    pub async fn get_profile_view(&self) -> Result<ProfileViewResponse, ApiError> {
        let stored = self
            .repository
            .load_profile_view()
            .await
            .map_err(map_storage_error)?;
        let trends = self
            .repository
            .load_preference_trends(self.clock.now_ms())
            .await
            .map_err(map_storage_error)?;
        let weather_location = match (
            stored.city,
            stored.country,
            stored.country_code,
            stored.latitude,
            stored.longitude,
            stored.timezone,
        ) {
            (
                Some(city),
                Some(country),
                Some(country_code),
                Some(latitude),
                Some(longitude),
                Some(timezone),
            ) => Some(WeatherLocation {
                city,
                region: stored.region,
                country,
                country_code,
                latitude: latitude.parse().map_err(|_| ApiError::unexpected())?,
                longitude: longitude.parse().map_err(|_| ApiError::unexpected())?,
                timezone,
            }),
            (None, None, None, None, None, _) => None,
            _ => {
                return Err(ApiError::from_reason(
                    InternalReason::StorageIntegrityFailed,
                ));
            }
        };
        let companion_style = match stored.companion_style.as_str() {
            "quiet_warm" => CompanionStyle::QuietWarm,
            _ => return Err(ApiError::unexpected()),
        };
        let narration_density = narration_density(&stored.narration_density)?;
        Ok(ProfileViewResponse {
            profile: UserProfileView {
                display_name: stored.display_name,
                companion_style,
                initial_preferences: stored.initial_preferences,
                narration_density,
                weather_location,
            },
            preference_trends: trends
                .into_iter()
                .map(|trend| {
                    Ok(PreferenceTrend {
                        kind: trend.kind,
                        label: trend.label,
                        direction: match trend.direction.as_str() {
                            "up" => TrendDirection::Up,
                            "stable" => TrendDirection::Stable,
                            "down" => TrendDirection::Down,
                            _ => return Err(ApiError::unexpected()),
                        },
                        sample_count: trend.sample_count,
                        window_days: trend.window_days,
                    })
                })
                .collect::<Result<_, ApiError>>()?,
            revision: stored.revision,
        })
    }

    pub async fn update_profile(&self, request: UpdateProfileRequest) -> Result<Ack, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .ack_requests
            .execute(
                "api_v1_update_profile",
                request_id,
                request_hash,
                || async {
                    validate_profile_patch(&request)?;
                    let companion = request.patch.companion_style.map(|_| "quiet_warm");
                    let narration = request.patch.narration_density.map(narration_density_str);
                    let revision = self
                        .repository
                        .update_profile_fields(
                            request.expected_revision,
                            request.patch.display_name.as_deref(),
                            companion,
                            request.patch.initial_preferences.as_deref(),
                            narration,
                            self.clock.now_ms(),
                        )
                        .await
                        .map_err(map_storage_error)?;
                    Ok(Ack {
                        request_id,
                        revision,
                    })
                },
            )
            .await
    }

    pub async fn list_summaries(
        &self,
        request: PageRequest,
    ) -> Result<SessionSummaryPage, ApiError> {
        let offset = decode_cursor(request.cursor.as_deref())?;
        let (items, next) = self
            .repository
            .list_session_summaries(offset, request.limit)
            .await
            .map_err(map_storage_error)?;
        let items = items
            .into_iter()
            .map(|item| {
                Ok(SessionSummaryView {
                    summary_id: item.summary_id,
                    covered_from: timestamp(item.covered_from_ms)?,
                    covered_to: timestamp(item.covered_to_ms)?,
                    summary: item.summary,
                    generation_kind: match item.generation_kind.as_str() {
                        "llm" => SummaryGenerationKind::Llm,
                        "deterministic" => SummaryGenerationKind::Deterministic,
                        _ => return Err(ApiError::unexpected()),
                    },
                    revision: item.revision,
                })
            })
            .collect::<Result<_, ApiError>>()?;
        Ok(SessionSummaryPage {
            items,
            next_cursor: next.map(encode_cursor),
        })
    }

    pub async fn delete_summary(&self, request: DeleteSummaryRequest) -> Result<Ack, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.state
            .ack_requests
            .execute(
                "api_v1_delete_session_summary",
                request_id,
                request_hash,
                || async {
                    let revision = self
                        .repository
                        .delete_session_summary(
                            request.summary_id,
                            request.expected_revision,
                            self.clock.now_ms(),
                        )
                        .await
                        .map_err(map_storage_error)?;
                    Ok(Ack {
                        request_id,
                        revision,
                    })
                },
            )
            .await
    }
}

#[allow(clippy::too_many_arguments)] // Keeps the accepted user-message provenance explicit.
async fn run_chat_operation(
    repository: Repository,
    provider: Option<Arc<dyn ChatProvider>>,
    events: Arc<dyn ChatEventSink>,
    clock: Arc<dyn Clock>,
    operation_id: Uuid,
    session_id: Uuid,
    source_message_id: Uuid,
    operation: Arc<ChatOperation>,
) {
    let Ok(context) = repository.load_chat_context(session_id).await else {
        return;
    };
    let call = ProviderCallContext {
        correlation_id: Uuid::now_v7(),
        locale: "zh-CN",
        deadline: std::time::Instant::now() + CHAT_TIMEOUT,
        cancellation: operation.cancellation.clone(),
    };
    let output = match provider {
        Some(provider) => match provider.respond(&context, &call).await {
            Ok(output) => Some(output),
            Err(ChatProviderError::Cancelled) => None,
            Err(ChatProviderError::Unavailable | ChatProviderError::InvalidResponse) => {
                Some(super::ChatProviderOutput {
                    text: DEGRADED_CHAT_MESSAGE.to_owned(),
                    proposed_memories: Vec::new(),
                    provider: "local",
                    model: "deterministic".to_owned(),
                    prompt_version: "chat-degraded-v1",
                })
            }
        },
        None => Some(super::ChatProviderOutput {
            text: DEGRADED_CHAT_MESSAGE.to_owned(),
            proposed_memories: Vec::new(),
            provider: "local",
            model: "deterministic".to_owned(),
            prompt_version: "chat-degraded-v1",
        }),
    };
    let Some(output) = output else { return };
    let mut state = operation.state.lock().await;
    if !matches!(*state, ChatState::Active) || operation.cancellation.is_cancelled() {
        return;
    }
    let proposals = output
        .proposed_memories
        .iter()
        .map(|value| NewMemoryProposal {
            kind: value.kind,
            content: value.content.clone(),
            confidence: value.confidence,
        })
        .collect::<Vec<_>>();
    let now = clock.now();
    let used_memory_ids = if output.provider == "local" {
        Vec::new()
    } else {
        context
            .approved_memories
            .iter()
            .map(|memory| memory.memory_id)
            .collect::<Vec<_>>()
    };
    let persisted = repository
        .persist_chat_result(
            session_id,
            source_message_id,
            &used_memory_ids,
            &output.text,
            &proposals,
            Some(output.provider),
            Some(&output.model),
            output.prompt_version,
            now.timestamp_millis(),
        )
        .await;
    let Ok((_message_id, memories)) = persisted else {
        return;
    };
    *state = ChatState::Terminal;
    let occurred_at = format_time(now);
    let _ = events.assistant_message(
        operation_id,
        operation.program_id,
        &output.text,
        &occurred_at,
    );
    for memory in memories
        .into_iter()
        .filter_map(|value| memory_to_dto(value).ok())
    {
        let _ = events.memory_proposed(&memory, &occurred_at);
    }
}

fn memory_to_dto(value: StoredMemory) -> Result<MemoryRecord, ApiError> {
    Ok(MemoryRecord {
        schema_version: crate::ipc::IPC_SCHEMA_VERSION.to_owned(),
        memory_id: value.memory_id.to_string(),
        status: match value.status {
            StoredMemoryStatus::Proposed => MemoryRecordStatus::Proposed,
            StoredMemoryStatus::Approved => MemoryRecordStatus::Approved,
            StoredMemoryStatus::Disabled => MemoryRecordStatus::Disabled,
        },
        kind: match value.kind {
            StoredMemoryKind::Preference => MemoryRecordKind::Preference,
            StoredMemoryKind::Routine => MemoryRecordKind::Routine,
            StoredMemoryKind::Boundary => MemoryRecordKind::Boundary,
            StoredMemoryKind::Biographical => MemoryRecordKind::Biographical,
        },
        content: value.content,
        confidence: value.confidence,
        source_session_id: value.source_session_id.map(|id| id.to_string()),
        created_at: timestamp(value.created_at_ms)?,
        updated_at: timestamp(value.updated_at_ms)?,
        approved_at: value.approved_at_ms.map(timestamp).transpose()?,
        last_used_at: value.last_used_at_ms.map(timestamp).transpose()?,
        enabled: value.enabled,
        revision: value.revision,
    })
}

fn validate_chat_text(text: &str) -> Result<(), ApiError> {
    if text.trim().is_empty()
        || text.chars().count() > 4_000
        || text.chars().any(|value| value == '\0')
    {
        Err(ApiError::from_reason(InternalReason::RequestInvalid))
    } else {
        Ok(())
    }
}
fn requests_less_talk(text: &str) -> bool {
    matches!(
        text.trim().trim_matches(['。', '！', '!', '，', ',']),
        "少说一点" | "请少说一点" | "今天少说一点" | "安静一点" | "请安静一点" | "今天安静一点"
    )
}
fn validate_memory_content(text: &str) -> Result<(), ApiError> {
    if text.is_empty() || text.chars().count() > 500 || text.chars().any(char::is_control) {
        Err(ApiError::from_reason(InternalReason::RequestInvalid))
    } else {
        Ok(())
    }
}
fn validate_profile_patch(request: &UpdateProfileRequest) -> Result<(), ApiError> {
    let patch = &request.patch;
    if patch.is_empty()
        || patch
            .display_name
            .as_ref()
            .is_some_and(|value| value.chars().count() > 80)
        || patch.initial_preferences.as_ref().is_some_and(|items| {
            items.len() > 20
                || items.iter().any(|value| {
                    value.is_empty()
                        || value.chars().count() > 100
                        || value.chars().any(char::is_control)
                })
        })
    {
        Err(ApiError::from_reason(InternalReason::InvalidPatch))
    } else {
        Ok(())
    }
}
fn narration_density(value: &str) -> Result<NarrationDensity, ApiError> {
    match value {
        "quiet" => Ok(NarrationDensity::Quiet),
        "balanced" => Ok(NarrationDensity::Balanced),
        "frequent" => Ok(NarrationDensity::Frequent),
        _ => Err(ApiError::unexpected()),
    }
}
const fn narration_density_str(value: NarrationDensity) -> &'static str {
    match value {
        NarrationDensity::Quiet => "quiet",
        NarrationDensity::Balanced => "balanced",
        NarrationDensity::Frequent => "frequent",
    }
}
fn decode_cursor(value: Option<&str>) -> Result<u64, ApiError> {
    match value {
        None => Ok(0),
        Some(value) => value
            .strip_prefix("v1:")
            .and_then(|number| number.parse().ok())
            .ok_or_else(|| ApiError::from_reason(InternalReason::RequestInvalid)),
    }
}
fn encode_cursor(value: u64) -> String {
    format!("v1:{value}")
}
fn timestamp(value: i64) -> Result<String, ApiError> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .map(format_time)
        .ok_or_else(ApiError::unexpected)
}
fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[allow(clippy::needless_pass_by_value)] // Required by Result::map_err without exposing storage errors.
fn map_storage_error(error: StorageError) -> ApiError {
    ApiError::from_reason(match error.reason() {
        StorageReason::StorageReadFailed => InternalReason::StorageReadFailed,
        StorageReason::StorageWriteFailed | StorageReason::InvalidSetting => {
            InternalReason::StorageWriteFailed
        }
        StorageReason::StorageIntegrityFailed | StorageReason::ForeignDatabase => {
            InternalReason::StorageIntegrityFailed
        }
        StorageReason::EntityNotFound => InternalReason::EntityNotFound,
        StorageReason::RevisionConflict => InternalReason::RevisionConflict,
        StorageReason::ResourceBusy => InternalReason::ResourceBusy,
        StorageReason::MigrationFailed => InternalReason::MigrationFailed,
        StorageReason::DatabaseVersionUnsupported => InternalReason::DatabaseVersionUnsupported,
        StorageReason::PathDenied => InternalReason::PathDenied,
        StorageReason::PathOutsideScope => InternalReason::PathOutsideRoot,
        StorageReason::UnsafeReparsePoint => InternalReason::UnsafeReparsePoint,
    })
}
