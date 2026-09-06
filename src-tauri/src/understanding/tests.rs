use std::sync::{Arc, Mutex as StdMutex};

use chrono::{DateTime, TimeZone, Utc};
use tokio::sync::Notify;
use uuid::Uuid;

use super::*;
use crate::{
    contracts::MemoryRecord,
    ipc::{ApiError, ErrorId},
    providers::{Clock, ProviderCallContext},
    storage::{AppPaths, ChatContextSnapshot, Repository, Storage, StoredMemoryKind},
};

#[derive(Default)]
struct RecordedEvents {
    user: StdMutex<Vec<Uuid>>,
    assistant: StdMutex<Vec<String>>,
    proposals: StdMutex<Vec<MemoryRecord>>,
    cancelled: StdMutex<Vec<Uuid>>,
    terminal: Notify,
}

impl ChatEventSink for RecordedEvents {
    fn user_message(
        &self,
        operation_id: Uuid,
        _program_id: Uuid,
        _text: &str,
        _occurred_at: &str,
    ) -> Result<(), ApiError> {
        self.user.lock().expect("user events").push(operation_id);
        Ok(())
    }
    fn assistant_message(
        &self,
        _operation_id: Uuid,
        _program_id: Uuid,
        text: &str,
        _occurred_at: &str,
    ) -> Result<(), ApiError> {
        self.assistant
            .lock()
            .expect("assistant events")
            .push(text.to_owned());
        self.terminal.notify_one();
        Ok(())
    }
    fn memory_proposed(&self, memory: &MemoryRecord, _occurred_at: &str) -> Result<(), ApiError> {
        self.proposals
            .lock()
            .expect("proposal events")
            .push(memory.clone());
        Ok(())
    }
    fn cancelled(&self, operation_id: Uuid, _occurred_at: &str) -> Result<(), ApiError> {
        self.cancelled
            .lock()
            .expect("cancelled events")
            .push(operation_id);
        self.terminal.notify_one();
        Ok(())
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(10_000)
            .single()
            .expect("fixed time")
    }
}

struct UnavailableProvider;
impl ChatProvider for UnavailableProvider {
    fn respond<'a>(
        &'a self,
        _context: &'a ChatContextSnapshot,
        _call: &'a ProviderCallContext,
    ) -> ChatFuture<'a> {
        Box::pin(async { Err(ChatProviderError::Unavailable) })
    }
}

struct WaitingProvider {
    started: Arc<Notify>,
}
impl ChatProvider for WaitingProvider {
    fn respond<'a>(
        &'a self,
        _context: &'a ChatContextSnapshot,
        call: &'a ProviderCallContext,
    ) -> ChatFuture<'a> {
        Box::pin(async move {
            self.started.notify_one();
            while !call.cancellation.is_cancelled() {
                tokio::task::yield_now().await;
            }
            Err(ChatProviderError::Cancelled)
        })
    }
}

struct MemoryProvider;
impl ChatProvider for MemoryProvider {
    fn respond<'a>(
        &'a self,
        _context: &'a ChatContextSnapshot,
        _call: &'a ProviderCallContext,
    ) -> ChatFuture<'a> {
        Box::pin(async {
            Ok(ChatProviderOutput {
                text: "我会把它作为待审批建议。".to_owned(),
                proposed_memories: vec![ProposedMemory {
                    kind: StoredMemoryKind::Preference,
                    content: "喜欢安静的环境音乐".to_owned(),
                    confidence: 0.92,
                }],
                provider: "fixture",
                model: "fixture-model".to_owned(),
                prompt_version: "fixture-v1",
            })
        })
    }
}

async fn fixture() -> (tempfile::TempDir, Storage, Repository, Uuid) {
    let temp = tempfile::tempdir().expect("temp");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("paths");
    let storage = Storage::open(&paths, "0.4.0").await.expect("storage");
    let repository = storage.repository();
    let program_id = Uuid::now_v7();
    repository
        .begin_local_program(program_id, 1)
        .await
        .expect("program");
    (temp, storage, repository, program_id)
}

