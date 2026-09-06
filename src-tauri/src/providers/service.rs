use super::{
    Ack, AppBehaviorSettings, AppSettingsEffect, AudioOutputBehavior, CancelOperationRequest,
    CancelOperationResponse, CancelOperationState, CandidateSecretValidator, Clock,
    DeleteSecretRequest, DeleteSecretResponse, Integration, ListVoicesRequest, NarrationDensity,
    OperationAccepted, OperationKind, OriginSecretStatus, PreviewVoiceRequest, ProviderCallContext,
    ProviderFailure, ProviderHealthProbe, ProviderTestInput, ProviderTestKind, SecretStatus,
    SecretValidationInput, SettingsPatch, SettingsView, TestProviderRequest, TestProviderResponse,
    UpdateSettingsRequest, ValidateSecretRequest, ValidateSecretResponse, VoicePreviewEventSink,
    VoicePreviewInput, VoicePreviewTerminal, VoicePreviewer, VoicesResponse, WeatherLocation,
};
use crate::{
    ipc::{ApiError, InternalReason, PublicField, RequestHash, Revision, canonical_request_hash},
    storage::{
        CanonicalOrigin, CredentialTarget, NewProviderOutcome, ProviderOutcomeStatus,
        ProviderRequestKind, ProviderStatusPromotion, ProviderStatusSnapshot,
        ProviderUsageProvider, Repository, SecretError, SecretVault, StorageError, StorageReason,
        StoredProviderSettings, StoredWeatherLocation,
    },
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::Arc,
    time::Instant,
};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

use super::dto::WeatherLocationAction;
use super::registry::ProviderRegistry;

pub const PREVIEW_PHRASE_V1: &str = "你好，我是 CyberKindred，很高兴陪你听一会儿。";

const SECRET_VALIDATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const PROVIDER_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const MODEL_CAPABILITY_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const PREVIEW_ACCEPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const PREVIEW_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const CANCEL_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const IDEMPOTENCY_WINDOW: std::time::Duration = std::time::Duration::from_mins(10);
const IDEMPOTENCY_CAPACITY: usize = 256;
const MAX_SETTING_TEXT_CHARS: usize = 100;

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
        client_request_id: Uuid,
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
            entries.retain(|_, stored| stored.expires_at > now);
            if let Some(stored) = entries.get(&client_request_id) {
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
                entries.insert(client_request_id, Arc::clone(&stored));
                stored
            }
        };

        // Only identical retries share this per-key lock. Different request IDs
        // never queue behind provider I/O in the endpoint-wide map lock.
        let mut stored_result = entry.result.lock().await;
        if let Some(result) = stored_result.as_ref() {
            return result.clone();
        }
        let result = operation().await;
        *stored_result = Some(result.clone());
        result
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretRequestFingerprint<'a> {
    client_request_id: Uuid,
    kind: super::SecretKind,
    origin: &'a str,
    value_digest: String,
}

struct ProviderOperationCompletion {
    finished: AtomicBool,
    notify: Notify,
}

impl ProviderOperationCompletion {
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

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

struct ProviderCompletionGuard(Arc<ProviderOperationCompletion>);

impl Drop for ProviderCompletionGuard {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }
}

/// Provider/settings application service. Construction and read-only calls have
/// no network, paid-provider, credential-write, or audio side effects.
pub struct ProviderService {
    repository: Repository,
    vault: Arc<Mutex<Box<dyn SecretVault>>>,
    validator: Arc<dyn CandidateSecretValidator>,
    health_probe: Arc<dyn ProviderHealthProbe>,
    voice_previewer: Arc<dyn VoicePreviewer>,
    preview_events: Arc<dyn VoicePreviewEventSink>,
    clock: Arc<dyn Clock>,
    provider_state_mutations: Mutex<()>,
    registry: Arc<Mutex<ProviderRegistry>>,
    registry_hydrated: Mutex<bool>,
    verified_secret: Mutex<Option<(String, String)>>,
    secret_validation_timeout: std::time::Duration,
    provider_test_timeout: std::time::Duration,
    preview_accept_timeout: std::time::Duration,
    preview_operation_timeout: std::time::Duration,
    validate_secret_requests: AsyncIdempotency<ValidateSecretResponse>,
    delete_secret_requests: AsyncIdempotency<DeleteSecretResponse>,
    provider_test_requests: AsyncIdempotency<TestProviderResponse>,
    update_settings_requests: AsyncIdempotency<Ack>,
    preview_voice_requests: AsyncIdempotency<OperationAccepted>,
    cancel_operation_requests: AsyncIdempotency<CancelOperationResponse>,
    settings_effect: Option<Arc<dyn AppSettingsEffect>>,
    accepting_previews: AtomicBool,
    preview_admission: Mutex<()>,
    preview_completions: Mutex<HashMap<Uuid, Arc<ProviderOperationCompletion>>>,
    #[cfg(test)]
    fail_next_settings_write_after_secret: AtomicBool,
    #[cfg(test)]
    fail_next_settings_write_after_model_probe: AtomicBool,
    #[cfg(test)]
    fail_next_preview_accept_persist: AtomicBool,
    #[cfg(test)]
    fail_next_preview_terminal_persist: Arc<AtomicBool>,
}

