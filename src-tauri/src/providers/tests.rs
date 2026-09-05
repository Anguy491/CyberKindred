use super::*;
use crate::{
    ipc::ErrorId,
    providers::{
        IntegrationState, ProviderFailureCategory, SecretKind, VoicePreviewCancelDisposition,
        VoiceProvider, VoiceView,
    },
    storage::{AppPaths, ProviderOutcomeStatus, ProviderUsageProvider, SecretValue, Storage},
};
use chrono::{DateTime, Utc};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

const FIXED_NOW: &str = "2026-09-02T03:04:05.000Z";

#[derive(Clone, Default)]
struct VaultState(Arc<StdMutex<HashMap<String, String>>>);

struct FakeVault {
    state: VaultState,
}

impl SecretVault for FakeVault {
    fn set(&mut self, target: &CredentialTarget, value: SecretValue) -> Result<(), SecretError> {
        let exposed = value.with_exposed(ToOwned::to_owned);
        self.state
            .0
            .lock()
            .map_err(|_| SecretError::OperationFailed)?
            .insert(target.as_resource().to_owned(), exposed);
        Ok(())
    }

    fn get(&self, target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
        self.state
            .0
            .lock()
            .map_err(|_| SecretError::OperationFailed)?
            .get(target.as_resource())
            .cloned()
            .map(SecretValue::new)
            .transpose()
    }

    fn delete(&mut self, target: &CredentialTarget) -> Result<(), SecretError> {
        self.state
            .0
            .lock()
            .map_err(|_| SecretError::OperationFailed)?
            .remove(target.as_resource());
        Ok(())
    }

    fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
        let mut values = self
            .state
            .0
            .lock()
            .map_err(|_| SecretError::OperationFailed)?;
        let count = u32::try_from(values.len()).map_err(|_| SecretError::OperationFailed)?;
        values.clear();
        Ok(count)
    }
}

struct PresenceOnlyVault {
    contains_calls: Arc<AtomicU64>,
    get_calls: Arc<AtomicU64>,
}

impl SecretVault for PresenceOnlyVault {
    fn set(&mut self, _target: &CredentialTarget, _value: SecretValue) -> Result<(), SecretError> {
        Err(SecretError::OperationFailed)
    }

    fn get(&self, _target: &CredentialTarget) -> Result<Option<SecretValue>, SecretError> {
        self.get_calls.fetch_add(1, Ordering::AcqRel);
        Err(SecretError::OperationFailed)
    }

    fn contains(&self, _target: &CredentialTarget) -> Result<bool, SecretError> {
        self.contains_calls.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }

    fn delete(&mut self, _target: &CredentialTarget) -> Result<(), SecretError> {
        Err(SecretError::OperationFailed)
    }

    fn delete_cyberkindred_namespace(&mut self) -> Result<u32, SecretError> {
        Err(SecretError::OperationFailed)
    }
}

#[derive(Clone)]
struct FakeValidator {
    calls: Arc<StdMutex<u64>>,
    result: Arc<StdMutex<Result<(), ProviderFailure>>>,
}

impl CandidateSecretValidator for FakeValidator {
    fn validate<'a>(
        &'a self,
        _input: SecretValidationInput<'a>,
        _context: &'a ProviderCallContext,
    ) -> super::super::traits::ProviderFuture<'a, Result<(), ProviderFailure>> {
        Box::pin(async move {
            *self
                .calls
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))? += 1;
            self.result
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                .clone()
        })
    }
}

struct BlockingValidator {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl CandidateSecretValidator for BlockingValidator {
    fn validate<'a>(
        &'a self,
        _input: SecretValidationInput<'a>,
        _context: &'a ProviderCallContext,
    ) -> super::super::traits::ProviderFuture<'a, Result<(), ProviderFailure>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        })
    }
}

#[derive(Clone)]
struct FakeProbe {
    calls: Arc<StdMutex<Vec<ProbeCall>>>,
    result: Arc<StdMutex<Result<u64, ProviderFailure>>>,
    expected_secret: Arc<StdMutex<Option<String>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProbeCall {
    kind: ProviderTestKind,
    origin: Option<String>,
    model_id: Option<String>,
    has_sixty_second_budget: bool,
}

impl ProviderHealthProbe for FakeProbe {
    fn test<'a>(
        &'a self,
        input: ProviderTestInput<'a>,
        context: &'a ProviderCallContext,
    ) -> super::super::traits::ProviderFuture<'a, Result<u64, ProviderFailure>> {
        Box::pin(async move {
            let remaining = context
                .deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or_default();
            let has_sixty_second_budget =
                remaining > Duration::from_secs(59) && remaining <= Duration::from_secs(60);
            let (call, secret_matches) = match input {
                ProviderTestInput::OpenAi {
                    kind,
                    origin,
                    secret,
                    model_id,
                } => {
                    let expected_secret = self
                        .expected_secret
                        .lock()
                        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                        .clone();
                    let secret_matches = expected_secret
                        .as_ref()
                        .is_none_or(|expected| secret.with_exposed(|actual| actual == expected));
                    (
                        ProbeCall {
                            kind,
                            origin: Some(origin.as_str().to_owned()),
                            model_id: Some(model_id.to_owned()),
                            has_sixty_second_budget,
                        },
                        secret_matches,
                    )
                }
                ProviderTestInput::Metadata => (
                    ProbeCall {
                        kind: ProviderTestKind::Metadata,
                        origin: None,
                        model_id: None,
                        has_sixty_second_budget,
                    },
                    true,
                ),
                ProviderTestInput::Weather => (
                    ProbeCall {
                        kind: ProviderTestKind::Weather,
                        origin: None,
                        model_id: None,
                        has_sixty_second_budget,
                    },
                    true,
                ),
            };
            self.calls
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                .push(call);
            if !secret_matches {
                return Err(ProviderFailure::new(
                    ProviderFailureCategory::InvalidResponse,
                ));
            }
            self.result
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                .clone()
        })
    }
}

#[derive(Clone)]
struct FakePreviewer {
    calls: Arc<StdMutex<Vec<(String, String)>>>,
    operation_ids: Arc<StdMutex<Vec<Uuid>>>,
    cancellations: Arc<StdMutex<Vec<Uuid>>>,
    result: Arc<StdMutex<Result<(), ProviderFailure>>>,
    delay: Arc<StdMutex<Duration>>,
}

impl VoicePreviewer for FakePreviewer {
    fn voices(&self) -> Vec<VoiceView> {
        vec![
            VoiceView {
                voice_id: "alloy".to_owned(),
                display_name: "Alloy".to_owned(),
                preview_available: true,
            },
            VoiceView {
                voice_id: "verse".to_owned(),
                display_name: "Verse".to_owned(),
                preview_available: true,
            },
        ]
    }

    fn preview<'a>(
        &'a self,
        input: VoicePreviewInput<'a>,
        _context: &'a ProviderCallContext,
    ) -> super::super::traits::ProviderFuture<'a, Result<(), ProviderFailure>> {
        Box::pin(async move {
            let delay = *self
                .delay
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
            self.calls
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                .push((input.voice_id.to_owned(), input.text.to_owned()));
            self.operation_ids
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                .push(input.operation_id);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            self.result
                .lock()
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?
                .clone()
        })
    }

    fn cancel(&self, operation_id: Uuid) -> VoicePreviewCancelDisposition {
        if let Ok(mut cancellations) = self.cancellations.lock() {
            if cancellations.contains(&operation_id) {
                return VoicePreviewCancelDisposition::AlreadyTerminal;
            }
            cancellations.push(operation_id);
            VoicePreviewCancelDisposition::Cancelled
        } else {
            VoicePreviewCancelDisposition::NotFound
        }
    }

    fn is_cancelled(&self, operation_id: Uuid) -> bool {
        self.cancellations
            .lock()
            .is_ok_and(|cancellations| cancellations.contains(&operation_id))
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(FIXED_NOW).map_or(DateTime::<Utc>::UNIX_EPOCH, |value| {
            value.with_timezone(&Utc)
        })
    }
}

