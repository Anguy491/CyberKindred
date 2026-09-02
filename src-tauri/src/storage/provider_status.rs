//! Strongly typed persistence for non-content provider reliability outcomes.

use uuid::Uuid;

use super::{Repository, StorageError, StorageReason};

const MAX_MODEL_CHARS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProviderUsageProvider {
    OpenAi,
    MusicBrainz,
    CoverArtArchive,
    OpenMeteo,
}

impl ProviderUsageProvider {
    const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::MusicBrainz => "musicbrainz",
            Self::CoverArtArchive => "cover_art_archive",
            Self::OpenMeteo => "open_meteo",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "openai" => Ok(Self::OpenAi),
            "musicbrainz" => Ok(Self::MusicBrainz),
            "cover_art_archive" => Ok(Self::CoverArtArchive),
            "open_meteo" => Ok(Self::OpenMeteo),
            _ => Err(integrity_error()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderRequestKind {
    SecretValidation,
    HealthCheck,
    ProgramGeneration,
    CompanionTurn,
    SessionSummary,
    Speech,
    VoicePreview,
    MetadataMatch,
    CoverFetch,
    LocationSearch,
    CurrentWeather,
}

impl ProviderRequestKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SecretValidation => "secret_validation",
            Self::HealthCheck => "health_check",
            Self::ProgramGeneration => "program_generation",
            Self::CompanionTurn => "companion_turn",
            Self::SessionSummary => "session_summary",
            Self::Speech => "speech",
            Self::VoicePreview => "voice_preview",
            Self::MetadataMatch => "metadata_match",
            Self::CoverFetch => "cover_fetch",
            Self::LocationSearch => "location_search",
            Self::CurrentWeather => "current_weather",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "secret_validation" => Ok(Self::SecretValidation),
            "health_check" => Ok(Self::HealthCheck),
            "program_generation" => Ok(Self::ProgramGeneration),
            "companion_turn" => Ok(Self::CompanionTurn),
            "session_summary" => Ok(Self::SessionSummary),
            "speech" => Ok(Self::Speech),
            "voice_preview" => Ok(Self::VoicePreview),
            "metadata_match" => Ok(Self::MetadataMatch),
            "cover_fetch" => Ok(Self::CoverFetch),
            "location_search" => Ok(Self::LocationSearch),
            "current_weather" => Ok(Self::CurrentWeather),
            _ => Err(integrity_error()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderOutcomeStatus {
    Success,
    Authentication,
    RateLimit,
    Timeout,
    Unavailable,
    InvalidResponse,
}

impl ProviderOutcomeStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Authentication => "authentication",
            Self::RateLimit => "rate_limit",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::InvalidResponse => "invalid_response",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "success" => Ok(Self::Success),
            "authentication" => Ok(Self::Authentication),
            "rate_limit" => Ok(Self::RateLimit),
            "timeout" => Ok(Self::Timeout),
            "unavailable" => Ok(Self::Unavailable),
            "invalid_response" => Ok(Self::InvalidResponse),
            _ => Err(integrity_error()),
        }
    }
}

/// Minimal outcome accepted by the provider usage repository.
///
/// The type has no request body, response body, secret, URL, or path field.
pub struct NewProviderOutcome {
    provider: ProviderUsageProvider,
    request_kind: ProviderRequestKind,
    model: Option<String>,
    latency_ms: u64,
    status: ProviderOutcomeStatus,
    correlation_id: Uuid,
    created_at_ms: i64,
}