impl ProviderService {
    #[must_use]
    pub fn new(
        repository: Repository,
        vault: Box<dyn SecretVault>,
        validator: Arc<dyn CandidateSecretValidator>,
        health_probe: Arc<dyn ProviderHealthProbe>,
        voice_previewer: Arc<dyn VoicePreviewer>,
        preview_events: Arc<dyn VoicePreviewEventSink>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            vault: Arc::new(Mutex::new(vault)),
            validator,
            health_probe,
            voice_previewer,
            preview_events,
            clock,
            provider_state_mutations: Mutex::new(()),
            registry: Arc::new(Mutex::new(ProviderRegistry::default())),
            registry_hydrated: Mutex::new(false),
            verified_secret: Mutex::new(None),
            secret_validation_timeout: SECRET_VALIDATION_TIMEOUT,
            provider_test_timeout: PROVIDER_TEST_TIMEOUT,
            preview_accept_timeout: PREVIEW_ACCEPT_TIMEOUT,
            preview_operation_timeout: PREVIEW_OPERATION_TIMEOUT,
            validate_secret_requests: AsyncIdempotency::new(),
            delete_secret_requests: AsyncIdempotency::new(),
            provider_test_requests: AsyncIdempotency::new(),
            update_settings_requests: AsyncIdempotency::new(),
            preview_voice_requests: AsyncIdempotency::new(),
            cancel_operation_requests: AsyncIdempotency::new(),
            settings_effect: None,
            accepting_previews: AtomicBool::new(true),
            preview_admission: Mutex::new(()),
            preview_completions: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_next_settings_write_after_secret: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_settings_write_after_model_probe: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_preview_accept_persist: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_preview_terminal_persist: Arc::new(AtomicBool::new(false)),
        }
    }

    #[must_use]
    pub fn with_settings_effect(mut self, effect: Arc<dyn AppSettingsEffect>) -> Self {
        self.settings_effect = Some(effect);
        self
    }

    /// Refreshes the path-free Apple integration projection before API-007.
    ///
    /// # Errors
    ///
    /// Returns a stable storage error when the registry cannot be hydrated.
    pub async fn refresh_apple_status(
        &self,
        app_installed: Option<bool>,
        connected: bool,
        controllable: bool,
    ) -> Result<(), ApiError> {
        self.ensure_registry_hydrated().await?;
        self.registry.lock().await.apple_session(
            app_installed,
            connected,
            controllable,
            self.clock.now_rfc3339(),
        );
        Ok(())
    }

    #[cfg(test)]
    fn with_test_preview_timeouts(
        mut self,
        accept_timeout: std::time::Duration,
        operation_timeout: std::time::Duration,
    ) -> Self {
        self.preview_accept_timeout = accept_timeout;
        self.preview_operation_timeout = operation_timeout;
        self
    }

    #[cfg(test)]
    fn fail_next_settings_write_after_secret(&self) {
        self.fail_next_settings_write_after_secret
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn fail_next_settings_write_after_model_probe(&self) {
        self.fail_next_settings_write_after_model_probe
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn fail_next_preview_terminal_persist(&self) {
        self.fail_next_preview_terminal_persist
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn preview_terminal_persist_failure_pending(&self) -> bool {
        self.fail_next_preview_terminal_persist
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn fail_next_preview_accept_persist(&self) {
        self.fail_next_preview_accept_persist
            .store(true, Ordering::Release);
    }

    /// Returns API-007 without exposing a secret or a local path.
    ///
    /// # Errors
    ///
    /// Returns a stable safe error when settings or the exact origin-scoped
    /// credential status cannot be read.
    pub async fn get_settings(&self) -> Result<SettingsView, ApiError> {
        self.ensure_registry_hydrated().await?;
        let snapshots = self
            .repository
            .load_provider_statuses()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let settings = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let origin = parse_origin(&settings.provider_origin)?;
        let target = CredentialTarget::openai(&origin);
        let configured = self
            .vault
            .lock()
            .await
            .contains(&target)
            .map_err(map_secret_operation_error)?;

        let mut registry = self.registry.lock().await;
        hydrate_registry(&mut registry, &snapshots)?;
        registry.set_enabled(Integration::Musicbrainz, settings.metadata_enabled);
        registry.set_enabled(Integration::Weather, settings.weather_enabled);
        if configured {
            registry.credential_present();
        } else {
            registry.secret_deleted();
        }
        let statuses = registry.statuses();
        drop(registry);
        let last_verified_at = self
            .verified_secret
            .lock()
            .await
            .as_ref()
            .filter(|(verified_origin, _)| verified_origin == origin.as_str())
            .map(|(_, verified_at)| verified_at.clone());

        stored_to_view(settings, configured, last_verified_at, statuses)
    }

    /// Validates one candidate through the injected boundary before mutating the
    /// exact origin-scoped credential or selected provider origin.
    ///
    /// # Errors
    ///
    /// Returns the API contract's safe validation, provider, storage, or
    /// idempotency error. A failed validation preserves the previous settings
    /// and credential.
    pub async fn validate_and_set_secret(
        &self,
        request: ValidateSecretRequest,
    ) -> Result<ValidateSecretResponse, ApiError> {
        let value_digest = request.value.with_exposed(|value| {
            let digest = Sha256::digest(value.as_bytes());
            hex::encode(digest)
        });
        let request_hash = canonical_request_hash(&SecretRequestFingerprint {
            client_request_id: request.client_request_id,
            kind: request.kind,
            origin: &request.origin,
            value_digest,
        })?;
        let request_id = request.client_request_id;
        self.validate_secret_requests
            .execute(request_id, request_hash, || {
                self.validate_and_set_secret_once(request)
            })
            .await
    }

    async fn validate_and_set_secret_once(
        &self,
        request: ValidateSecretRequest,
    ) -> Result<ValidateSecretResponse, ApiError> {
        let _mutation_guard = self.provider_state_mutations.lock().await;
        self.ensure_registry_hydrated().await?;
        let candidate_origin = parse_origin(&request.origin)?;
        let context = ProviderCallContext::new(self.secret_validation_timeout);
        let started_at = Instant::now();
        let validation = tokio::time::timeout(
            self.secret_validation_timeout,
            self.validator.validate(
                SecretValidationInput {
                    origin: &candidate_origin,
                    candidate: &request.value,
                },
                &context,
            ),
        )
        .await
        .map_err(|_| ProviderFailure::new(super::ProviderFailureCategory::Timeout))
        .and_then(std::convert::identity);
        let candidate_outcome_id = self
            .record_provider_outcome(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::SecretValidation,
                None,
                elapsed_millis(started_at),
                outcome_status(&validation),
                context.correlation_id,
            )
            .await?;
        validation.map_err(|failure| failure.into_api_error(true))?;

        let previous = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let candidate_target = CredentialTarget::openai(&candidate_origin);

        let mut vault = self.vault.lock().await;
        let previous_candidate_secret = vault
            .get(&candidate_target)
            .map_err(map_secret_operation_error)?;
        vault
            .set(&candidate_target, request.value)
            .map_err(map_secret_operation_error)?;

        #[cfg(test)]
        if self
            .fail_next_settings_write_after_secret
            .swap(false, Ordering::AcqRel)
        {
            restore_candidate_write(vault.as_mut(), &candidate_target, previous_candidate_secret)?;
            return Err(ApiError::from_reason(InternalReason::StorageWriteFailed));
        }

        let mut updated = previous.clone();
        updated.provider_origin = candidate_origin.as_str().to_owned();
        if let Err(error) = self
            .repository
            .save_provider_settings(
                previous.revision,
                &updated,
                self.clock.now_ms(),
                false,
                Some(
                    ProviderStatusPromotion::secret_validation(candidate_outcome_id)
                        .map_err(|error| map_storage_error(&error))?,
                ),
            )
            .await
        {
            restore_candidate_write(vault.as_mut(), &candidate_target, previous_candidate_secret)?;
            return Err(map_storage_error(&error));
        }
        drop(vault);

        let verified_at = self.clock.now_rfc3339();
        self.registry.lock().await.configured(verified_at.clone());
        *self.verified_secret.lock().await =
            Some((candidate_origin.as_str().to_owned(), verified_at.clone()));
        Ok(ValidateSecretResponse {
            request_id: request.client_request_id,
            configured: true,
            verified_at,
        })
    }

    /// Deletes only the requested canonical origin's credential (API-005).
    ///
    /// # Errors
    ///
    /// Returns a stable safe request, credential, storage, or idempotency error.
    pub async fn delete_secret(
        &self,
        request: DeleteSecretRequest,
    ) -> Result<DeleteSecretResponse, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.delete_secret_requests
            .execute(request_id, request_hash, || {
                self.delete_secret_once(request)
            })
            .await
    }

    async fn delete_secret_once(
        &self,
        request: DeleteSecretRequest,
    ) -> Result<DeleteSecretResponse, ApiError> {
        let _mutation_guard = self.provider_state_mutations.lock().await;
        self.ensure_registry_hydrated().await?;
        let requested_origin = parse_origin(&request.origin)?;
        let target = CredentialTarget::openai(&requested_origin);
        let current = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        self.vault
            .lock()
            .await
            .delete(&target)
            .map_err(map_secret_operation_error)?;

        if current.provider_origin == requested_origin.as_str() {
            self.registry.lock().await.secret_deleted();
            *self.verified_secret.lock().await = None;
        }
        Ok(DeleteSecretResponse {
            request_id: request.client_request_id,
            configured: false,
        })
    }

    /// Runs one explicitly requested provider health probe without retries.
    ///
    /// # Errors
    ///
    /// Returns a stable safe secret, provider, storage, or idempotency error.
    pub async fn test_provider(
        &self,
        request: TestProviderRequest,
    ) -> Result<TestProviderResponse, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.provider_test_requests
            .execute(request_id, request_hash, || {
                self.test_provider_once(request)
            })
            .await
    }

    #[allow(clippy::too_many_lines)] // Exhaustive provider-kind branches keep request policy visible.
    async fn test_provider_once(
        &self,
        request: TestProviderRequest,
    ) -> Result<TestProviderResponse, ApiError> {
        self.ensure_registry_hydrated().await?;
        let settings = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let context = ProviderCallContext::new(self.provider_test_timeout);
        let started_at = Instant::now();
        let integration = integration_for_test(request.kind);
        let result = match request.kind {
            ProviderTestKind::Llm | ProviderTestKind::Tts => {
                let origin = parse_origin(&settings.provider_origin)?;
                let target = CredentialTarget::openai(&origin);
                let model_id = match request.kind {
                    ProviderTestKind::Llm => settings.llm_model_id.as_str(),
                    ProviderTestKind::Tts => settings.tts_model_id.as_str(),
                    ProviderTestKind::Metadata | ProviderTestKind::Weather => {
                        return Err(ApiError::unexpected());
                    }
                };
                let secret = self
                    .vault
                    .lock()
                    .await
                    .get(&target)
                    .map_err(map_secret_operation_error)?
                    .ok_or_else(|| ApiError::from_reason(InternalReason::SecretMissing))?;
                match tokio::time::timeout(
                    self.provider_test_timeout,
                    self.health_probe.test(
                        ProviderTestInput::OpenAi {
                            kind: request.kind,
                            origin: &origin,
                            secret: &secret,
                            model_id,
                        },
                        &context,
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(ProviderFailure::new(
                        super::ProviderFailureCategory::Timeout,
                    )),
                }
            }
            ProviderTestKind::Metadata => {
                match tokio::time::timeout(
                    self.provider_test_timeout,
                    self.health_probe
                        .test(ProviderTestInput::Metadata, &context),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(ProviderFailure::new(
                        super::ProviderFailureCategory::Timeout,
                    )),
                }
            }
            ProviderTestKind::Weather => {
                match tokio::time::timeout(
                    self.provider_test_timeout,
                    self.health_probe.test(ProviderTestInput::Weather, &context),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(ProviderFailure::new(
                        super::ProviderFailureCategory::Timeout,
                    )),
                }
            }
        };

        let latency_ms = result
            .as_ref()
            .copied()
            .unwrap_or_else(|_| elapsed_millis(started_at));
        self.record_provider_outcome(
            usage_provider_for_test(request.kind),
            ProviderRequestKind::HealthCheck,
            model_for_test(&settings, request.kind),
            latency_ms,
            outcome_status(&result),
            context.correlation_id,
        )
        .await?;

        match result {
            Ok(latency_ms) => {
                self.registry
                    .lock()
                    .await
                    .provider_success(integration, self.clock.now_rfc3339());
                Ok(TestProviderResponse {
                    request_id: request.client_request_id,
                    ok: true,
                    latency_ms,
                    safe_message: "连接测试成功。".to_owned(),
                })
            }
            Err(failure) => {
                self.registry
                    .lock()
                    .await
                    .provider_failure(integration, failure.category);
                Err(failure.into_api_error(false))
            }
        }
    }

    /// Applies the strict API-008 patch with optimistic concurrency.
    ///
    /// # Errors
    ///
    /// Returns a safe validation, revision, credential, storage, or idempotency
    /// error. Empty and unknown-field patches are rejected.
    pub async fn update_settings(&self, request: UpdateSettingsRequest) -> Result<Ack, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.update_settings_requests
            .execute(request_id, request_hash, || {
                self.update_settings_once(request)
            })
            .await
    }

    async fn update_settings_once(&self, request: UpdateSettingsRequest) -> Result<Ack, ApiError> {
        let _mutation_guard = self.provider_state_mutations.lock().await;
        self.ensure_registry_hydrated().await?;
        if request.patch.is_empty() {
            return Err(ApiError::from_reason(InternalReason::InvalidPatch)
                .with_field(PublicField::Request));
        }
        let current = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        Revision::new(current.revision).ensure_expected(request.expected_revision)?;
        let current_model_id = current.llm_model_id.clone();
        let current_behavior = AppBehaviorSettings {
            minimize_to_tray: current.minimize_to_tray,
            launch_at_startup: current.launch_at_startup,
        };
        let (updated, clear_weather_location) =
            self.apply_settings_patch(current, request.patch).await?;
        let updated_behavior = AppBehaviorSettings {
            minimize_to_tray: updated.minimize_to_tray,
            launch_at_startup: updated.launch_at_startup,
        };
        let model_changed = updated.llm_model_id != current_model_id;
        let status_promotion = if model_changed {
            Some(
                ProviderStatusPromotion::model_validation(
                    self.validate_candidate_model(&updated).await?,
                )
                .map_err(|error| map_storage_error(&error))?,
            )
        } else {
            None
        };
        let effect_applied = current_behavior != updated_behavior;
        if effect_applied && let Some(effect) = &self.settings_effect {
            effect.apply(current_behavior, updated_behavior)?;
        }
        #[cfg(test)]
        if model_changed
            && self
                .fail_next_settings_write_after_model_probe
                .swap(false, Ordering::AcqRel)
        {
            if effect_applied
                && let Some(effect) = &self.settings_effect
                && effect.apply(updated_behavior, current_behavior).is_err()
            {
                return Err(ApiError::unexpected());
            }
            return Err(ApiError::from_reason(InternalReason::StorageWriteFailed));
        }
        let revision = match self
            .repository
            .save_provider_settings(
                request.expected_revision,
                &updated,
                self.clock.now_ms(),
                clear_weather_location,
                status_promotion,
            )
            .await
        {
            Ok(revision) => revision,
            Err(error) => {
                if effect_applied
                    && let Some(effect) = &self.settings_effect
                    && effect.apply(updated_behavior, current_behavior).is_err()
                {
                    return Err(ApiError::unexpected());
                }
                return Err(map_storage_error(&error));
            }
        };
        let mut registry = self.registry.lock().await;
        registry.set_enabled(Integration::Musicbrainz, updated.metadata_enabled);
        registry.set_enabled(Integration::Weather, updated.weather_enabled);
        if model_changed {
            registry.provider_success(Integration::Openai, self.clock.now_rfc3339());
        }
        Ok(Ack {
            request_id: request.client_request_id,
            revision,
        })
    }

    async fn validate_candidate_model(
        &self,
        settings: &StoredProviderSettings,
    ) -> Result<Uuid, ApiError> {
        let origin = parse_origin(&settings.provider_origin)?;
        let secret = self
            .vault
            .lock()
            .await
            .get(&CredentialTarget::openai(&origin))
            .map_err(map_secret_operation_error)?
            .ok_or_else(|| ApiError::from_reason(InternalReason::SecretMissing))?;
        let context = ProviderCallContext::new(MODEL_CAPABILITY_PROBE_TIMEOUT);
        let started_at = Instant::now();
        let result = tokio::time::timeout(
            MODEL_CAPABILITY_PROBE_TIMEOUT,
            self.health_probe.test(
                ProviderTestInput::OpenAi {
                    kind: ProviderTestKind::Llm,
                    origin: &origin,
                    secret: &secret,
                    model_id: &settings.llm_model_id,
                },
                &context,
            ),
        )
        .await
        .map_err(|_| ProviderFailure::new(super::ProviderFailureCategory::Timeout))
        .and_then(std::convert::identity);
        let latency_ms = result
            .as_ref()
            .copied()
            .unwrap_or_else(|_| elapsed_millis(started_at));
        let candidate_outcome_id = self
            .record_provider_outcome(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::ModelValidation,
                Some(settings.llm_model_id.clone()),
                latency_ms,
                outcome_status(&result),
                context.correlation_id,
            )
            .await?;
        result
            .map(|_| candidate_outcome_id)
            .map_err(|failure| failure.into_api_error(false))
    }

    async fn apply_settings_patch(
        &self,
        mut settings: StoredProviderSettings,
        patch: SettingsPatch,
    ) -> Result<(StoredProviderSettings, bool), ApiError> {
        use super::dto::PatchField::Value;

        if let Value(value) = patch.provider_origin {
            let origin = parse_origin(&value)?;
            if origin.as_str() != settings.provider_origin {
                let configured = self
                    .vault
                    .lock()
                    .await
                    .contains(&CredentialTarget::openai(&origin))
                    .map_err(map_secret_operation_error)?;
                if !configured {
                    return Err(ApiError::from_reason(InternalReason::SecretMissing));
                }
            }
            settings.provider_origin = origin.as_str().to_owned();
        }
        if let Value(value) = patch.llm_model_id {
            validate_setting_text(&value)?;
            settings.llm_model_id = value;
        }
        if let Value(value) = patch.tts_model_id {
            validate_setting_text(&value)?;
            settings.tts_model_id = value;
        }
        if let Value(value) = patch.tts_voice_id {
            validate_setting_text(&value)?;
            let voices = validated_voices(self.voice_previewer.as_ref())?;
            if !voices.iter().any(|voice| voice.voice_id == value) {
                return Err(ApiError::from_reason(InternalReason::InvalidCandidate));
            }
            settings.tts_voice_id = value;
        }
        if let Value(value) = patch.metadata_enabled {
            settings.metadata_enabled = value;
        }
        if let Value(value) = patch.weather_enabled {
            settings.weather_enabled = value;
        }
        if let Value(value) = patch.default_source_id {
            if let Some(candidate) = &value {
                validate_setting_text(candidate)?;
                if !matches!(candidate.as_str(), "local" | "apple_music") {
                    return Err(ApiError::from_reason(InternalReason::InvalidCandidate));
                }
            }
            settings.default_source_id = value;
        }
        if let Value(value) = patch.narration_density {
            value.as_str().clone_into(&mut settings.narration_density);
        }
        if let Value(value) = patch.tts_enabled {
            settings.tts_enabled = value;
        }
        if let Value(value) = patch.audio_output_device_id {
            if let Some(candidate) = &value {
                validate_setting_text(candidate)?;
            }
            settings.audio_output_device_id = value;
        }
        if let Value(value) = patch.audio_output_behavior {
            value
                .as_str()
                .clone_into(&mut settings.audio_output_behavior);
        }
        if let Value(value) = patch.minimize_to_tray {
            settings.minimize_to_tray = value;
        }
        if let Value(value) = patch.launch_at_startup {
            settings.launch_at_startup = value;
        }
        if let Value(value) = patch.notifications_enabled {
            settings.notifications_enabled = value;
        }
        let clear_weather_location = matches!(
            patch.weather_location_action,
            Value(WeatherLocationAction::Clear)
        );

        if settings.audio_output_behavior == AudioOutputBehavior::FixedDevice.as_str()
            && settings.audio_output_device_id.is_none()
        {
            return Err(ApiError::from_reason(InternalReason::InvalidPatch));
        }
        Ok((settings, clear_weather_location))
    }

    /// Lists the injected adapter's static/read-only TTS voice catalog (API-043).
    ///
    /// # Errors
    ///
    /// Returns `ERR-1305` if the adapter supplies an invalid catalog.
    pub fn list_voices(&self, _request: ListVoicesRequest) -> Result<VoicesResponse, ApiError> {
        Ok(VoicesResponse {
            voices: validated_voices(self.voice_previewer.as_ref())?,
        })
    }

    /// Performs one explicit API-009 preview with the fixed Chinese phrase and
    /// no automatic replay.
    ///
    /// # Errors
    ///
    /// Returns a stable safe secret, provider, validation, storage, or
    /// idempotency error.
    pub async fn preview_voice(
        &self,
        request: PreviewVoiceRequest,
    ) -> Result<OperationAccepted, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.preview_voice_requests
            .execute(request_id, request_hash, || {
                self.preview_voice_once(request)
            })
            .await
    }

    /// Cancels the API-038 voice-preview slice against the durable terminal.
    ///
    /// # Errors
    ///
    /// Returns a stable validation, unsupported-kind, not-found, timeout, or
    /// storage error. No event or playback stop occurs unless cancellation wins
    /// the authoritative SQLite transition.
    pub async fn cancel_operation(
        &self,
        request: CancelOperationRequest,
    ) -> Result<CancelOperationResponse, ApiError> {
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.cancel_operation_requests
            .execute(request_id, request_hash, || async move {
                tokio::time::timeout(
                    CANCEL_OPERATION_TIMEOUT,
                    self.cancel_operation_once(request),
                )
                .await
                .map_err(|_| ApiError::from_reason(InternalReason::ResourceBusy))?
            })
            .await
    }

    pub(crate) async fn quiesce_for_reset(&self) -> Result<(), ApiError> {
        let _admission = self.preview_admission.lock().await;
        self.accepting_previews.store(false, Ordering::Release);
        for operation_id in self
            .repository
            .load_active_voice_preview_ids()
            .await
            .map_err(|error| map_storage_error(&error))?
        {
            self.cancel_operation_once(CancelOperationRequest {
                client_request_id: Uuid::now_v7(),
                operation_id,
                expected_kind: OperationKind::VoicePreview,
            })
            .await?;
        }
        let completions = self
            .preview_completions
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for completion in completions {
            completion.wait().await;
        }
        Ok(())
    }

    async fn cancel_operation_once(
        &self,
        request: CancelOperationRequest,
    ) -> Result<CancelOperationResponse, ApiError> {
        if request.expected_kind != OperationKind::VoicePreview {
            return Err(ApiError::from_reason(InternalReason::CapabilityAbsent));
        }
        let (cancelled, record) = self
            .repository
            .cancel_voice_preview(request.operation_id, self.clock.now_rfc3339())
            .await
            .map_err(|error| map_storage_error(&error))?;
        if cancelled {
            let _cancel_disposition = self.voice_previewer.cancel(request.operation_id);
            if record.delivered_at_ms.is_none() {
                let terminal = VoicePreviewTerminal {
                    operation_id: record.operation_id,
                    occurred_at: record.occurred_at.clone(),
                    error: None,
                    cancelled: true,
                };
                if self.preview_events.publish(terminal).is_ok() {
                    let _delivery_mark = self
                        .repository
                        .mark_voice_preview_terminal_delivered(
                            record.outbox_id,
                            record.operation_id,
                            self.clock.now_ms(),
                        )
                        .await;
                }
            }
        }
        Ok(CancelOperationResponse {
            request_id: request.client_request_id,
            operation_id: request.operation_id,
            state: if cancelled {
                CancelOperationState::Cancelled
            } else {
                CancelOperationState::AlreadyTerminal
            },
        })
    }

    #[allow(clippy::too_many_lines)] // The accepted background operation keeps terminal ordering explicit.
    async fn preview_voice_once(
        &self,
        request: PreviewVoiceRequest,
    ) -> Result<OperationAccepted, ApiError> {
        let _admission = self.preview_admission.lock().await;
        if !self.accepting_previews.load(Ordering::Acquire) {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        self.ensure_registry_hydrated().await?;
        let prepared = tokio::time::timeout(
            self.preview_accept_timeout,
            self.prepare_voice_preview(&request.voice_id),
        )
        .await
        .map_err(|_| provider_timeout_error())??;
        let accepted = OperationAccepted {
            operation_id: Uuid::now_v7(),
            accepted_at: self.clock.now_rfc3339(),
        };
        #[cfg(test)]
        let persisted_accept = if self
            .fail_next_preview_accept_persist
            .swap(false, Ordering::AcqRel)
        {
            Err(StorageError::new(StorageReason::StorageWriteFailed))
        } else {
            self.repository
                .persist_voice_preview_accepted(accepted.operation_id, accepted.accepted_at.clone())
                .await
        };
        #[cfg(not(test))]
        let persisted_accept = self
            .repository
            .persist_voice_preview_accepted(accepted.operation_id, accepted.accepted_at.clone())
            .await;
        persisted_accept.map_err(|error| map_storage_error(&error))?;

        let operation_id = accepted.operation_id;
        let completion = Arc::new(ProviderOperationCompletion::new());
        let mut completions = self.preview_completions.lock().await;
        completions.retain(|_, completion| !completion.is_finished());
        completions.insert(operation_id, Arc::clone(&completion));
        drop(completions);
        let previewer = Arc::clone(&self.voice_previewer);
        let preview_events = Arc::clone(&self.preview_events);
        let registry = Arc::clone(&self.registry);
        let clock = Arc::clone(&self.clock);
        let operation_timeout = self.preview_operation_timeout;
        let repository = self.repository.clone();
        #[cfg(test)]
        let fail_next_terminal_persist = Arc::clone(&self.fail_next_preview_terminal_persist);
        tokio::spawn(async move {
            let _completion = ProviderCompletionGuard(completion);
            let context = ProviderCallContext::new(operation_timeout);
            let started_at = Instant::now();
            let result = if let Ok(result) = tokio::time::timeout(
                operation_timeout,
                previewer.preview(
                    VoicePreviewInput {
                        operation_id,
                        origin: &prepared.origin,
                        secret: &prepared.secret,
                        model_id: &prepared.model_id,
                        voice_id: &prepared.voice_id,
                        text: PREVIEW_PHRASE_V1,
                    },
                    &context,
                ),
            )
            .await
            {
                if previewer.is_cancelled(operation_id) {
                    return;
                }
                result
            } else {
                if previewer.is_cancelled(operation_id) {
                    return;
                }
                let _cancel_disposition = previewer.cancel(operation_id);
                Err(ProviderFailure::new(
                    super::ProviderFailureCategory::Timeout,
                ))
            };
            let status = outcome_status(&result);
            let persisted = NewProviderOutcome::new(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::VoicePreview,
                Some(prepared.model_id.clone()),
                elapsed_millis(started_at),
                status,
                context.correlation_id,
                clock.now_ms(),
            )
            .map_err(|error| map_storage_error(&error));
            let persisted = match persisted {
                Ok(outcome) => repository
                    .record_provider_outcome(outcome)
                    .await
                    .map(|_| ())
                    .map_err(|error| map_storage_error(&error)),
                Err(error) => Err(error),
            };
            let terminal_error = match (result, persisted) {
                (_, Err(error)) => {
                    registry.lock().await.provider_failure(
                        Integration::Openai,
                        super::ProviderFailureCategory::Unavailable,
                    );
                    Some(error)
                }
                (Ok(()), Ok(())) => {
                    registry
                        .lock()
                        .await
                        .provider_success(Integration::Openai, clock.now_rfc3339());
                    None
                }
                (Err(failure), Ok(())) => {
                    let category = failure.category;
                    registry
                        .lock()
                        .await
                        .provider_failure(Integration::Openai, category);
                    Some(failure.into_api_error(false))
                }
            };
            let terminal = VoicePreviewTerminal {
                operation_id,
                occurred_at: clock.now_rfc3339(),
                error: terminal_error,
                cancelled: false,
            };
            #[cfg(test)]
            let persisted_terminal = if fail_next_terminal_persist.swap(false, Ordering::AcqRel) {
                Err(StorageError::new(StorageReason::StorageWriteFailed))
            } else {
                repository
                    .persist_voice_preview_terminal(
                        terminal.operation_id,
                        terminal.occurred_at.clone(),
                        terminal.error.clone(),
                    )
                    .await
            };
            #[cfg(not(test))]
            let persisted_terminal = repository
                .persist_voice_preview_terminal(
                    terminal.operation_id,
                    terminal.occurred_at.clone(),
                    terminal.error.clone(),
                )
                .await;
            let Ok(outbox_record) = persisted_terminal else {
                registry.lock().await.provider_failure(
                    Integration::Openai,
                    super::ProviderFailureCategory::Unavailable,
                );
                // The accepted row remains pending for startup recovery.
                // Never emit a terminal that was not first made authoritative
                // with its outbox record.
                return;
            };
            if outbox_record.delivered_at_ms.is_some() {
                return;
            }
            if preview_events.publish(terminal).is_err() {
                registry.lock().await.provider_failure(
                    Integration::Openai,
                    super::ProviderFailureCategory::Unavailable,
                );
                return;
            }
            if repository
                .mark_voice_preview_terminal_delivered(
                    outbox_record.outbox_id,
                    outbox_record.operation_id,
                    clock.now_ms(),
                )
                .await
                .is_err()
            {
                registry.lock().await.provider_failure(
                    Integration::Openai,
                    super::ProviderFailureCategory::Unavailable,
                );
            }
        });
        Ok(accepted)
    }

    async fn ensure_registry_hydrated(&self) -> Result<(), ApiError> {
        let mut hydrated = self.registry_hydrated.lock().await;
        if *hydrated {
            return Ok(());
        }
        let snapshots = self
            .repository
            .load_provider_statuses()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let mut restored = ProviderRegistry::default();
        hydrate_registry(&mut restored, &snapshots)?;
        *self.registry.lock().await = restored;
        *hydrated = true;
        Ok(())
    }

    async fn record_provider_outcome(
        &self,
        provider: ProviderUsageProvider,
        request_kind: ProviderRequestKind,
        model: Option<String>,
        latency_ms: u64,
        status: ProviderOutcomeStatus,
        correlation_id: Uuid,
    ) -> Result<Uuid, ApiError> {
        let outcome = NewProviderOutcome::new(
            provider,
            request_kind,
            model,
            latency_ms,
            status,
            correlation_id,
            self.clock.now_ms(),
        )
        .map_err(|error| map_storage_error(&error))?;
        self.repository
            .record_provider_outcome(outcome)
            .await
            .map_err(|error| map_storage_error(&error))
    }

    async fn prepare_voice_preview(
        &self,
        voice_id: &str,
    ) -> Result<PreparedVoicePreview, ApiError> {
        let voices = validated_voices(self.voice_previewer.as_ref())?;
        if !voices
            .iter()
            .any(|voice| voice.voice_id == voice_id && voice.preview_available)
        {
            return Err(ApiError::from_reason(InternalReason::InvalidCandidate));
        }
        let settings = self
            .repository
            .load_provider_settings()
            .await
            .map_err(|error| map_storage_error(&error))?;
        let origin = parse_origin(&settings.provider_origin)?;
        let secret = self
            .vault
            .lock()
            .await
            .get(&CredentialTarget::openai(&origin))
            .map_err(map_secret_operation_error)?
            .ok_or_else(|| ApiError::from_reason(InternalReason::SecretMissing))?;
        Ok(PreparedVoicePreview {
            origin,
            secret,
            model_id: settings.tts_model_id,
            voice_id: voice_id.to_owned(),
        })
    }
}

struct PreparedVoicePreview {
    origin: CanonicalOrigin,
    secret: crate::storage::SecretValue,
    model_id: String,
    voice_id: String,
}

fn hydrate_registry(
    registry: &mut ProviderRegistry,
    snapshots: &[ProviderStatusSnapshot],
) -> Result<(), ApiError> {
    let mut integrations: BTreeMap<Integration, (ProviderOutcomeStatus, i64, Option<i64>)> =
        BTreeMap::new();
    for snapshot in snapshots {
        let integration = match snapshot.provider {
            ProviderUsageProvider::OpenAi => Integration::Openai,
            ProviderUsageProvider::MusicBrainz | ProviderUsageProvider::CoverArtArchive => {
                Integration::Musicbrainz
            }
            ProviderUsageProvider::OpenMeteo => Integration::Weather,
        };
        if let Some((latest_status, latest_at, last_success_at)) =
            integrations.get_mut(&integration)
        {
            if snapshot.latest_outcome_at_ms > *latest_at {
                *latest_status = snapshot.latest_status;
                *latest_at = snapshot.latest_outcome_at_ms;
            }
            *last_success_at = match (*last_success_at, snapshot.last_success_at_ms) {
                (Some(current), Some(candidate)) => Some(current.max(candidate)),
                (current, None) => current,
                (None, candidate) => candidate,
            };
        } else {
            integrations.insert(
                integration,
                (
                    snapshot.latest_status,
                    snapshot.latest_outcome_at_ms,
                    snapshot.last_success_at_ms,
                ),
            );
        }
    }

    for (integration, (latest_status, latest_at, last_success_at)) in integrations {
        if let Some(last_success_at) = last_success_at {
            registry.provider_success(integration, timestamp_rfc3339(last_success_at)?);
        }
        match latest_status {
            ProviderOutcomeStatus::Success => {
                registry.provider_success(integration, timestamp_rfc3339(latest_at)?);
            }
            ProviderOutcomeStatus::Authentication => registry
                .provider_failure(integration, super::ProviderFailureCategory::Authentication),
            ProviderOutcomeStatus::RateLimit => {
                registry.provider_failure(integration, super::ProviderFailureCategory::RateLimit);
            }
            ProviderOutcomeStatus::Timeout => {
                registry.provider_failure(integration, super::ProviderFailureCategory::Timeout);
            }
            ProviderOutcomeStatus::Unavailable => {
                registry.provider_failure(integration, super::ProviderFailureCategory::Unavailable);
            }
            ProviderOutcomeStatus::InvalidResponse => registry
                .provider_failure(integration, super::ProviderFailureCategory::InvalidResponse),
        }
    }
    Ok(())
}

fn timestamp_rfc3339(timestamp_ms: i64) -> Result<String, ApiError> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| ApiError::from_reason(InternalReason::StorageIntegrityFailed))
}