#[derive(Clone, Default)]
struct RecordingPreviewEvents(
    Arc<StdMutex<Vec<VoicePreviewTerminal>>>,
    Arc<AtomicBool>,
    Arc<AtomicU64>,
);

impl VoicePreviewEventSink for RecordingPreviewEvents {
    fn publish(&self, terminal: VoicePreviewTerminal) -> Result<(), ApiError> {
        self.2.fetch_add(1, Ordering::AcqRel);
        if self.1.load(Ordering::Acquire) {
            return Err(ApiError::unexpected());
        }
        if let Ok(mut events) = self.0.lock() {
            events.push(terminal);
            Ok(())
        } else {
            Err(ApiError::unexpected())
        }
    }
}

#[derive(Clone, Default)]
struct RecordingSettingsEffect(Arc<StdMutex<Vec<(AppBehaviorSettings, AppBehaviorSettings)>>>);

impl AppSettingsEffect for RecordingSettingsEffect {
    fn apply(
        &self,
        previous: AppBehaviorSettings,
        next: AppBehaviorSettings,
    ) -> Result<(), ApiError> {
        self.0
            .lock()
            .map_err(|_| ApiError::unexpected())?
            .push((previous, next));
        Ok(())
    }
}

struct Fixture {
    temp: tempfile::TempDir,
    storage: Storage,
    service: ProviderService,
    vault: VaultState,
    validator_calls: Arc<StdMutex<u64>>,
    validator_result: Arc<StdMutex<Result<(), ProviderFailure>>>,
    probe_calls: Arc<StdMutex<Vec<ProbeCall>>>,
    probe_result: Arc<StdMutex<Result<u64, ProviderFailure>>>,
    probe_expected_secret: Arc<StdMutex<Option<String>>>,
    preview_calls: Arc<StdMutex<Vec<(String, String)>>>,
    preview_operation_ids: Arc<StdMutex<Vec<Uuid>>>,
    preview_cancellations: Arc<StdMutex<Vec<Uuid>>>,
    preview_result: Arc<StdMutex<Result<(), ProviderFailure>>>,
    preview_delay: Arc<StdMutex<Duration>>,
    preview_events: RecordingPreviewEvents,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped app paths");
        let storage = Storage::open(&paths, "provider-test")
            .await
            .expect("storage opens");
        let vault = VaultState::default();
        let validator_calls = Arc::new(StdMutex::new(0));
        let validator_result = Arc::new(StdMutex::new(Ok(())));
        let probe_calls = Arc::new(StdMutex::new(Vec::new()));
        let probe_result = Arc::new(StdMutex::new(Ok(7)));
        let probe_expected_secret = Arc::new(StdMutex::new(None));
        let preview_calls = Arc::new(StdMutex::new(Vec::new()));
        let preview_operation_ids = Arc::new(StdMutex::new(Vec::new()));
        let preview_cancellations = Arc::new(StdMutex::new(Vec::new()));
        let preview_result = Arc::new(StdMutex::new(Ok(())));
        let preview_delay = Arc::new(StdMutex::new(Duration::ZERO));
        let preview_events = RecordingPreviewEvents::default();
        let service = ProviderService::new(
            storage.repository(),
            Box::new(FakeVault {
                state: vault.clone(),
            }),
            Arc::new(FakeValidator {
                calls: validator_calls.clone(),
                result: validator_result.clone(),
            }),
            Arc::new(FakeProbe {
                calls: probe_calls.clone(),
                result: probe_result.clone(),
                expected_secret: probe_expected_secret.clone(),
            }),
            Arc::new(FakePreviewer {
                calls: preview_calls.clone(),
                operation_ids: preview_operation_ids.clone(),
                cancellations: preview_cancellations.clone(),
                result: preview_result.clone(),
                delay: preview_delay.clone(),
            }),
            Arc::new(preview_events.clone()),
            Arc::new(FixedClock),
        )
        .with_test_preview_timeouts(Duration::from_millis(500), Duration::from_millis(50));
        Self {
            temp,
            storage,
            service,
            vault,
            validator_calls,
            validator_result,
            probe_calls,
            probe_result,
            probe_expected_secret,
            preview_calls,
            preview_operation_ids,
            preview_cancellations,
            preview_result,
            preview_delay,
            preview_events,
        }
    }

    async fn configure(&self, origin: &str, value: &str) -> ValidateSecretResponse {
        self.service
            .validate_and_set_secret(ValidateSecretRequest {
                client_request_id: Uuid::now_v7(),
                kind: SecretKind::OpenaiApiKey,
                origin: origin.to_owned(),
                value: SecretValue::new(value.to_owned()).expect("valid secret"),
            })
            .await
            .expect("credential validates")
    }

    fn restarted_service(&self) -> ProviderService {
        ProviderService::new(
            self.storage.repository(),
            Box::new(FakeVault {
                state: self.vault.clone(),
            }),
            Arc::new(FakeValidator {
                calls: self.validator_calls.clone(),
                result: self.validator_result.clone(),
            }),
            Arc::new(FakeProbe {
                calls: self.probe_calls.clone(),
                result: self.probe_result.clone(),
                expected_secret: self.probe_expected_secret.clone(),
            }),
            Arc::new(FakePreviewer {
                calls: self.preview_calls.clone(),
                operation_ids: self.preview_operation_ids.clone(),
                cancellations: self.preview_cancellations.clone(),
                result: self.preview_result.clone(),
                delay: self.preview_delay.clone(),
            }),
            Arc::new(self.preview_events.clone()),
            Arc::new(FixedClock),
        )
        .with_test_preview_timeouts(Duration::from_millis(500), Duration::from_millis(50))
    }
}

fn count<T>(value: &Arc<StdMutex<T>>, read: impl FnOnce(&T) -> usize) -> usize {
    value.lock().map_or(usize::MAX, |guard| read(&guard))
}

#[tokio::test]
async fn provider_settings_checks_presence_without_materializing_secret() {
    let temp = tempfile::tempdir().expect("temp root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("scoped app paths");
    let storage = Storage::open(&paths, "provider-presence-test")
        .await
        .expect("storage opens");
    let contains_calls = Arc::new(AtomicU64::new(0));
    let get_calls = Arc::new(AtomicU64::new(0));
    let service = ProviderService::new(
        storage.repository(),
        Box::new(PresenceOnlyVault {
            contains_calls: Arc::clone(&contains_calls),
            get_calls: Arc::clone(&get_calls),
        }),
        Arc::new(FakeValidator {
            calls: Arc::new(StdMutex::new(0)),
            result: Arc::new(StdMutex::new(Ok(()))),
        }),
        Arc::new(FakeProbe {
            calls: Arc::new(StdMutex::new(Vec::new())),
            result: Arc::new(StdMutex::new(Ok(0))),
            expected_secret: Arc::new(StdMutex::new(None)),
        }),
        Arc::new(FakePreviewer {
            calls: Arc::new(StdMutex::new(Vec::new())),
            operation_ids: Arc::new(StdMutex::new(Vec::new())),
            cancellations: Arc::new(StdMutex::new(Vec::new())),
            result: Arc::new(StdMutex::new(Ok(()))),
            delay: Arc::new(StdMutex::new(Duration::ZERO)),
        }),
        Arc::new(RecordingPreviewEvents::default()),
        Arc::new(FixedClock),
    );

    let settings = service.get_settings().await.expect("settings view");
    assert!(settings.secret_status.origins[0].openai_api_key_configured);
    assert_eq!(contains_calls.load(Ordering::Acquire), 1);
    assert_eq!(get_calls.load(Ordering::Acquire), 0);
    storage.close().await;
}

async fn wait_for_preview_calls(calls: &Arc<StdMutex<Vec<(String, String)>>>, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while count(calls, Vec::len) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("preview task observed");
}

async fn wait_for_preview_events(events: &RecordingPreviewEvents, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while count(&events.0, Vec::len) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("preview terminal event observed");
}

async fn wait_for_preview_publish_attempts(events: &RecordingPreviewEvents, expected: u64) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while events.2.load(Ordering::Acquire) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("preview publish attempt observed");
}