impl NewProviderOutcome {
    /// Builds a validated, non-content provider outcome.
    ///
    /// # Errors
    ///
    /// Returns `storage_write_failed` for an invalid provider/request pairing,
    /// model identifier, timestamp, latency, or correlation UUID.
    pub fn new(
        provider: ProviderUsageProvider,
        request_kind: ProviderRequestKind,
        model: Option<String>,
        latency_ms: u64,
        status: ProviderOutcomeStatus,
        correlation_id: Uuid,
        created_at_ms: i64,
    ) -> Result<Self, StorageError> {
        validate_provider_request(provider, request_kind).map_err(|_| write_error())?;
        if let Some(model) = &model {
            validate_model(model).map_err(|_| write_error())?;
        }
        if latency_ms > i64::MAX.cast_unsigned() || correlation_id.is_nil() || created_at_ms < 0 {
            return Err(write_error());
        }
        Ok(Self {
            provider,
            request_kind,
            model,
            latency_ms,
            status,
            correlation_id,
            created_at_ms,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderStatusSnapshot {
    pub provider: ProviderUsageProvider,
    pub latest_status: ProviderOutcomeStatus,
    pub latest_outcome_at_ms: i64,
    pub last_success_at_ms: Option<i64>,
}

type ProviderOutcomeRow = (
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<f64>,
    i64,
    String,
    String,
    i64,
);

struct ValidatedOutcomeRow {
    id: Uuid,
    provider: ProviderUsageProvider,
    status: ProviderOutcomeStatus,
    created_at_ms: i64,
}

impl Repository {
    /// Persists one provider outcome without any request/response content.
    ///
    /// Token units and audio seconds are deliberately written as `NULL` by
    /// this M2 reliability slice; later metering has a separate typed path.
    ///
    /// # Errors
    ///
    /// Returns `storage_write_failed` if SQLite cannot atomically insert the
    /// already validated outcome.
    pub async fn record_provider_outcome(
        &self,
        outcome: NewProviderOutcome,
    ) -> Result<Uuid, StorageError> {
        let id = Uuid::now_v7();
        let latency_ms = i64::try_from(outcome.latency_ms).map_err(|_| write_error())?;
        sqlx::query(
            "INSERT INTO provider_usage(id, provider, request_kind, model, input_units, output_units, audio_seconds, latency_ms, status_class, correlation_id, created_at_ms) VALUES(?, ?, ?, ?, NULL, NULL, NULL, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(outcome.provider.as_str())
        .bind(outcome.request_kind.as_str())
        .bind(outcome.model)
        .bind(latency_ms)
        .bind(outcome.status.as_str())
        .bind(outcome.correlation_id.to_string())
        .bind(outcome.created_at_ms)
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        Ok(id)
    }

    /// Rebuilds each provider's latest status and last successful outcome.
    ///
    /// Every stored row is validated before a snapshot is returned. This makes
    /// unknown enum text, malformed UUIDs, invalid provider/request pairings,
    /// or corrupt numeric values fail closed after restart.
    ///
    /// # Errors
    ///
    /// Returns `storage_read_failed` for a query failure and
    /// `storage_integrity_failed` for any malformed stored row.
    pub async fn load_provider_statuses(
        &self,
    ) -> Result<Vec<ProviderStatusSnapshot>, StorageError> {
        let rows: Vec<ProviderOutcomeRow> = sqlx::query_as(
            "SELECT id, provider, request_kind, model, input_units, output_units, audio_seconds, latency_ms, status_class, correlation_id, created_at_ms FROM provider_usage ORDER BY provider ASC, created_at_ms ASC, id ASC",
        )
        .fetch_all(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;

        aggregate_provider_statuses(rows)
    }
}

fn aggregate_provider_statuses(
    rows: Vec<ProviderOutcomeRow>,
) -> Result<Vec<ProviderStatusSnapshot>, StorageError> {
    let mut snapshots: Vec<(ProviderStatusSnapshot, Uuid)> = Vec::new();
    for row in rows {
        let validated = validate_row(row)?;
        let existing = snapshots
            .iter_mut()
            .find(|(snapshot, _)| snapshot.provider == validated.provider);
        if let Some((snapshot, latest_id)) = existing {
            if validated.status == ProviderOutcomeStatus::Success {
                snapshot.last_success_at_ms = Some(
                    snapshot
                        .last_success_at_ms
                        .map_or(validated.created_at_ms, |current| {
                            current.max(validated.created_at_ms)
                        }),
                );
            }
            if (validated.created_at_ms, validated.id) > (snapshot.latest_outcome_at_ms, *latest_id)
            {
                snapshot.latest_status = validated.status;
                snapshot.latest_outcome_at_ms = validated.created_at_ms;
                *latest_id = validated.id;
            }
        } else {
            snapshots.push((
                ProviderStatusSnapshot {
                    provider: validated.provider,
                    latest_status: validated.status,
                    latest_outcome_at_ms: validated.created_at_ms,
                    last_success_at_ms: (validated.status == ProviderOutcomeStatus::Success)
                        .then_some(validated.created_at_ms),
                },
                validated.id,
            ));
        }
    }
    snapshots.sort_by_key(|(snapshot, _)| snapshot.provider);
    Ok(snapshots
        .into_iter()
        .map(|(snapshot, _)| snapshot)
        .collect())
}

fn validate_row(row: ProviderOutcomeRow) -> Result<ValidatedOutcomeRow, StorageError> {
    let (
        id,
        provider,
        request_kind,
        model,
        input_units,
        output_units,
        audio_seconds,
        latency_ms,
        status,
        correlation_id,
        created_at_ms,
    ) = row;
    let id = parse_uuid(&id)?;
    if id.get_version_num() != 7 {
        return Err(integrity_error());
    }
    let provider = ProviderUsageProvider::parse(&provider)?;
    let request_kind = ProviderRequestKind::parse(&request_kind)?;
    validate_provider_request(provider, request_kind)?;
    if let Some(model) = model {
        validate_model(&model)?;
    }
    validate_optional_nonnegative(input_units)?;
    validate_optional_nonnegative(output_units)?;
    if audio_seconds.is_some_and(|value| !value.is_finite() || value < 0.0)
        || latency_ms < 0
        || created_at_ms < 0
    {
        return Err(integrity_error());
    }
    let status = ProviderOutcomeStatus::parse(&status)?;
    let correlation_id = parse_uuid(&correlation_id)?;
    if correlation_id.is_nil() {
        return Err(integrity_error());
    }
    Ok(ValidatedOutcomeRow {
        id,
        provider,
        status,
        created_at_ms,
    })
}

fn validate_provider_request(
    provider: ProviderUsageProvider,
    request_kind: ProviderRequestKind,
) -> Result<(), StorageError> {
    let valid = match provider {
        ProviderUsageProvider::OpenAi => matches!(
            request_kind,
            ProviderRequestKind::SecretValidation
                | ProviderRequestKind::HealthCheck
                | ProviderRequestKind::ProgramGeneration
                | ProviderRequestKind::CompanionTurn
                | ProviderRequestKind::SessionSummary
                | ProviderRequestKind::Speech
                | ProviderRequestKind::VoicePreview
        ),
        ProviderUsageProvider::MusicBrainz => matches!(
            request_kind,
            ProviderRequestKind::HealthCheck | ProviderRequestKind::MetadataMatch
        ),
        ProviderUsageProvider::CoverArtArchive => request_kind == ProviderRequestKind::CoverFetch,
        ProviderUsageProvider::OpenMeteo => matches!(
            request_kind,
            ProviderRequestKind::HealthCheck
                | ProviderRequestKind::LocationSearch
                | ProviderRequestKind::CurrentWeather
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(integrity_error())
    }
}

fn validate_model(model: &str) -> Result<(), StorageError> {
    let count = model.chars().count();
    if count == 0
        || count > MAX_MODEL_CHARS
        || !model
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_:\u{40}".contains(character))
    {
        return Err(integrity_error());
    }
    Ok(())
}

fn validate_optional_nonnegative(value: Option<i64>) -> Result<(), StorageError> {
    if value.is_some_and(|value| value < 0) {
        Err(integrity_error())
    } else {
        Ok(())
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|_| integrity_error())
}

const fn write_error() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

const fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppPaths, Storage};

    async fn fixture() -> (tempfile::TempDir, AppPaths, Storage, Repository) {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = AppPaths::create(
            temp.path().join("data"),
            temp.path().join("cache"),
            temp.path().join("logs"),
        )
        .expect("scoped paths");
        let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
        let repository = storage.repository();
        (temp, paths, storage, repository)
    }

    fn outcome(status: ProviderOutcomeStatus, created_at_ms: i64) -> NewProviderOutcome {
        NewProviderOutcome::new(
            ProviderUsageProvider::OpenAi,
            ProviderRequestKind::HealthCheck,
            Some("gpt-5.4-mini".to_owned()),
            25,
            status,
            Uuid::now_v7(),
            created_at_ms,
        )
        .expect("valid outcome")
    }

    #[tokio::test]
    async fn provider_status_success_then_failure_survives_restart() {
        let (_temp, paths, storage, repository) = fixture().await;
        repository
            .record_provider_outcome(outcome(ProviderOutcomeStatus::Success, 100))
            .await
            .expect("success outcome");
        repository
            .record_provider_outcome(outcome(ProviderOutcomeStatus::Timeout, 200))
            .await
            .expect("failure outcome");
        drop(repository);
        storage.close().await;

        let reopened = Storage::open(&paths, "0.1.0")
            .await
            .expect("reopen storage");
        let statuses = reopened
            .repository()
            .load_provider_statuses()
            .await
            .expect("restore statuses");
        assert_eq!(
            statuses,
            vec![ProviderStatusSnapshot {
                provider: ProviderUsageProvider::OpenAi,
                latest_status: ProviderOutcomeStatus::Timeout,
                latest_outcome_at_ms: 200,
                last_success_at_ms: Some(100),
            }]
        );
        reopened.close().await;
    }

    #[tokio::test]
    async fn provider_status_rejects_unknown_or_corrupt_rows() {
        let (_temp, _paths, storage, repository) = fixture().await;
        sqlx::query(
            "INSERT INTO provider_usage(id, provider, request_kind, model, input_units, output_units, audio_seconds, latency_ms, status_class, correlation_id, created_at_ms) VALUES(?, 'unknown_provider', 'health_check', NULL, NULL, NULL, NULL, 0, 'success', ?, 1)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(Uuid::now_v7().to_string())
        .execute(&repository.writer)
        .await
        .expect("schema permits corruption fixture");

        let error = repository
            .load_provider_statuses()
            .await
            .expect_err("unknown provider must fail closed");
        assert_eq!(error.reason(), StorageReason::StorageIntegrityFailed);
        storage.close().await;
    }

    #[tokio::test]
    async fn provider_outcome_has_no_body_secret_path_or_metering_persistence() {
        const SECRET_CANARY: &str = "sk-provider-status-secret-canary";
        const PATH_CANARY: &str = "C:\\\\Users\\\\Canary\\\\private.mp3";
        let (temp, _paths, storage, repository) = fixture().await;
        let unsafe_model = format!("{SECRET_CANARY} {PATH_CANARY}");
        assert!(
            NewProviderOutcome::new(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::HealthCheck,
                Some(unsafe_model),
                0,
                ProviderOutcomeStatus::Success,
                Uuid::now_v7(),
                1,
            )
            .is_err()
        );
        repository
            .record_provider_outcome(outcome(ProviderOutcomeStatus::Success, 2))
            .await
            .expect("safe outcome");
        let stored: (Option<i64>, Option<i64>, Option<f64>) =
            sqlx::query_as("SELECT input_units, output_units, audio_seconds FROM provider_usage")
                .fetch_one(&repository.writer)
                .await
                .expect("stored outcome");
        assert_eq!(stored, (None, None, None));
        storage.passive_checkpoint().await.expect("checkpoint");

        for entry in std::fs::read_dir(temp.path().join("data")).expect("data directory") {
            let path = entry.expect("data entry").path();
            if path.is_file() {
                let bytes = std::fs::read(path).expect("read database artifact");
                for canary in [SECRET_CANARY, PATH_CANARY] {
                    assert!(
                        !bytes
                            .windows(canary.len())
                            .any(|window| window == canary.as_bytes()),
                        "forbidden canary must not enter provider usage storage"
                    );
                }
            }
        }
        storage.close().await;
    }

    #[test]
    fn provider_outcome_validates_every_input_before_write() {
        assert!(
            NewProviderOutcome::new(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::CoverFetch,
                None,
                0,
                ProviderOutcomeStatus::Success,
                Uuid::now_v7(),
                1,
            )
            .is_err()
        );
        assert!(
            NewProviderOutcome::new(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::HealthCheck,
                Some("contains/slash".to_owned()),
                0,
                ProviderOutcomeStatus::Success,
                Uuid::now_v7(),
                1,
            )
            .is_err()
        );
        assert!(
            NewProviderOutcome::new(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::HealthCheck,
                None,
                0,
                ProviderOutcomeStatus::Success,
                Uuid::nil(),
                1,
            )
            .is_err()
        );
        assert!(
            NewProviderOutcome::new(
                ProviderUsageProvider::OpenAi,
                ProviderRequestKind::HealthCheck,
                None,
                0,
                ProviderOutcomeStatus::Success,
                Uuid::now_v7(),
                -1,
            )
            .is_err()
        );
    }
}
