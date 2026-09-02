use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    Ack, CityScheduleMode, CompanionStyle, MusicSourceKind, NarrationDensity, OnboardingAiMode,
    OnboardingProfile, OnboardingState, OnboardingStep, OnboardingStepSubmission,
    OnboardingVoiceMode, PrivacyConfirmations, SaveOnboardingStepRequest,
};
use crate::{
    ipc::{ApiError, InternalReason, PublicField, RequestHash, canonical_request_hash},
    storage::{
        OnboardingStepWrite, OnboardingWriteError, Repository, StorageError, StorageReason,
        StoredOnboardingDocument, StoredOnboardingProfile, StoredOnboardingSnapshot,
    },
};

const IDEMPOTENCY_WINDOW: Duration = Duration::from_mins(10);
const IDEMPOTENCY_CAPACITY: usize = 256;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

struct IdempotencyEntry<T> {
    request_hash: RequestHash,
    expires_at: std::sync::Mutex<Option<Instant>>,
    result: Mutex<Option<Result<T, ApiError>>>,
}

impl<T> IdempotencyEntry<T> {
    fn retain_at(&self, now: Instant) -> bool {
        self.expires_at.lock().map_or(true, |expires_at| {
            expires_at.is_none_or(|value| value > now)
        })
    }

    fn mark_completed_at(&self, completed_at: Instant) {
        if let Ok(mut expires_at) = self.expires_at.lock() {
            *expires_at = Some(completed_at + IDEMPOTENCY_WINDOW);
        }
    }
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
            entries.retain(|_, stored| stored.retain_at(now));
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
                    expires_at: std::sync::Mutex::new(None),
                    result: Mutex::new(None),
                });
                entries.insert(client_request_id, Arc::clone(&stored));
                stored
            }
        };

        let mut stored_result = entry.result.lock().await;
        if let Some(result) = stored_result.as_ref() {
            return result.clone();
        }
        let result = operation().await;
        *stored_result = Some(result.clone());
        entry.mark_completed_at(Instant::now());
        result
    }
}

/// Application service for the authoritative, restart-safe onboarding aggregate.
///
/// It has no provider or audio dependency. API-003 records only prior explicit
/// choices and never initiates network, credential, notification, or audio work.
pub struct OnboardingService {
    repository: Repository,
    save_requests: AsyncIdempotency<Ack>,
}

impl OnboardingService {
    #[must_use]
    pub fn new(repository: Repository) -> Self {
        Self {
            repository,
            save_requests: AsyncIdempotency::new(),
        }
    }

    /// Returns API-002 from local persisted state.
    ///
    /// # Errors
    ///
    /// Returns a stable storage error when the aggregate cannot be safely rebuilt.
    pub async fn get_state(&self) -> Result<OnboardingState, ApiError> {
        self.repository
            .load_onboarding_snapshot()
            .await
            .map_err(|error| map_storage_error(&error))
            .and_then(snapshot_to_state)
    }

    /// Applies API-003 through a 10-minute payload-bound idempotency key.
    ///
    /// # Errors
    ///
    /// Returns a strict request, revision, transition, or storage error.
    pub async fn save_step(&self, request: SaveOnboardingStepRequest) -> Result<Ack, ApiError> {
        if request.expected_revision > MAX_JS_SAFE_INTEGER {
            return Err(ApiError::from_reason(InternalReason::RequestInvalid)
                .with_field(PublicField::ExpectedRevision));
        }
        let request_hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.save_requests
            .execute(request_id, request_hash, || self.save_step_once(request))
            .await
    }

    async fn save_step_once(&self, request: SaveOnboardingStepRequest) -> Result<Ack, ApiError> {
        let write = submission_to_write(request.submission)?;
        let revision = self
            .repository
            .save_onboarding_step(
                request.expected_revision,
                write,
                Utc::now().timestamp_millis(),
            )
            .await
            .map_err(map_write_error)?;
        Ok(Ack {
            request_id: request.client_request_id,
            revision,
        })
    }
}