#[tokio::test]
async fn provider_unconfirmed_paths_make_zero_paid_or_audio_calls() {
    let fixture = Fixture::new().await;
    let settings = fixture.service.get_settings().await.expect("settings");
    let voices = fixture
        .service
        .list_voices(ListVoicesRequest {
            provider: VoiceProvider::Tts,
        })
        .expect("static voices");

    assert!(!settings.tts_enabled);
    assert!(!settings.metadata_enabled);
    assert!(!settings.weather_enabled);
    assert!(!settings.secret_status.origins[0].openai_api_key_configured);
    assert_eq!(voices.voices.len(), 2);
    assert_eq!(*fixture.validator_calls.lock().expect("calls"), 0);
    assert_eq!(count(&fixture.probe_calls, Vec::len), 0);
    assert_eq!(count(&fixture.preview_calls, Vec::len), 0);
    assert_eq!(count(&fixture.vault.0, HashMap::len), 0);
}

#[tokio::test]
async fn provider_llm_test_passes_the_current_model_to_the_probe() {
    let fixture = Fixture::new().await;
    fixture
        .configure(
            "https://api.openai.com",
            "task-008-model-propagation-canary",
        )
        .await;
    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "llmModelId": "gpt-task-008-probe"
    }))
    .expect("model patch");
    fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 1,
            patch,
        })
        .await
        .expect("model setting updates");
    fixture.probe_calls.lock().expect("probe calls").clear();

    fixture
        .service
        .test_provider(TestProviderRequest {
            client_request_id: Uuid::now_v7(),
            kind: ProviderTestKind::Llm,
        })
        .await
        .expect("LLM probe succeeds");

    assert_eq!(
        fixture.probe_calls.lock().expect("probe calls").as_slice(),
        [ProbeCall {
            kind: ProviderTestKind::Llm,
            origin: Some("https://api.openai.com".to_owned()),
            model_id: Some("gpt-task-008-probe".to_owned()),
            has_sixty_second_budget: true,
        }]
    );
}

#[tokio::test]
async fn provider_idempotency_capacity_preserves_live_keys_and_rejects_new_work() {
    let store = AsyncIdempotency::<Uuid>::new();
    let first_id = Uuid::now_v7();
    let first_hash = canonical_request_hash(&first_id).expect("first fingerprint");

    for index in 0..IDEMPOTENCY_CAPACITY {
        let request_id = if index == 0 { first_id } else { Uuid::now_v7() };
        let hash = canonical_request_hash(&request_id).expect("request fingerprint");
        let result = store
            .execute(request_id, hash, || async move { Ok(request_id) })
            .await
            .expect("capacity slot");
        assert_eq!(result, request_id);
    }

    let overflow_called = Arc::new(StdMutex::new(false));
    let overflow_called_for_operation = Arc::clone(&overflow_called);
    let overflow = store
        .execute(
            Uuid::now_v7(),
            canonical_request_hash(&"overflow").expect("overflow fingerprint"),
            || async move {
                *overflow_called_for_operation.lock().expect("overflow flag") = true;
                Ok(Uuid::now_v7())
            },
        )
        .await
        .expect_err("live idempotency keys must not be evicted early");
    assert_eq!(overflow.error_id, ErrorId::ResourceBusy);
    assert!(!*overflow_called.lock().expect("overflow flag"));

    let replay = store
        .execute(first_id, first_hash, || async { Ok(Uuid::nil()) })
        .await
        .expect("original live key remains cached");
    assert_eq!(replay, first_id);
}

#[tokio::test]
async fn provider_idempotency_different_keys_do_not_queue_behind_provider_work() {
    let store = Arc::new(AsyncIdempotency::<Uuid>::new());
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let request_id = Uuid::now_v7();
        let request_hash = canonical_request_hash(&request_id).expect("request fingerprint");
        tasks.push(tokio::spawn(async move {
            store
                .execute(request_id, request_hash, || async move {
                    barrier.wait().await;
                    Ok(request_id)
                })
                .await
        }));
    }

    tokio::time::timeout(Duration::from_secs(1), barrier.wait())
        .await
        .expect("independent request IDs must both enter provider work");
    for task in tasks {
        assert!(task.await.expect("idempotency task").is_ok());
    }
}

#[test]
fn provider_storage_errors_preserve_the_contract_reason() {
    for (storage_reason, expected_reason) in [
        (StorageReason::PathDenied, "path_denied"),
        (StorageReason::PathOutsideScope, "path_outside_root"),
        (StorageReason::UnsafeReparsePoint, "unsafe_reparse_point"),
    ] {
        let error = map_storage_error(&StorageError::new(storage_reason));
        assert_eq!(
            error.details.and_then(|details| details.reason),
            Some(expected_reason.to_owned())
        );
    }
}