fn outcome_status<T>(result: &Result<T, ProviderFailure>) -> ProviderOutcomeStatus {
    match result {
        Ok(_) => ProviderOutcomeStatus::Success,
        Err(failure) => match failure.category {
            super::ProviderFailureCategory::Authentication => ProviderOutcomeStatus::Authentication,
            super::ProviderFailureCategory::RateLimit => ProviderOutcomeStatus::RateLimit,
            super::ProviderFailureCategory::Timeout => ProviderOutcomeStatus::Timeout,
            super::ProviderFailureCategory::Unavailable => ProviderOutcomeStatus::Unavailable,
            super::ProviderFailureCategory::InvalidResponse => {
                ProviderOutcomeStatus::InvalidResponse
            }
        },
    }
}

const fn usage_provider_for_test(kind: ProviderTestKind) -> ProviderUsageProvider {
    match kind {
        ProviderTestKind::Llm | ProviderTestKind::Tts => ProviderUsageProvider::OpenAi,
        ProviderTestKind::Metadata => ProviderUsageProvider::MusicBrainz,
        ProviderTestKind::Weather => ProviderUsageProvider::OpenMeteo,
    }
}

fn model_for_test(settings: &StoredProviderSettings, kind: ProviderTestKind) -> Option<String> {
    match kind {
        ProviderTestKind::Llm => Some(settings.llm_model_id.clone()),
        ProviderTestKind::Tts => Some(settings.tts_model_id.clone()),
        ProviderTestKind::Metadata | ProviderTestKind::Weather => None,
    }
}