fn submission_to_write(
    submission: OnboardingStepSubmission,
) -> Result<OnboardingStepWrite, ApiError> {
    match submission {
        OnboardingStepSubmission::Welcome {} => Ok(OnboardingStepWrite::Welcome),
        OnboardingStepSubmission::MusicSource { sources } => {
            if sources.is_empty()
                || sources
                    .iter()
                    .any(|source| sources.iter().filter(|item| *item == source).count() != 1)
            {
                return Err(invalid_request());
            }
            Ok(OnboardingStepWrite::MusicSource {
                sources: sources
                    .into_iter()
                    .map(|source| match source {
                        MusicSourceKind::Local => "local".to_owned(),
                        MusicSourceKind::AppleMusic => "apple_music".to_owned(),
                    })
                    .collect(),
            })
        }
        OnboardingStepSubmission::OpenaiKey { mode } => Ok(OnboardingStepWrite::OpenAiKey {
            mode: match mode {
                OnboardingAiMode::Verified => "verified",
                OnboardingAiMode::LocalOnly => "local_only",
            }
            .to_owned(),
        }),
        OnboardingStepSubmission::Voice { mode } => Ok(OnboardingStepWrite::Voice {
            mode: match mode {
                OnboardingVoiceMode::Selected => "selected",
                OnboardingVoiceMode::TextOnly => "text_only",
            }
            .to_owned(),
        }),
        OnboardingStepSubmission::Profile { profile } => {
            validate_profile(&profile)?;
            Ok(OnboardingStepWrite::Profile {
                profile: StoredOnboardingProfile {
                    display_name: profile.display_name,
                    companion_style: "quiet_warm".to_owned(),
                    initial_preferences: profile.initial_preferences,
                    narration_density: match profile.narration_density {
                        NarrationDensity::Quiet => "quiet",
                        NarrationDensity::Balanced => "balanced",
                        NarrationDensity::Frequent => "frequent",
                    }
                    .to_owned(),
                },
            })
        }
        OnboardingStepSubmission::CitySchedule { mode } => Ok(OnboardingStepWrite::CitySchedule {
            mode: match mode {
                CityScheduleMode::Configured => "configured",
                CityScheduleMode::NotNow => "not_now",
            }
            .to_owned(),
        }),
        OnboardingStepSubmission::Privacy { confirmations } => {
            if !confirmations.explicit_sound || !confirmations.raw_conversation_retention {
                return Err(invalid_request());
            }
            Ok(OnboardingStepWrite::Privacy)
        }
    }
}

fn validate_profile(profile: &OnboardingProfile) -> Result<(), ApiError> {
    if profile.display_name.chars().count() > 80
        || profile.companion_style != CompanionStyle::QuietWarm
        || profile.initial_preferences.len() > 20
        || profile
            .initial_preferences
            .iter()
            .any(|value| value.is_empty() || value.chars().count() > 100)
    {
        return Err(invalid_request());
    }
    Ok(())
}