#[tokio::test]
async fn provider_secret_switch_retains_old_origin_until_exact_delete() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-old-canary")
        .await;
    fixture
        .configure("https://gateway.example.com", "task-008-new-canary")
        .await;

    let old_target = CredentialTarget::openai(
        &CanonicalOrigin::parse("https://api.openai.com").expect("origin"),
    );
    let new_target = CredentialTarget::openai(
        &CanonicalOrigin::parse("https://gateway.example.com").expect("origin"),
    );
    {
        let vault = fixture.vault.0.lock().expect("vault state");
        assert_eq!(
            vault.get(old_target.as_resource()).map(String::as_str),
            Some("task-008-old-canary")
        );
        assert_eq!(
            vault.get(new_target.as_resource()).map(String::as_str),
            Some("task-008-new-canary")
        );
    }
    let settings = fixture.service.get_settings().await.expect("settings");
    assert_eq!(settings.provider_origin, "https://gateway.example.com");
    assert!(settings.secret_status.origins[0].openai_api_key_configured);

    fixture
        .service
        .delete_secret(DeleteSecretRequest {
            client_request_id: Uuid::now_v7(),
            kind: SecretKind::OpenaiApiKey,
            origin: "https://api.openai.com".to_owned(),
        })
        .await
        .expect("exact old origin deletes");
    let vault = fixture.vault.0.lock().expect("vault state");
    assert!(!vault.contains_key(old_target.as_resource()));
    assert_eq!(
        vault.get(new_target.as_resource()).map(String::as_str),
        Some("task-008-new-canary")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_secret_delete_cannot_interleave_with_a_validated_state_commit() {
    let temp = tempfile::tempdir().expect("temp root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("scoped app paths");
    let storage = Storage::open(&paths, "provider-mutation-gate-test")
        .await
        .expect("storage opens");
    let vault = VaultState::default();
    let old_origin =
        CanonicalOrigin::parse("https://api.openai.com").expect("default canonical origin");
    FakeVault {
        state: vault.clone(),
    }
    .set(
        &CredentialTarget::openai(&old_origin),
        SecretValue::new("task-008-old-mutation-canary".to_owned()).expect("valid secret"),
    )
    .expect("seed old secret");
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let service = Arc::new(ProviderService::new(
        storage.repository(),
        Box::new(FakeVault {
            state: vault.clone(),
        }),
        Arc::new(BlockingValidator {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        }),
        Arc::new(FakeProbe {
            calls: Arc::new(StdMutex::new(Vec::new())),
            result: Arc::new(StdMutex::new(Ok(7))),
            expected_secret: Arc::new(StdMutex::new(None)),
        }),
        Arc::new(FakePreviewer {
            calls: Arc::new(StdMutex::new(Vec::new())),
            operation_ids: Arc::new(StdMutex::new(Vec::new())),
            cancellations: Arc::new(StdMutex::new(Vec::new())),
            result: Arc::new(StdMutex::new(Ok(()))),
            delay: Arc::new(StdMutex::new(Duration::ZERO)),
        }),
        Arc::new(RecordingPreviewEvents::default()),
        Arc::new(FixedClock),
    ));

    let configure_service = Arc::clone(&service);
    let configure = tokio::spawn(async move {
        configure_service
            .validate_and_set_secret(ValidateSecretRequest {
                client_request_id: Uuid::now_v7(),
                kind: SecretKind::OpenaiApiKey,
                origin: "https://gateway.example.com".to_owned(),
                value: SecretValue::new("task-008-new-mutation-canary".to_owned())
                    .expect("valid secret"),
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .expect("validation entered");

    let delete_service = Arc::clone(&service);
    let delete = tokio::spawn(async move {
        delete_service
            .delete_secret(DeleteSecretRequest {
                client_request_id: Uuid::now_v7(),
                kind: SecretKind::OpenaiApiKey,
                origin: "https://api.openai.com".to_owned(),
            })
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !delete.is_finished(),
        "delete must wait for the state commit"
    );

    release.notify_one();
    assert!(configure.await.expect("configure task").is_ok());
    assert!(delete.await.expect("delete task").is_ok());
    storage.close().await;
}

#[tokio::test]
async fn provider_secret_failure_preserves_previous_key_settings_and_safe_error() {
    for (failure, expected) in [
        (
            ProviderFailure::new(ProviderFailureCategory::Authentication),
            ErrorId::SecretRejected,
        ),
        (
            ProviderFailure::rate_limited(Some(1234)),
            ErrorId::ProviderRateLimited,
        ),
        (
            ProviderFailure::new(ProviderFailureCategory::Unavailable),
            ErrorId::ProviderUnavailable,
        ),
    ] {
        let fixture = Fixture::new().await;
        fixture
            .configure("https://api.openai.com", "task-008-preserved-canary")
            .await;
        *fixture.validator_result.lock().expect("validator result") = Err(failure);
        let error = fixture
            .service
            .validate_and_set_secret(ValidateSecretRequest {
                client_request_id: Uuid::now_v7(),
                kind: SecretKind::OpenaiApiKey,
                origin: "https://gateway.example.com".to_owned(),
                value: SecretValue::new("task-008-rejected-canary".to_owned()).expect("candidate"),
            })
            .await
            .expect_err("validation rejected");
        assert_eq!(error.error_id, expected);
        assert!(
            !serde_json::to_string(&error)
                .expect("safe error")
                .contains("canary")
        );
        let settings = fixture.service.get_settings().await.expect("settings");
        assert_eq!(settings.provider_origin, "https://api.openai.com");
        let restarted = fixture
            .restarted_service()
            .get_settings()
            .await
            .expect("restart hydration after rejected candidate credential");
        let openai = restarted
            .integration_statuses
            .iter()
            .find(|status| status.integration == Integration::Openai)
            .expect("OpenAI integration status");
        assert_eq!(openai.state, IntegrationState::Connected);
        let old_target = CredentialTarget::openai(
            &CanonicalOrigin::parse("https://api.openai.com").expect("origin"),
        );
        let vault = fixture.vault.0.lock().expect("vault state");
        assert_eq!(
            vault.get(old_target.as_resource()).map(String::as_str),
            Some("task-008-preserved-canary")
        );
        assert!(
            !vault
                .values()
                .any(|value| value == "task-008-rejected-canary")
        );
    }
}

#[tokio::test]
async fn provider_delete_reads_settings_before_irreversibly_removing_key() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-delete-preserved-canary")
        .await;
    let target = CredentialTarget::openai(
        &CanonicalOrigin::parse("https://api.openai.com").expect("origin"),
    );
    let Fixture {
        storage,
        service,
        vault,
        ..
    } = fixture;
    storage.close().await;

    let error = service
        .delete_secret(DeleteSecretRequest {
            client_request_id: Uuid::now_v7(),
            kind: SecretKind::OpenaiApiKey,
            origin: "https://api.openai.com".to_owned(),
        })
        .await
        .expect_err("closed settings store must fail before deletion");
    assert_eq!(error.error_id, ErrorId::StorageFailed);
    assert_eq!(
        vault
            .0
            .lock()
            .expect("vault state")
            .get(target.as_resource())
            .map(String::as_str),
        Some("task-008-delete-preserved-canary")
    );
}

#[tokio::test]
async fn provider_same_origin_persistence_failure_restores_previous_key() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-old-persisted-canary")
        .await;
    fixture.service.fail_next_settings_write_after_secret();
    let error = fixture
        .service
        .validate_and_set_secret(ValidateSecretRequest {
            client_request_id: Uuid::now_v7(),
            kind: SecretKind::OpenaiApiKey,
            origin: "https://api.openai.com".to_owned(),
            value: SecretValue::new("task-008-new-unpersisted-canary".to_owned())
                .expect("candidate"),
        })
        .await
        .expect_err("simulated settings persistence failure");
    assert_eq!(error.error_id, ErrorId::StorageFailed);

    let target = CredentialTarget::openai(
        &CanonicalOrigin::parse("https://api.openai.com").expect("origin"),
    );
    {
        let vault = fixture.vault.0.lock().expect("vault state");
        assert_eq!(
            vault.get(target.as_resource()).map(String::as_str),
            Some("task-008-old-persisted-canary")
        );
        assert!(
            !vault
                .values()
                .any(|value| value == "task-008-new-unpersisted-canary")
        );
    }
    let settings = fixture.service.get_settings().await.expect("settings");
    assert_eq!(settings.provider_origin, "https://api.openai.com");
    assert_eq!(settings.revision, 1);
    let restarted = fixture
        .restarted_service()
        .get_settings()
        .await
        .expect("restart hydration after unapplied credential success");
    let openai = restarted
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Openai)
        .expect("OpenAI integration status");
    assert_eq!(openai.state, IntegrationState::Connected);
}

#[tokio::test]
async fn provider_cross_origin_persistence_failure_restores_existing_candidate_key() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-current-origin-canary")
        .await;
    let candidate_target = CredentialTarget::openai(
        &CanonicalOrigin::parse("https://gateway.example.com").expect("origin"),
    );
    fixture.vault.0.lock().expect("vault state").insert(
        candidate_target.as_resource().to_owned(),
        "task-008-existing-candidate-canary".to_owned(),
    );

    fixture.service.fail_next_settings_write_after_secret();
    let error = fixture
        .service
        .validate_and_set_secret(ValidateSecretRequest {
            client_request_id: Uuid::now_v7(),
            kind: SecretKind::OpenaiApiKey,
            origin: "https://gateway.example.com".to_owned(),
            value: SecretValue::new("task-008-unpersisted-candidate-canary".to_owned())
                .expect("candidate"),
        })
        .await
        .expect_err("simulated settings persistence failure");
    assert_eq!(error.error_id, ErrorId::StorageFailed);

    {
        let vault = fixture.vault.0.lock().expect("vault state");
        assert_eq!(
            vault
                .get(candidate_target.as_resource())
                .map(String::as_str),
            Some("task-008-existing-candidate-canary")
        );
        assert!(
            !vault
                .values()
                .any(|value| value == "task-008-unpersisted-candidate-canary")
        );
    }
    let settings = fixture.service.get_settings().await.expect("settings");
    assert_eq!(settings.provider_origin, "https://api.openai.com");
}

#[tokio::test]
async fn provider_secret_canary_never_reaches_sqlite() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-sqlite-canary")
        .await;
    fixture
        .storage
        .truncate_checkpoint()
        .await
        .expect("checkpoint");

    let temp_root = fixture.temp.path();
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(temp_root).expect("read temp root") {
        let entry = entry.expect("entry");
        if entry.file_type().expect("type").is_dir() {
            for child in std::fs::read_dir(entry.path()).expect("read child") {
                let child = child.expect("child");
                if child.file_type().expect("type").is_file()
                    && std::fs::read(child.path())
                        .expect("read storage artifact")
                        .windows(b"task-008-sqlite-canary".len())
                        .any(|window| window == b"task-008-sqlite-canary")
                {
                    hits.push(child.file_name());
                }
            }
        }
    }
    assert!(hits.is_empty(), "secret canary found in SQLite artifacts");
}

#[tokio::test]
async fn provider_settings_patch_is_strict_nullable_and_revision_guarded() {
    assert!(serde_json::from_value::<SettingsPatch>(serde_json::json!({"mystery": true})).is_err());
    assert!(
        serde_json::from_value::<SettingsPatch>(serde_json::json!({"llmModelId": null})).is_err()
    );
    let nullable: SettingsPatch = serde_json::from_value(serde_json::json!({
        "defaultSourceId": null,
        "audioOutputDeviceId": null
    }))
    .expect("nullable fields");
    assert_eq!(
        nullable.default_source_id,
        super::super::dto::PatchField::Value(None)
    );

    let fixture = Fixture::new().await;
    let empty_error = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch: SettingsPatch::default(),
        })
        .await
        .expect_err("empty patch");
    assert_eq!(empty_error.error_id, ErrorId::RequestInvalid);

    let unknown_source: SettingsPatch = serde_json::from_value(serde_json::json!({
        "defaultSourceId": "untrusted-player"
    }))
    .expect("shape-valid source patch");
    let source_error = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch: unknown_source,
        })
        .await
        .expect_err("only v1 source adapters may be persisted");
    assert_eq!(source_error.error_id, ErrorId::RequestInvalid);

    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "metadataEnabled": true,
        "narrationDensity": "frequent"
    }))
    .expect("valid patch");
    let ack = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch,
        })
        .await
        .expect("update");
    assert_eq!(ack.revision, 1);
    let stale_patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "weatherEnabled": true
    }))
    .expect("valid patch");
    let stale = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch: stale_patch,
        })
        .await
        .expect_err("stale revision");
    assert_eq!(stale.error_id, ErrorId::Conflict);
    assert_eq!(
        stale.details.and_then(|details| details.current_revision),
        Some(1)
    );
}