#[allow(clippy::manual_unwrap_or)] // Runtime paths avoid panic-associated unwrap APIs.
fn elapsed_millis(started_at: Instant) -> u64 {
    match u64::try_from(started_at.elapsed().as_millis()) {
        Ok(value) => value,
        Err(_) => u64::MAX,
    }
}

fn restore_candidate_write(
    vault: &mut dyn SecretVault,
    candidate_target: &CredentialTarget,
    previous_secret: Option<crate::storage::SecretValue>,
) -> Result<(), ApiError> {
    if let Some(secret) = previous_secret {
        vault
            .set(candidate_target, secret)
            .map_err(map_secret_operation_error)
    } else {
        vault
            .delete(candidate_target)
            .map_err(map_secret_operation_error)
    }
}

fn parse_origin(value: &str) -> Result<CanonicalOrigin, ApiError> {
    CanonicalOrigin::parse(value).map_err(|_| {
        ApiError::from_reason(InternalReason::InvalidCandidate).with_field(PublicField::Request)
    })
}

fn validate_setting_text(value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.chars().count() > MAX_SETTING_TEXT_CHARS {
        Err(ApiError::from_reason(InternalReason::InvalidPatch))
    } else {
        Ok(())
    }
}

fn provider_timeout_error() -> ApiError {
    ProviderFailure::new(super::ProviderFailureCategory::Timeout).into_api_error(false)
}