#[tokio::test]
async fn unavailable_openai_has_deterministic_local_only_fallback() {
    let (_temp, storage, repository, program_id) = fixture().await;
    let events = Arc::new(RecordedEvents::default());
    let service = UnderstandingService::new(
        repository.clone(),
        Some(Arc::new(UnavailableProvider)),
        events.clone(),
        Arc::new(FixedClock),
    );
    service
        .submit_chat(SubmitChatRequest {
            client_request_id: Uuid::now_v7(),
            program_id,
            text: "陪我聊聊".to_owned(),
        })
        .await
        .expect("accepted");
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.terminal.notified(),
    )
    .await
    .expect("fallback event");
    assert_eq!(
        events.assistant.lock().expect("assistant").as_slice(),
        [DEGRADED_CHAT_MESSAGE]
    );
    assert!(events.proposals.lock().expect("proposals").is_empty());
    assert_eq!(repository.message_count().await.expect("message count"), 2);
    storage.close().await;
}

#[tokio::test]
async fn explicit_less_talk_chat_updates_the_active_program_before_acknowledgement() {
    let (_temp, storage, repository, program_id) = fixture().await;
    let events = Arc::new(RecordedEvents::default());
    let service = UnderstandingService::new(
        repository.clone(),
        Some(Arc::new(UnavailableProvider)),
        events.clone(),
        Arc::new(FixedClock),
    );
    service
        .submit_chat(SubmitChatRequest {
            client_request_id: Uuid::now_v7(),
            program_id,
            text: "今天少说一点".to_owned(),
        })
        .await
        .expect("accepted");
    assert!(
        !repository
            .voice_allowed_after_feedback(program_id)
            .await
            .expect("less-talk policy")
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.terminal.notified(),
    )
    .await
    .expect("terminal event");
    storage.close().await;
}

#[tokio::test]
async fn cancellation_keeps_user_turn_and_suppresses_reply_and_proposal() {
    let (_temp, storage, repository, program_id) = fixture().await;
    let events = Arc::new(RecordedEvents::default());
    let started = Arc::new(Notify::new());
    let service = UnderstandingService::new(
        repository.clone(),
        Some(Arc::new(WaitingProvider {
            started: started.clone(),
        })),
        events.clone(),
        Arc::new(FixedClock),
    );
    let accepted = service
        .submit_chat(SubmitChatRequest {
            client_request_id: Uuid::now_v7(),
            program_id,
            text: "取消这次回答".to_owned(),
        })
        .await
        .expect("accepted");
    tokio::time::timeout(std::time::Duration::from_secs(1), started.notified())
        .await
        .expect("provider start");
    let cancelled = service
        .cancel_chat(Uuid::now_v7(), accepted.operation_id)
        .await
        .expect("cancel");
    assert_eq!(
        cancelled.state,
        crate::providers::CancelOperationState::Cancelled
    );
    tokio::task::yield_now().await;
    assert_eq!(repository.message_count().await.expect("message count"), 1);
    assert!(events.assistant.lock().expect("assistant").is_empty());
    assert!(events.proposals.lock().expect("proposals").is_empty());
    storage.close().await;
}

#[tokio::test]
async fn proposal_event_precedes_explicit_approval_and_next_context_use() {
    let (_temp, storage, repository, program_id) = fixture().await;
    let events = Arc::new(RecordedEvents::default());
    let service = UnderstandingService::new(
        repository.clone(),
        Some(Arc::new(MemoryProvider)),
        events.clone(),
        Arc::new(FixedClock),
    );
    service
        .submit_chat(SubmitChatRequest {
            client_request_id: Uuid::now_v7(),
            program_id,
            text: "我喜欢安静的环境音乐".to_owned(),
        })
        .await
        .expect("accepted");
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.terminal.notified(),
    )
    .await
    .expect("assistant event");
    for _ in 0..20 {
        if !events.proposals.lock().expect("proposals").is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let proposal = events.proposals.lock().expect("proposals")[0].clone();
    assert_eq!(
        proposal.status,
        crate::contracts::MemoryRecordStatus::Proposed
    );
    let approved = service
        .approve_memory(MemoryMutationRequest {
            client_request_id: Uuid::now_v7(),
            memory_id: Uuid::parse_str(&proposal.memory_id).expect("memory id"),
            expected_revision: proposal.revision,
        })
        .await
        .expect("approve");
    assert!(approved.enabled);
    let (_, approved_tags, _) = repository
        .load_program_planning_facts()
        .await
        .expect("planning context");
    assert_eq!(approved_tags, ["喜欢安静的环境音乐"]);
    service
        .submit_chat(SubmitChatRequest {
            client_request_id: Uuid::now_v7(),
            program_id,
            text: "再陪我聊一句".to_owned(),
        })
        .await
        .expect("second accepted");
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.terminal.notified(),
    )
    .await
    .expect("second assistant event");
    let approved_page = service
        .list_memories(ListMemoriesRequest {
            cursor: None,
            limit: 20,
            status: Some(MemoryListStatus::Approved),
        })
        .await
        .expect("approved list");
    assert!(approved_page.items[0].last_used_at.is_some());
    storage.close().await;
}