#[tokio::test]
async fn provider_app_behavior_effect_commits_the_exact_transition() {
    let fixture = Fixture::new().await;
    let effect = RecordingSettingsEffect::default();
    let service = fixture
        .restarted_service()
        .with_settings_effect(Arc::new(effect.clone()));
    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "minimizeToTray": true,
        "launchAtStartup": true
    }))
    .expect("app behavior patch");

    let ack = service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch,
        })
        .await
        .expect("effect and settings commit");

    assert_eq!(ack.revision, 1);
    assert_eq!(
        effect.0.lock().expect("effect calls").as_slice(),
        [(
            AppBehaviorSettings {
                minimize_to_tray: false,
                launch_at_startup: false,
            },
            AppBehaviorSettings {
                minimize_to_tray: true,
                launch_at_startup: true,
            },
        )]
    );
    let settings = service.get_settings().await.expect("committed settings");
    assert!(settings.minimize_to_tray);
    assert!(settings.launch_at_startup);
}

#[tokio::test]
async fn provider_app_behavior_effect_rolls_back_when_storage_commit_fails() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-028-rollback-canary")
        .await;
    let effect = RecordingSettingsEffect::default();
    let service = fixture
        .restarted_service()
        .with_settings_effect(Arc::new(effect.clone()));
    service.fail_next_settings_write_after_model_probe();
    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "llmModelId": "gpt-task-028-uncommitted",
        "minimizeToTray": true
    }))
    .expect("combined patch");

    let error = service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 1,
            patch,
        })
        .await
        .expect_err("simulated storage failure");

    assert_eq!(error.error_id, ErrorId::StorageFailed);
    let disabled = AppBehaviorSettings {
        minimize_to_tray: false,
        launch_at_startup: false,
    };
    let tray_enabled = AppBehaviorSettings {
        minimize_to_tray: true,
        launch_at_startup: false,
    };
    assert_eq!(
        effect.0.lock().expect("effect calls").as_slice(),
        [(disabled, tray_enabled), (tray_enabled, disabled)]
    );
    let settings = service.get_settings().await.expect("unchanged settings");
    assert!(!settings.minimize_to_tray);
    assert_eq!(settings.llm_model_id, "gpt-5.6-luna");
    assert_eq!(settings.revision, 1);
}

#[tokio::test]
async fn provider_model_change_probes_merged_origin_model_and_credential_before_atomic_save() {
    let fixture = Fixture::new().await;
    let candidate_target = CredentialTarget::openai(
        &CanonicalOrigin::parse("https://gateway.example.com").expect("candidate origin"),
    );
    fixture.vault.0.lock().expect("vault state").insert(
        candidate_target.as_resource().to_owned(),
        "task-008-candidate-origin-key".to_owned(),
    );
    *fixture
        .probe_expected_secret
        .lock()
        .expect("expected secret") = Some("task-008-candidate-origin-key".to_owned());

    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "providerOrigin": "https://gateway.example.com",
        "llmModelId": "gpt-task-008-candidate",
        "metadataEnabled": true
    }))
    .expect("candidate patch");
    let ack = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch,
        })
        .await
        .expect("capability probe and atomic save succeed");
    assert_eq!(ack.revision, 1);
    assert_eq!(
        fixture.probe_calls.lock().expect("probe calls").as_slice(),
        [ProbeCall {
            kind: ProviderTestKind::Llm,
            origin: Some("https://gateway.example.com".to_owned()),
            model_id: Some("gpt-task-008-candidate".to_owned()),
            has_sixty_second_budget: true,
        }]
    );
    let statuses = fixture
        .storage
        .repository()
        .load_provider_statuses()
        .await
        .expect("successful outcome remains readable");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].provider, ProviderUsageProvider::OpenAi);
    assert_eq!(statuses[0].latest_status, ProviderOutcomeStatus::Success);

    let settings = fixture.service.get_settings().await.expect("settings");
    assert_eq!(settings.provider_origin, "https://gateway.example.com");
    assert_eq!(settings.llm_model_id, "gpt-task-008-candidate");
    assert!(settings.metadata_enabled);
    assert_eq!(settings.revision, 1);
}