fn snapshot_to_state(snapshot: StoredOnboardingSnapshot) -> Result<OnboardingState, ApiError> {
    let StoredOnboardingSnapshot { document, profile } = snapshot;
    let StoredOnboardingDocument {
        completed,
        completed_steps,
        source_selection,
        ai_mode,
        voice_mode,
        city_schedule_mode,
        privacy_confirmations,
        revision,
    } = document;
    Ok(OnboardingState {
        completed,
        completed_steps: completed_steps
            .into_iter()
            .map(|value| parse_step(&value))
            .collect::<Result<Vec<_>, _>>()?,
        source_selection: source_selection
            .into_iter()
            .map(|value| match value.as_str() {
                "local" => Ok(MusicSourceKind::Local),
                "apple_music" => Ok(MusicSourceKind::AppleMusic),
                _ => Err(integrity_error()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        ai_mode: ai_mode
            .map(|value| match value.as_str() {
                "verified" => Ok(OnboardingAiMode::Verified),
                "local_only" => Ok(OnboardingAiMode::LocalOnly),
                _ => Err(integrity_error()),
            })
            .transpose()?,
        voice_mode: voice_mode
            .map(|value| match value.as_str() {
                "selected" => Ok(OnboardingVoiceMode::Selected),
                "text_only" => Ok(OnboardingVoiceMode::TextOnly),
                _ => Err(integrity_error()),
            })
            .transpose()?,
        city_schedule_mode: city_schedule_mode
            .map(|value| match value.as_str() {
                "configured" => Ok(CityScheduleMode::Configured),
                "not_now" => Ok(CityScheduleMode::NotNow),
                _ => Err(integrity_error()),
            })
            .transpose()?,
        profile: stored_profile_to_dto(profile)?,
        privacy_confirmations: PrivacyConfirmations {
            explicit_sound: privacy_confirmations.explicit_sound,
            raw_conversation_retention: privacy_confirmations.raw_conversation_retention,
        },
        revision,
    })
}

fn parse_step(value: &str) -> Result<OnboardingStep, ApiError> {
    match value {
        "welcome" => Ok(OnboardingStep::Welcome),
        "music_source" => Ok(OnboardingStep::MusicSource),
        "openai_key" => Ok(OnboardingStep::OpenaiKey),
        "voice" => Ok(OnboardingStep::Voice),
        "profile" => Ok(OnboardingStep::Profile),
        "city_schedule" => Ok(OnboardingStep::CitySchedule),
        "privacy" => Ok(OnboardingStep::Privacy),
        _ => Err(integrity_error()),
    }
}

fn stored_profile_to_dto(profile: StoredOnboardingProfile) -> Result<OnboardingProfile, ApiError> {
    Ok(OnboardingProfile {
        display_name: profile.display_name,
        companion_style: match profile.companion_style.as_str() {
            "quiet_warm" => CompanionStyle::QuietWarm,
            _ => return Err(integrity_error()),
        },
        initial_preferences: profile.initial_preferences,
        narration_density: match profile.narration_density.as_str() {
            "quiet" => NarrationDensity::Quiet,
            "balanced" => NarrationDensity::Balanced,
            "frequent" => NarrationDensity::Frequent,
            _ => return Err(integrity_error()),
        },
    })
}

fn map_write_error(error: OnboardingWriteError) -> ApiError {
    match error {
        OnboardingWriteError::Storage(error) => map_storage_error(&error),
        OnboardingWriteError::RevisionConflict { current_revision } => {
            ApiError::from_reason(InternalReason::RevisionConflict)
                .with_field(PublicField::ExpectedRevision)
                .with_current_revision(current_revision)
        }
        OnboardingWriteError::InvalidTransition | OnboardingWriteError::LocalRootRequired => {
            invalid_request()
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

fn invalid_request() -> ApiError {
    ApiError::from_reason(InternalReason::InvalidCandidate).with_field(PublicField::Request)
}

fn integrity_error() -> ApiError {
    ApiError::from_reason(InternalReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod idempotency_tests {
    use super::*;

    #[test]
    fn live_entry_never_expires_and_completed_window_starts_at_completion() {
        let started_at = Instant::now();
        let entry = IdempotencyEntry::<Ack> {
            request_hash: canonical_request_hash(&"fixture").expect("request hash"),
            expires_at: std::sync::Mutex::new(None),
            result: Mutex::new(None),
        };
        let far_after_start = started_at + IDEMPOTENCY_WINDOW + IDEMPOTENCY_WINDOW;
        assert!(entry.retain_at(far_after_start));

        entry.mark_completed_at(far_after_start);
        assert!(entry.retain_at(far_after_start + IDEMPOTENCY_WINDOW / 2));
        assert!(!entry.retain_at(far_after_start + IDEMPOTENCY_WINDOW));
    }
}