#[tokio::test]
async fn profile_edit_is_revisioned_and_changes_next_program_context() {
    let (_temp, storage, repository, _) = fixture().await;
    let events = Arc::new(RecordedEvents::default());
    let service = UnderstandingService::new(repository.clone(), None, events, Arc::new(FixedClock));
    let initial = service.get_profile_view().await.expect("initial profile");
    assert_eq!(initial.revision, 0);
    let updated = service
        .update_profile(UpdateProfileRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch: ProfilePatch {
                display_name: Some("小岚".to_owned()),
                companion_style: None,
                initial_preferences: Some(vec!["ambient".to_owned()]),
                narration_density: Some(NarrationDensity::Quiet),
            },
        })
        .await
        .expect("profile update");
    assert_eq!(updated.revision, 1);
    let view = service.get_profile_view().await.expect("updated profile");
    assert_eq!(view.profile.display_name, "小岚");
    assert_eq!(view.profile.initial_preferences, ["ambient"]);
    assert_eq!(view.profile.narration_density, NarrationDensity::Quiet);
    let (profile_tags, _, _) = repository
        .load_program_planning_facts()
        .await
        .expect("next context");
    assert_eq!(profile_tags, ["ambient"]);
    storage.close().await;
}

#[tokio::test]
async fn memory_rejection_disappears_from_lists_and_retains_only_bounded_tombstone() {
    let (_temp, storage, repository, program_id) = fixture().await;
    let events = Arc::new(RecordedEvents::default());
    let service = UnderstandingService::new(
        repository,
        Some(Arc::new(MemoryProvider)),
        events.clone(),
        Arc::new(FixedClock),
    );
    service
        .submit_chat(SubmitChatRequest {
            client_request_id: Uuid::now_v7(),
            program_id,
            text: "我喜欢安静的环境音乐".to_owned(),
        })
        .await
        .expect("accepted");
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.terminal.notified(),
    )
    .await
    .expect("assistant event");
    for _ in 0..20 {
        if !events.proposals.lock().expect("proposals").is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let proposal = events.proposals.lock().expect("proposals")[0].clone();
    let response = service
        .reject_memory(MemoryMutationRequest {
            client_request_id: Uuid::now_v7(),
            memory_id: Uuid::parse_str(&proposal.memory_id).expect("memory id"),
            expected_revision: 0,
        })
        .await
        .expect("reject");
    assert_eq!(response.status, RejectedStatus::Rejected);
    let page = service
        .list_memories(ListMemoriesRequest {
            cursor: None,
            limit: 20,
            status: None,
        })
        .await
        .expect("memory list");
    assert!(page.items.is_empty());
    storage.close().await;
}

#[tokio::test]
async fn mutation_keys_replay_original_result_and_reject_changed_payloads() {
    let (_temp, storage, repository, program_id) = fixture().await;
    let service = UnderstandingService::new(
        repository,
        None,
        Arc::new(RecordedEvents::default()),
        Arc::new(FixedClock),
    );
    service
        .update_profile(UpdateProfileRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch: ProfilePatch {
                display_name: Some("偏好测试".to_owned()),
                companion_style: None,
                initial_preferences: None,
                narration_density: None,
            },
        })
        .await
        .expect("profile baseline");
    let request = SubmitFeedbackRequest {
        client_request_id: Uuid::now_v7(),
        program_id,
        track_id: None,
        kind: FeedbackKind::LessTalk,
    };
    let first = service
        .submit_feedback(request.clone())
        .await
        .expect("first feedback");
    let replay = service
        .submit_feedback(request.clone())
        .await
        .expect("identical replay");
    assert_eq!(replay, first);
    let profile = service.get_profile_view().await.expect("profile trends");
    assert_eq!(profile.preference_trends.len(), 1);
    assert_eq!(profile.preference_trends[0].sample_count, 1);

    let conflict = service
        .submit_feedback(SubmitFeedbackRequest {
            track_id: Some(Uuid::now_v7()),
            kind: FeedbackKind::Like,
            ..request
        })
        .await
        .expect_err("changed payload must conflict");
    assert_eq!(conflict.error_id, ErrorId::Conflict);
    storage.close().await;
}