#[tokio::test]
async fn provider_model_probe_failures_preserve_every_setting_and_revision() {
    for (failure, expected_error) in [
        (
            ProviderFailure::new(ProviderFailureCategory::Authentication),
            ErrorId::ProviderAuthentication,
        ),
        (
            ProviderFailure::rate_limited(Some(1_000)),
            ErrorId::ProviderRateLimited,
        ),
        (
            ProviderFailure::new(ProviderFailureCategory::Timeout),
            ErrorId::ProviderTimedOut,
        ),
        (
            ProviderFailure::new(ProviderFailureCategory::Unavailable),
            ErrorId::ProviderUnavailable,
        ),
        (
            ProviderFailure::new(ProviderFailureCategory::InvalidResponse),
            ErrorId::ProviderInvalidResponse,
        ),
    ] {
        let fixture = Fixture::new().await;
        fixture
            .configure("https://api.openai.com", "task-008-atomicity-key")
            .await;
        let before = fixture
            .service
            .get_settings()
            .await
            .expect("before settings");
        *fixture.probe_result.lock().expect("probe result") = Err(failure);
        let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
            "llmModelId": "gpt-task-008-rejected",
            "metadataEnabled": true,
            "narrationDensity": "frequent"
        }))
        .expect("candidate patch");

        let error = fixture
            .service
            .update_settings(UpdateSettingsRequest {
                client_request_id: Uuid::now_v7(),
                expected_revision: 1,
                patch,
            })
            .await
            .expect_err("failed probe rejects the entire patch");
        assert_eq!(error.error_id, expected_error);
        assert_eq!(count(&fixture.probe_calls, Vec::len), 1);
        let statuses = fixture
            .storage
            .repository()
            .load_provider_statuses()
            .await
            .expect("provider outcome remains readable");
        let openai = statuses
            .iter()
            .find(|status| status.provider == ProviderUsageProvider::OpenAi)
            .expect("committed OpenAI outcome");
        assert_eq!(openai.latest_status, ProviderOutcomeStatus::Success);
        let after = fixture
            .service
            .get_settings()
            .await
            .expect("after settings");
        assert_eq!(after, before);
        let restarted = fixture
            .restarted_service()
            .get_settings()
            .await
            .expect("restart hydration");
        let openai = restarted
            .integration_statuses
            .iter()
            .find(|status| status.integration == Integration::Openai)
            .expect("OpenAI integration status");
        assert_eq!(openai.state, IntegrationState::Connected);
    }
}

#[tokio::test]
async fn provider_model_probe_success_settings_failure_does_not_pollute_restart_status() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-model-save-fail-key")
        .await;
    let before = fixture
        .service
        .get_settings()
        .await
        .expect("before settings");
    fixture.service.fail_next_settings_write_after_model_probe();
    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "llmModelId": "gpt-task-008-uncommitted-success",
        "metadataEnabled": true
    }))
    .expect("candidate patch");

    let error = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 1,
            patch,
        })
        .await
        .expect_err("simulated settings persistence failure");
    assert_eq!(error.error_id, ErrorId::StorageFailed);
    assert_eq!(count(&fixture.probe_calls, Vec::len), 1);
    assert_eq!(
        fixture
            .service
            .get_settings()
            .await
            .expect("after settings"),
        before
    );
    let statuses = fixture
        .storage
        .repository()
        .load_provider_statuses()
        .await
        .expect("committed statuses");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].latest_status, ProviderOutcomeStatus::Success);
    let restarted = fixture
        .restarted_service()
        .get_settings()
        .await
        .expect("restart hydration");
    let openai = restarted
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Openai)
        .expect("OpenAI integration status");
    assert_eq!(openai.state, IntegrationState::Connected);
}

#[tokio::test]
async fn provider_unchanged_or_absent_model_patch_makes_zero_provider_calls() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-zero-call-key")
        .await;
    let current = fixture.service.get_settings().await.expect("settings");

    let same_model: SettingsPatch = serde_json::from_value(serde_json::json!({
        "llmModelId": current.llm_model_id,
        "metadataEnabled": true
    }))
    .expect("same-model patch");
    fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 1,
            patch: same_model,
        })
        .await
        .expect("same model saves without probe");

    let no_model: SettingsPatch = serde_json::from_value(serde_json::json!({
        "weatherEnabled": true
    }))
    .expect("non-model patch");
    fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 2,
            patch: no_model,
        })
        .await
        .expect("non-model patch saves without probe");
    assert_eq!(count(&fixture.probe_calls, Vec::len), 0);

    let invalid_merged_patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "llmModelId": "gpt-task-008-must-not-probe",
        "audioOutputBehavior": "fixed_device"
    }))
    .expect("invalid merged patch");
    let error = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 3,
            patch: invalid_merged_patch,
        })
        .await
        .expect_err("whole patch validates before provider use");
    assert_eq!(error.error_id, ErrorId::RequestInvalid);
    assert_eq!(count(&fixture.probe_calls, Vec::len), 0);
    assert_eq!(
        fixture
            .service
            .get_settings()
            .await
            .expect("unchanged settings")
            .revision,
        3
    );
}

#[tokio::test]
async fn provider_model_change_without_candidate_origin_credential_is_atomic_and_offline() {
    let fixture = Fixture::new().await;
    let before = fixture
        .service
        .get_settings()
        .await
        .expect("before settings");
    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "llmModelId": "gpt-task-008-no-credential",
        "metadataEnabled": true
    }))
    .expect("candidate patch");
    let error = fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch,
        })
        .await
        .expect_err("credential is required before a model probe");
    assert_eq!(error.error_id, ErrorId::SecretMissing);
    assert_eq!(count(&fixture.probe_calls, Vec::len), 0);
    assert_eq!(
        fixture
            .service
            .get_settings()
            .await
            .expect("after settings"),
        before
    );
}

#[tokio::test]
async fn provider_registry_tracks_integrations_independently() {
    let fixture = Fixture::new().await;
    let enable: SettingsPatch = serde_json::from_value(serde_json::json!({
        "metadataEnabled": true,
        "weatherEnabled": true
    }))
    .expect("enable patch");
    fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 0,
            patch: enable,
        })
        .await
        .expect("enable integrations");
    fixture
        .service
        .test_provider(TestProviderRequest {
            client_request_id: Uuid::now_v7(),
            kind: ProviderTestKind::Metadata,
        })
        .await
        .expect("metadata succeeds");
    *fixture.probe_result.lock().expect("probe result") =
        Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
    fixture
        .service
        .test_provider(TestProviderRequest {
            client_request_id: Uuid::now_v7(),
            kind: ProviderTestKind::Weather,
        })
        .await
        .expect_err("weather fails");

    let settings = fixture.service.get_settings().await.expect("settings");
    let metadata = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Musicbrainz)
        .expect("metadata status");
    let weather = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Weather)
        .expect("weather status");
    assert_eq!(metadata.state, IntegrationState::Connected);
    assert_eq!(weather.state, IntegrationState::Degraded);
}