fn validated_voices(previewer: &dyn VoicePreviewer) -> Result<Vec<super::VoiceView>, ApiError> {
    let voices = previewer.voices();
    if voices.is_empty() {
        return Err(ApiError::from_reason(
            InternalReason::ProviderInvalidResponse,
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    for voice in &voices {
        if voice.voice_id.is_empty()
            || voice.voice_id.chars().count() > MAX_SETTING_TEXT_CHARS
            || voice.display_name.is_empty()
            || voice.display_name.chars().count() > MAX_SETTING_TEXT_CHARS
            || !ids.insert(&voice.voice_id)
        {
            return Err(ApiError::from_reason(
                InternalReason::ProviderInvalidResponse,
            ));
        }
    }
    Ok(voices)
}

const fn integration_for_test(kind: ProviderTestKind) -> Integration {
    match kind {
        ProviderTestKind::Llm | ProviderTestKind::Tts => Integration::Openai,
        ProviderTestKind::Metadata => Integration::Musicbrainz,
        ProviderTestKind::Weather => Integration::Weather,
    }
}

fn stored_to_view(
    stored: StoredProviderSettings,
    configured: bool,
    last_verified_at: Option<String>,
    integration_statuses: Vec<super::IntegrationStatus>,
) -> Result<SettingsView, ApiError> {
    let narration_density = match stored.narration_density.as_str() {
        "quiet" => NarrationDensity::Quiet,
        "balanced" => NarrationDensity::Balanced,
        "frequent" => NarrationDensity::Frequent,
        _ => {
            return Err(ApiError::from_reason(
                InternalReason::StorageIntegrityFailed,
            ));
        }
    };
    let audio_output_behavior = match stored.audio_output_behavior.as_str() {
        "follow_system_default" => AudioOutputBehavior::FollowSystemDefault,
        "fixed_device" => AudioOutputBehavior::FixedDevice,
        _ => {
            return Err(ApiError::from_reason(
                InternalReason::StorageIntegrityFailed,
            ));
        }
    };
    let weather_location = stored.weather_location.map(weather_location_from_stored);
    Ok(SettingsView {
        provider_origin: stored.provider_origin.clone(),
        llm_model_id: stored.llm_model_id,
        tts_model_id: stored.tts_model_id,
        tts_voice_id: stored.tts_voice_id,
        metadata_enabled: stored.metadata_enabled,
        weather_enabled: stored.weather_enabled,
        default_source_id: stored.default_source_id,
        narration_density,
        tts_enabled: stored.tts_enabled,
        audio_output_device_id: stored.audio_output_device_id,
        audio_output_behavior,
        minimize_to_tray: stored.minimize_to_tray,
        launch_at_startup: stored.launch_at_startup,
        notifications_enabled: stored.notifications_enabled,
        weather_location,
        secret_status: SecretStatus {
            origins: vec![OriginSecretStatus {
                origin: stored.provider_origin,
                openai_api_key_configured: configured,
                last_verified_at,
            }],
        },
        integration_statuses,
        revision: stored.revision,
    })
}

fn weather_location_from_stored(stored: StoredWeatherLocation) -> WeatherLocation {
    WeatherLocation {
        city: stored.city,
        region: stored.region,
        country: stored.country,
        country_code: stored.country_code,
        latitude: stored.latitude,
        longitude: stored.longitude,
        timezone: stored.timezone,
    }
}

fn map_secret_operation_error(error: SecretError) -> ApiError {
    match error {
        SecretError::InvalidOrigin | SecretError::InvalidValue => {
            ApiError::from_reason(InternalReason::InvalidCandidate)
        }
        SecretError::Unavailable
        | SecretError::OperationFailed
        | SecretError::ReplacementFailedRestored
        | SecretError::CredentialStateUnknown => {
            ApiError::from_reason(InternalReason::UnexpectedInternal)
        }
    }
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