#[tokio::test]
async fn provider_registry_distinguishes_apple_install_and_session_states() {
    let fixture = Fixture::new().await;

    fixture
        .service
        .refresh_apple_status(None, false, false)
        .await
        .expect("unknown installation status");
    let settings = fixture.service.get_settings().await.expect("settings");
    let apple = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::AppleMusic)
        .expect("Apple status");
    assert_eq!(apple.state, IntegrationState::Unavailable);
    assert!(apple.safe_message.contains("无法确认"));

    fixture
        .service
        .refresh_apple_status(Some(true), false, false)
        .await
        .expect("installed without session");
    let settings = fixture.service.get_settings().await.expect("settings");
    let apple = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::AppleMusic)
        .expect("Apple status");
    assert!(apple.safe_message.contains("当前没有媒体会话"));

    fixture
        .service
        .refresh_apple_status(Some(true), true, true)
        .await
        .expect("controllable session");
    let settings = fixture.service.get_settings().await.expect("settings");
    let apple = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::AppleMusic)
        .expect("Apple status");
    assert_eq!(apple.state, IntegrationState::Connected);
    assert_eq!(apple.last_success_at.as_deref(), Some(FIXED_NOW));
}

#[tokio::test]
async fn provider_registry_restores_integration_status_without_forging_origin_verification_time() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-restart-canary")
        .await;
    let enable: SettingsPatch = serde_json::from_value(serde_json::json!({
        "metadataEnabled": true
    }))
    .expect("enable patch");
    fixture
        .service
        .update_settings(UpdateSettingsRequest {
            client_request_id: Uuid::now_v7(),
            expected_revision: 1,
            patch: enable,
        })
        .await
        .expect("enable metadata");
    fixture
        .service
        .test_provider(TestProviderRequest {
            client_request_id: Uuid::now_v7(),
            kind: ProviderTestKind::Metadata,
        })
        .await
        .expect("metadata succeeds");

    let restarted = ProviderService::new(
        fixture.storage.repository(),
        Box::new(FakeVault {
            state: fixture.vault.clone(),
        }),
        Arc::new(FakeValidator {
            calls: Arc::clone(&fixture.validator_calls),
            result: Arc::clone(&fixture.validator_result),
        }),
        Arc::new(FakeProbe {
            calls: Arc::clone(&fixture.probe_calls),
            result: Arc::clone(&fixture.probe_result),
            expected_secret: Arc::clone(&fixture.probe_expected_secret),
        }),
        Arc::new(FakePreviewer {
            calls: Arc::clone(&fixture.preview_calls),
            operation_ids: Arc::clone(&fixture.preview_operation_ids),
            cancellations: Arc::clone(&fixture.preview_cancellations),
            result: Arc::clone(&fixture.preview_result),
            delay: Arc::clone(&fixture.preview_delay),
        }),
        Arc::new(RecordingPreviewEvents::default()),
        Arc::new(FixedClock),
    );
    let settings = restarted.get_settings().await.expect("restored settings");
    let openai = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Openai)
        .expect("OpenAI status");
    let metadata = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Musicbrainz)
        .expect("metadata status");

    assert_eq!(openai.state, IntegrationState::Connected);
    assert_eq!(openai.last_success_at.as_deref(), Some(FIXED_NOW));
    assert_eq!(metadata.state, IntegrationState::Connected);
    assert_eq!(metadata.last_success_at.as_deref(), Some(FIXED_NOW));
    assert!(settings.secret_status.origins[0].openai_api_key_configured);
    assert_eq!(settings.secret_status.origins[0].last_verified_at, None);
}

#[tokio::test]
async fn provider_preview_is_explicit_fixed_phrase_once_and_idempotent() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-preview-canary")
        .await;
    let request_id = Uuid::now_v7();
    let first = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: request_id,
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("preview accepted");
    let second = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: request_id,
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("retry cached");
    assert_eq!(first, second);
    wait_for_preview_events(&fixture.preview_events, 1).await;
    let calls = fixture.preview_calls.lock().expect("preview calls");
    assert_eq!(
        calls.as_slice(),
        &[("alloy".to_owned(), PREVIEW_PHRASE_V1.to_owned())]
    );
    let events = fixture.preview_events.0.lock().expect("preview events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].operation_id, first.operation_id);
    assert!(events[0].error.is_none());
}

#[tokio::test]
async fn provider_preview_failure_is_cached_without_automatic_replay() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-preview-fail-canary")
        .await;
    *fixture.preview_result.lock().expect("preview result") =
        Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
    let request_id = Uuid::now_v7();
    let mut responses = Vec::new();
    for _ in 0..2 {
        let accepted = fixture
            .service
            .preview_voice(PreviewVoiceRequest {
                client_request_id: request_id,
                voice_id: "alloy".to_owned(),
            })
            .await
            .expect("preview accepted before provider completion");
        responses.push(accepted);
    }
    assert_eq!(responses[0], responses[1]);
    wait_for_preview_events(&fixture.preview_events, 1).await;
    assert_eq!(count(&fixture.preview_calls, Vec::len), 1);
    {
        let events = fixture.preview_events.0.lock().expect("preview events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].error.as_ref().map(|error| error.error_id),
            Some(ErrorId::ProviderUnavailable)
        );
    }
    let settings = fixture.service.get_settings().await.expect("settings");
    let openai = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Openai)
        .expect("OpenAI status");
    assert_eq!(openai.state, IntegrationState::Degraded);
    assert_eq!(openai.safe_message, "服务或网络当前不可用。");
}

#[tokio::test]
async fn provider_preview_accepts_before_bounded_operation_times_out() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-timeout-canary")
        .await;
    *fixture.preview_delay.lock().expect("preview delay") = Duration::from_millis(200);

    fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: Uuid::now_v7(),
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("accepted without awaiting preview completion");
    assert_eq!(count(&fixture.preview_events.0, Vec::len), 0);
    wait_for_preview_calls(&fixture.preview_calls, 1).await;
    wait_for_preview_events(&fixture.preview_events, 1).await;
    {
        let events = fixture.preview_events.0.lock().expect("preview events");
        assert_eq!(
            events[0].error.as_ref().map(|error| error.error_id),
            Some(ErrorId::ProviderTimedOut)
        );
    }
    let settings = fixture.service.get_settings().await.expect("settings");
    let openai = settings
        .integration_statuses
        .iter()
        .find(|status| status.integration == Integration::Openai)
        .expect("OpenAI status");
    assert_eq!(openai.state, IntegrationState::Degraded);
    assert_eq!(openai.safe_message, "服务响应超时。");
    assert_eq!(count(&fixture.preview_calls, Vec::len), 1);
}

#[tokio::test]
async fn provider_preview_accept_write_failure_returns_error_without_spawning() {
    let fixture = Fixture::new().await;
    fixture
        .configure(
            "https://api.openai.com",
            "task-008-accept-persist-secret-canary",
        )
        .await;
    fixture.service.fail_next_preview_accept_persist();

    let error = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: Uuid::now_v7(),
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect_err("accept persistence failure must reject API-009");

    assert_eq!(error.error_id, ErrorId::StorageFailed);
    tokio::task::yield_now().await;
    assert_eq!(count(&fixture.preview_calls, Vec::len), 0);
    assert_eq!(fixture.preview_events.2.load(Ordering::Acquire), 0);
    assert_eq!(
        fixture
            .storage
            .repository()
            .count_voice_preview_recovery_work()
            .await
            .expect("outbox remains readable"),
        0
    );
}

#[tokio::test]
async fn provider_preview_publish_failure_leaves_authoritative_terminal_pending() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-008-publish-fail-canary")
        .await;
    fixture.preview_events.1.store(true, Ordering::Release);
    let accepted = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: Uuid::now_v7(),
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("preview accepted");
    wait_for_preview_publish_attempts(&fixture.preview_events, 1).await;

    assert!(fixture.preview_events.0.lock().expect("events").is_empty());
    let pending = fixture
        .storage
        .repository()
        .load_pending_voice_preview_terminals(10)
        .await
        .expect("pending outbox terminal");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].operation_id, accepted.operation_id);
    assert!(pending[0].error.is_none());
}

#[tokio::test]
async fn provider_preview_terminal_persist_failure_waits_for_authoritative_recovery() {
    let fixture = Fixture::new().await;
    fixture
        .configure(
            "https://api.openai.com",
            "task-008-terminal-persist-secret-canary",
        )
        .await;
    fixture.service.fail_next_preview_terminal_persist();
    let accepted = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: Uuid::now_v7(),
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("preview accepted");
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.service.preview_terminal_persist_failure_pending() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("terminal persistence attempt completed");

    assert_eq!(fixture.preview_events.2.load(Ordering::Acquire), 0);
    assert!(fixture.preview_events.0.lock().expect("events").is_empty());
    let pending = fixture
        .storage
        .repository()
        .load_pending_voice_preview_terminals(10)
        .await
        .expect("outbox remains readable");
    assert!(pending.is_empty());
    assert_eq!(
        fixture
            .storage
            .repository()
            .count_voice_preview_recovery_work()
            .await
            .expect("accepted row remains recoverable"),
        1
    );

    let recovered_events = RecordingPreviewEvents::default();
    let recovery = crate::providers::events::StartupVoicePreviewOutboxRecovery::new(
        fixture.storage.repository(),
        Arc::new(recovered_events.clone()),
        Arc::new(FixedClock),
    );
    let report = recovery
        .ensure_recovered()
        .await
        .expect("accepted operation recovers authoritatively");
    assert_eq!(report.recovered_accepted, 1);
    assert_eq!(report.delivered, 1);
    let recovered = recovered_events.0.lock().expect("recovered events");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].operation_id, accepted.operation_id);
    assert_eq!(
        recovered[0].error.as_ref().map(|error| error.error_id),
        Some(ErrorId::UnexpectedInternal)
    );
}

#[tokio::test]
async fn voice_preview_cancel_preparing_is_exact_idempotent_and_evt_011_only() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-017-cancel-secret-canary")
        .await;
    *fixture.preview_delay.lock().expect("preview delay") = Duration::from_millis(200);
    let accepted = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: Uuid::now_v7(),
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("preview accepted");
    wait_for_preview_calls(&fixture.preview_calls, 1).await;

    let first_request_id = Uuid::now_v7();
    let cancelled = fixture
        .service
        .cancel_operation(CancelOperationRequest {
            client_request_id: first_request_id,
            operation_id: accepted.operation_id,
            expected_kind: OperationKind::VoicePreview,
        })
        .await
        .expect("cancel wins while preparing");
    assert_eq!(cancelled.request_id, first_request_id);
    assert_eq!(cancelled.operation_id, accepted.operation_id);
    assert_eq!(cancelled.state, CancelOperationState::Cancelled);

    let replay = fixture
        .service
        .cancel_operation(CancelOperationRequest {
            client_request_id: first_request_id,
            operation_id: accepted.operation_id,
            expected_kind: OperationKind::VoicePreview,
        })
        .await
        .expect("same API-038 request is idempotent");
    assert_eq!(replay, cancelled);
    let second = fixture
        .service
        .cancel_operation(CancelOperationRequest {
            client_request_id: Uuid::now_v7(),
            operation_id: accepted.operation_id,
            expected_kind: OperationKind::VoicePreview,
        })
        .await
        .expect("later cancel observes terminal");
    assert_eq!(second.state, CancelOperationState::AlreadyTerminal);

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        fixture
            .preview_cancellations
            .lock()
            .expect("cancellations")
            .as_slice(),
        &[accepted.operation_id]
    );
    assert_eq!(
        fixture
            .preview_operation_ids
            .lock()
            .expect("operation ids")
            .as_slice(),
        &[accepted.operation_id]
    );
    let events = fixture.preview_events.0.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].operation_id, accepted.operation_id);
    assert!(events[0].cancelled);
    assert!(events[0].error.is_none());
    let serialized = serde_json::to_string(&(
        events[0].operation_id,
        events[0].cancelled,
        &events[0].error,
    ))
    .expect("safe event observation serializes");
    assert!(!serialized.contains("task-017-cancel-secret-canary"));
    assert!(!serialized.contains("C:\\Users\\"));
}

#[tokio::test]
async fn voice_preview_cancel_after_completed_is_already_terminal_without_stop() {
    let fixture = Fixture::new().await;
    fixture
        .configure("https://api.openai.com", "task-017-terminal-canary")
        .await;
    let accepted = fixture
        .service
        .preview_voice(PreviewVoiceRequest {
            client_request_id: Uuid::now_v7(),
            voice_id: "alloy".to_owned(),
        })
        .await
        .expect("preview accepted");
    wait_for_preview_events(&fixture.preview_events, 1).await;
    let response = fixture
        .service
        .cancel_operation(CancelOperationRequest {
            client_request_id: Uuid::now_v7(),
            operation_id: accepted.operation_id,
            expected_kind: OperationKind::VoicePreview,
        })
        .await
        .expect("completed operation is idempotently terminal");
    assert_eq!(response.state, CancelOperationState::AlreadyTerminal);
    assert!(
        fixture
            .preview_cancellations
            .lock()
            .expect("cancellations")
            .is_empty()
    );
    assert_eq!(fixture.preview_events.0.lock().expect("events").len(), 1);
}

#[tokio::test]
async fn api_038_accepts_closed_enum_but_rejects_unimplemented_kind_without_side_effect() {
    let fixture = Fixture::new().await;
    let error = fixture
        .service
        .cancel_operation(CancelOperationRequest {
            client_request_id: Uuid::now_v7(),
            operation_id: Uuid::now_v7(),
            expected_kind: OperationKind::Chat,
        })
        .await
        .expect_err("M3 slice supports only voice preview");
    assert_eq!(error.error_id, ErrorId::CapabilityUnsupported);
    assert_eq!(count(&fixture.preview_calls, Vec::len), 0);
    assert_eq!(count(&fixture.preview_cancellations, Vec::len), 0);
    assert_eq!(fixture.preview_events.2.load(Ordering::Acquire), 0);
}

#[test]
fn api_038_request_and_response_are_exact_closed_contract_shapes() {
    let request_id = Uuid::now_v7();
    let operation_id = Uuid::now_v7();
    for kind in ["chat", "voice_preview", "library_scan", "data_export"] {
        let request: CancelOperationRequest = serde_json::from_value(serde_json::json!({
            "clientRequestId": request_id,
            "operationId": operation_id,
            "expectedKind": kind
        }))
        .expect("closed operation kind");
        assert_eq!(request.client_request_id, request_id);
        assert_eq!(request.operation_id, operation_id);
    }
    assert!(
        serde_json::from_value::<CancelOperationRequest>(serde_json::json!({
            "clientRequestId": request_id,
            "operationId": operation_id,
            "expectedKind": "voice_preview",
            "secret": "sk-forbidden"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<CancelOperationRequest>(serde_json::json!({
            "clientRequestId": request_id,
            "operationId": operation_id,
            "expectedKind": "future_kind"
        }))
        .is_err()
    );
    let response = CancelOperationResponse {
        request_id,
        operation_id,
        state: CancelOperationState::Cancelled,
    };
    assert_eq!(
        serde_json::to_value(response).expect("response"),
        serde_json::json!({
            "requestId": request_id,
            "operationId": operation_id,
            "state": "cancelled"
        })
    );
}
