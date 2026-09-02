use super::{
    Repository, StorageError, StorageReason,
    library_roots::has_valid_enabled_library_root_in_transaction,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const ONBOARDING_KEY: &str = "ui.onboarding_state";
const NARRATION_DENSITY_KEY: &str = "program.narration_density";
const SETTING_SCHEMA_VERSION: i64 = 1;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const STEP_ORDER: [&str; 7] = [
    "welcome",
    "music_source",
    "openai_key",
    "voice",
    "profile",
    "city_schedule",
    "privacy",
];

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredOnboardingDocument {
    pub completed: bool,
    pub completed_steps: Vec<String>,
    pub source_selection: Vec<String>,
    pub ai_mode: Option<String>,
    pub voice_mode: Option<String>,
    pub city_schedule_mode: Option<String>,
    pub privacy_confirmations: StoredPrivacyConfirmations,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredPrivacyConfirmations {
    pub explicit_sound: bool,
    pub raw_conversation_retention: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredOnboardingProfile {
    pub display_name: String,
    pub companion_style: String,
    pub initial_preferences: Vec<String>,
    pub narration_density: String,
}

impl Default for StoredOnboardingProfile {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            companion_style: "quiet_warm".to_owned(),
            initial_preferences: Vec::new(),
            narration_density: "balanced".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoredOnboardingSnapshot {
    pub document: StoredOnboardingDocument,
    pub profile: StoredOnboardingProfile,
}

pub(crate) enum OnboardingStepWrite {
    Welcome,
    MusicSource { sources: Vec<String> },
    OpenAiKey { mode: String },
    Voice { mode: String },
    Profile { profile: StoredOnboardingProfile },
    CitySchedule { mode: String },
    Privacy,
}

impl OnboardingStepWrite {
    fn step_index(&self) -> usize {
        match self {
            Self::Welcome => 0,
            Self::MusicSource { .. } => 1,
            Self::OpenAiKey { .. } => 2,
            Self::Voice { .. } => 3,
            Self::Profile { .. } => 4,
            Self::CitySchedule { .. } => 5,
            Self::Privacy => 6,
        }
    }

    fn requires_local_root(&self) -> bool {
        matches!(
            self,
            Self::MusicSource { sources } if sources.iter().any(|source| source == "local")
        )
    }
}

#[derive(Debug)]
pub(crate) enum OnboardingWriteError {
    Storage(StorageError),
    RevisionConflict { current_revision: u64 },
    InvalidTransition,
    LocalRootRequired,
}

impl From<StorageError> for OnboardingWriteError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl Repository {
    pub(crate) async fn load_onboarding_snapshot(
        &self,
    ) -> Result<StoredOnboardingSnapshot, StorageError> {
        let mut transaction = self.writer.begin().await.map_err(|_| read_error())?;
        let document = load_document(&mut transaction).await?;
        let profile = load_profile(&mut transaction).await?;
        if document.completed_steps.len() > 4 && !profile_row_exists(&mut transaction).await? {
            return Err(integrity_error());
        }
        transaction.commit().await.map_err(|_| read_error())?;
        Ok(StoredOnboardingSnapshot { document, profile })
    }

    pub(crate) async fn save_onboarding_step(
        &self,
        expected_revision: u64,
        write: OnboardingStepWrite,
        updated_at_ms: i64,
    ) -> Result<u64, OnboardingWriteError> {
        if updated_at_ms < 0 {
            return Err(write_error().into());
        }
        let mut transaction = self
            .writer
            .begin()
            .await
            .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
        let mut document = load_document(&mut transaction)
            .await
            .map_err(OnboardingWriteError::Storage)?;
        if document.revision != expected_revision {
            return Err(OnboardingWriteError::RevisionConflict {
                current_revision: document.revision,
            });
        }
        let step_index = write.step_index();
        if step_index > document.completed_steps.len() {
            return Err(OnboardingWriteError::InvalidTransition);
        }
        if write.requires_local_root() {
            let has_valid_root = has_valid_enabled_library_root_in_transaction(&mut transaction)
                .await
                .map_err(OnboardingWriteError::Storage)?;
            if !has_valid_root {
                return Err(OnboardingWriteError::LocalRootRequired);
            }
        }

        let profile = apply_write(&mut document, write)?;
        if step_index == document.completed_steps.len() {
            document
                .completed_steps
                .push(STEP_ORDER[step_index].to_owned());
        }
        document.revision = document
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_JS_SAFE_INTEGER)
            .ok_or_else(|| OnboardingWriteError::Storage(write_error()))?;
        document.completed = document.completed_steps.len() == STEP_ORDER.len()
            && document.privacy_confirmations.explicit_sound
            && document.privacy_confirmations.raw_conversation_retention;
        validate_document(&document).map_err(OnboardingWriteError::Storage)?;
        let encoded = serde_json::to_string(&document)
            .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
        sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(ONBOARDING_KEY)
        .bind(encoded)
        .bind(SETTING_SCHEMA_VERSION)
        .bind(updated_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| OnboardingWriteError::Storage(write_error()))?;

        if let Some(profile) = profile {
            save_profile(&mut transaction, &profile, updated_at_ms).await?;
        }
        transaction
            .commit()
            .await
            .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
        Ok(document.revision)
    }

    #[cfg(test)]
    pub(crate) async fn reject_onboarding_profile_writes(&self) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TRIGGER onboarding_test_reject_profile BEFORE INSERT ON user_profile BEGIN SELECT RAISE(ABORT, 'fixture rejection'); END",
        )
        .execute(&self.writer)
        .await
        .map(|_| ())
        .map_err(|_| write_error())
    }

    #[cfg(test)]
    pub(crate) async fn store_onboarding_document_fixture(
        &self,
        document: &StoredOnboardingDocument,
    ) -> Result<(), StorageError> {
        let encoded = serde_json::to_string(document).map_err(|_| write_error())?;
        sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, ?, 0) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(ONBOARDING_KEY)
        .bind(encoded)
        .bind(SETTING_SCHEMA_VERSION)
        .execute(&self.writer)
        .await
        .map(|_| ())
        .map_err(|_| write_error())
    }

    #[cfg(test)]
    pub(crate) async fn store_settings_revision_fixture(
        &self,
        revision: u64,
    ) -> Result<(), StorageError> {
        let encoded = serde_json::to_string(&revision).map_err(|_| write_error())?;
        sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('ui.settings_revision', ?, ?, 0) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(encoded)
        .bind(SETTING_SCHEMA_VERSION)
        .execute(&self.writer)
        .await
        .map(|_| ())
        .map_err(|_| write_error())
    }

    #[cfg(test)]
    pub(crate) async fn onboarding_storage_values(
        &self,
    ) -> Result<
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        StorageError,
    > {
        let state: String = sqlx::query_scalar(
            "SELECT value_json FROM app_settings WHERE key = 'ui.onboarding_state'",
        )
        .fetch_one(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let narration: Option<String> = sqlx::query_scalar(
            "SELECT value_json FROM app_settings WHERE key = 'program.narration_density'",
        )
        .fetch_optional(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let settings_revision: Option<String> = sqlx::query_scalar(
            "SELECT value_json FROM app_settings WHERE key = 'ui.settings_revision'",
        )
        .fetch_optional(&self.writer)
        .await
        .map_err(|_| read_error())?;
        let profile: Option<(String, String)> = sqlx::query_as(
            "SELECT routine_json, program_preferences_json FROM user_profile WHERE id = 'current'",
        )
        .fetch_optional(&self.writer)
        .await
        .map_err(|_| read_error())?;
        Ok((
            state,
            narration,
            settings_revision,
            profile.as_ref().map(|value| value.0.clone()),
            profile.map(|value| value.1),
        ))
    }

    #[cfg(test)]
    pub(crate) async fn onboarding_external_effect_counts(
        &self,
    ) -> Result<(i64, i64, i64), StorageError> {
        let provider_usage = sqlx::query_scalar("SELECT count(*) FROM provider_usage")
            .fetch_one(&self.writer)
            .await
            .map_err(|_| read_error())?;
        let outbox_events = sqlx::query_scalar("SELECT count(*) FROM outbox_events")
            .fetch_one(&self.writer)
            .await
            .map_err(|_| read_error())?;
        let tts_cache_entries = sqlx::query_scalar("SELECT count(*) FROM tts_cache_entries")
            .fetch_one(&self.writer)
            .await
            .map_err(|_| read_error())?;
        Ok((provider_usage, outbox_events, tts_cache_entries))
    }
}

async fn load_document(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<StoredOnboardingDocument, StorageError> {
    let value: Option<(String, i64)> =
        sqlx::query_as("SELECT value_json, schema_version FROM app_settings WHERE key = ?")
            .bind(ONBOARDING_KEY)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| read_error())?;
    let document = match value {
        None => StoredOnboardingDocument::default(),
        Some((_, version)) if version != SETTING_SCHEMA_VERSION => return Err(integrity_error()),
        Some((encoded, _)) => serde_json::from_str(&encoded).map_err(|_| integrity_error())?,
    };
    validate_document(&document)?;
    Ok(document)
}

async fn profile_row_exists(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<bool, StorageError> {
    let exists: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_profile WHERE id = 'current')")
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| read_error())?;
    Ok(exists == 1)
}

async fn load_profile(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<StoredOnboardingProfile, StorageError> {
    let row: Option<(Option<String>, String, String)> = sqlx::query_as(
        "SELECT display_name, routine_json, program_preferences_json FROM user_profile WHERE id = 'current'",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| read_error())?;
    let Some((display_name, routine_json, preferences_json)) = row else {
        return Ok(StoredOnboardingProfile::default());
    };
    let routine = parse_object(&routine_json)?;
    let preferences = parse_object(&preferences_json)?;
    let companion_style = match optional_string(&routine, "companionStyle")? {
        Some(value) => value,
        None => "quiet_warm".to_owned(),
    };
    let initial_preferences =
        optional_string_array(&preferences, "initialPreferences")?.unwrap_or_default();
    let narration_density = match optional_string(&preferences, "narrationDensity")? {
        Some(value) => value,
        None => "balanced".to_owned(),
    };
    let profile = StoredOnboardingProfile {
        display_name: display_name.unwrap_or_default(),
        companion_style,
        initial_preferences,
        narration_density,
    };
    validate_profile(&profile)?;
    Ok(profile)
}

fn apply_write(
    document: &mut StoredOnboardingDocument,
    write: OnboardingStepWrite,
) -> Result<Option<StoredOnboardingProfile>, OnboardingWriteError> {
    match write {
        OnboardingStepWrite::Welcome => Ok(None),
        OnboardingStepWrite::MusicSource { sources } => {
            document.source_selection = sources;
            Ok(None)
        }
        OnboardingStepWrite::OpenAiKey { mode } => {
            document.ai_mode = Some(mode);
            Ok(None)
        }
        OnboardingStepWrite::Voice { mode } => {
            document.voice_mode = Some(mode);
            Ok(None)
        }
        OnboardingStepWrite::Profile { profile } => {
            validate_profile(&profile).map_err(OnboardingWriteError::Storage)?;
            Ok(Some(profile))
        }
        OnboardingStepWrite::CitySchedule { mode } => {
            document.city_schedule_mode = Some(mode);
            Ok(None)
        }
        OnboardingStepWrite::Privacy => {
            document.privacy_confirmations.explicit_sound = true;
            document.privacy_confirmations.raw_conversation_retention = true;
            Ok(None)
        }
    }
}

async fn save_profile(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    profile: &StoredOnboardingProfile,
    updated_at_ms: i64,
) -> Result<(), OnboardingWriteError> {
    let current_settings_revision: Option<(String, i64)> = sqlx::query_as(
        "SELECT value_json, schema_version FROM app_settings WHERE key = 'ui.settings_revision'",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    let current_settings_revision = current_settings_revision
        .map(|(encoded, schema_version)| {
            if schema_version != SETTING_SCHEMA_VERSION {
                return Err(OnboardingWriteError::Storage(integrity_error()));
            }
            serde_json::from_str::<u64>(&encoded)
                .map_err(|_| OnboardingWriteError::Storage(integrity_error()))
        })
        .transpose()?;
    let current_settings_revision = current_settings_revision.unwrap_or(0);
    if current_settings_revision > MAX_JS_SAFE_INTEGER {
        return Err(OnboardingWriteError::Storage(integrity_error()));
    }
    let next_settings_revision = current_settings_revision
        .checked_add(1)
        .filter(|revision| *revision <= MAX_JS_SAFE_INTEGER)
        .ok_or_else(|| OnboardingWriteError::Storage(write_error()))?;
    let existing: Option<(String, String)> = sqlx::query_as(
        "SELECT routine_json, program_preferences_json FROM user_profile WHERE id = 'current'",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    let mut routine = match existing.as_ref() {
        Some((value, _)) => parse_object(value).map_err(OnboardingWriteError::Storage)?,
        None => Map::new(),
    };
    let mut preferences = match existing.as_ref() {
        Some((_, value)) => parse_object(value).map_err(OnboardingWriteError::Storage)?,
        None => Map::new(),
    };
    routine.insert(
        "companionStyle".to_owned(),
        Value::String(profile.companion_style.clone()),
    );
    preferences.insert(
        "initialPreferences".to_owned(),
        serde_json::to_value(&profile.initial_preferences)
            .map_err(|_| OnboardingWriteError::Storage(write_error()))?,
    );
    preferences.insert(
        "narrationDensity".to_owned(),
        Value::String(profile.narration_density.clone()),
    );
    let routine_json = serde_json::to_string(&routine)
        .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    let preferences_json = serde_json::to_string(&preferences)
        .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    let display_name = (!profile.display_name.is_empty()).then_some(profile.display_name.as_str());
    sqlx::query(
        "INSERT INTO user_profile(id, display_name, locale, timezone, routine_json, program_preferences_json, profile_revision, created_at_ms, updated_at_ms) VALUES('current', ?, 'zh-CN', 'UTC', ?, ?, 1, ?, ?) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, routine_json = excluded.routine_json, program_preferences_json = excluded.program_preferences_json, profile_revision = user_profile.profile_revision + 1, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(display_name)
    .bind(routine_json)
    .bind(preferences_json)
    .bind(updated_at_ms)
    .bind(updated_at_ms)
    .execute(&mut **transaction)
    .await
    .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    let narration = serde_json::to_string(&profile.narration_density)
        .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    sqlx::query(
        "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(NARRATION_DENSITY_KEY)
    .bind(narration)
    .bind(SETTING_SCHEMA_VERSION)
    .bind(updated_at_ms)
    .execute(&mut **transaction)
    .await
    .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    let settings_revision = serde_json::to_string(&next_settings_revision)
        .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    sqlx::query(
        "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('ui.settings_revision', ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
    )
    .bind(settings_revision)
    .bind(SETTING_SCHEMA_VERSION)
    .bind(updated_at_ms)
    .execute(&mut **transaction)
    .await
    .map_err(|_| OnboardingWriteError::Storage(write_error()))?;
    Ok(())
}

fn validate_document(document: &StoredOnboardingDocument) -> Result<(), StorageError> {
    let completed_count = document.completed_steps.len();
    if document.revision > MAX_JS_SAFE_INTEGER
        || document.revision < completed_count as u64
        || completed_count > STEP_ORDER.len()
        || document
            .completed_steps
            .iter()
            .enumerate()
            .any(|(index, step)| step != STEP_ORDER[index])
        || !valid_sources(&document.source_selection)
        || !optional_allowed(document.ai_mode.as_deref(), &["verified", "local_only"])
        || !optional_allowed(document.voice_mode.as_deref(), &["selected", "text_only"])
        || !optional_allowed(
            document.city_schedule_mode.as_deref(),
            &["configured", "not_now"],
        )
    {
        return Err(integrity_error());
    }
    let completed = completed_count == STEP_ORDER.len()
        && document.privacy_confirmations.explicit_sound
        && document.privacy_confirmations.raw_conversation_retention;
    if document.completed != completed
        || (completed_count < 2 && !document.source_selection.is_empty())
        || (completed_count > 1 && document.source_selection.is_empty())
        || (completed_count < 3 && document.ai_mode.is_some())
        || (completed_count > 2 && document.ai_mode.is_none())
        || (completed_count < 4 && document.voice_mode.is_some())
        || (completed_count > 3 && document.voice_mode.is_none())
        || (completed_count < 6 && document.city_schedule_mode.is_some())
        || (completed_count > 5 && document.city_schedule_mode.is_none())
        || (completed_count < 7
            && (document.privacy_confirmations.explicit_sound
                || document.privacy_confirmations.raw_conversation_retention))
        || (completed_count > 6
            && (!document.privacy_confirmations.explicit_sound
                || !document.privacy_confirmations.raw_conversation_retention))
    {
        return Err(integrity_error());
    }
    Ok(())
}

fn validate_profile(profile: &StoredOnboardingProfile) -> Result<(), StorageError> {
    if profile.display_name.chars().count() > 80
        || profile.companion_style != "quiet_warm"
        || profile.initial_preferences.len() > 20
        || profile
            .initial_preferences
            .iter()
            .any(|value| value.is_empty() || value.chars().count() > 100)
        || !matches!(
            profile.narration_density.as_str(),
            "quiet" | "balanced" | "frequent"
        )
    {
        return Err(integrity_error());
    }
    Ok(())
}

fn valid_sources(sources: &[String]) -> bool {
    !sources.iter().any(|source| {
        !matches!(source.as_str(), "local" | "apple_music")
            || sources
                .iter()
                .filter(|candidate| *candidate == source)
                .count()
                != 1
    })
}

fn optional_allowed(value: Option<&str>, allowed: &[&str]) -> bool {
    value.is_none_or(|candidate| allowed.contains(&candidate))
}

fn parse_object(encoded: &str) -> Result<Map<String, Value>, StorageError> {
    match serde_json::from_str(encoded).map_err(|_| integrity_error())? {
        Value::Object(value) => Ok(value),
        _ => Err(integrity_error()),
    }
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, StorageError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(integrity_error()),
    }
}

fn optional_string_array(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, StorageError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| integrity_error())
}

const fn read_error() -> StorageError {
    StorageError::new(StorageReason::StorageReadFailed)
}

const fn write_error() -> StorageError {
    StorageError::new(StorageReason::StorageWriteFailed)
}

const fn integrity_error() -> StorageError {
    StorageError::new(StorageReason::StorageIntegrityFailed)
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn valid_document(completed_count: usize) -> StoredOnboardingDocument {
        let mut document = StoredOnboardingDocument {
            completed_steps: STEP_ORDER[..completed_count]
                .iter()
                .map(|step| (*step).to_owned())
                .collect(),
            revision: completed_count as u64,
            ..StoredOnboardingDocument::default()
        };
        if completed_count >= 2 {
            document.source_selection = vec!["local".to_owned()];
        }
        if completed_count >= 3 {
            document.ai_mode = Some("local_only".to_owned());
        }
        if completed_count >= 4 {
            document.voice_mode = Some("text_only".to_owned());
        }
        if completed_count >= 6 {
            document.city_schedule_mode = Some("not_now".to_owned());
        }
        if completed_count >= 7 {
            document.privacy_confirmations.explicit_sound = true;
            document.privacy_confirmations.raw_conversation_retention = true;
            document.completed = true;
        }
        document
    }

    #[test]
    fn document_requires_exact_prefix_coherence_for_future_fields() {
        for completed_count in 0..=STEP_ORDER.len() {
            assert!(validate_document(&valid_document(completed_count)).is_ok());
        }

        let mut premature_source = valid_document(1);
        premature_source.source_selection = vec!["local".to_owned()];
        let mut premature_ai = valid_document(2);
        premature_ai.ai_mode = Some("verified".to_owned());
        let mut premature_voice = valid_document(3);
        premature_voice.voice_mode = Some("selected".to_owned());
        let mut premature_city = valid_document(5);
        premature_city.city_schedule_mode = Some("configured".to_owned());
        let mut premature_privacy = valid_document(6);
        premature_privacy.privacy_confirmations.explicit_sound = true;

        for invalid in [
            premature_source,
            premature_ai,
            premature_voice,
            premature_city,
            premature_privacy,
        ] {
            assert_eq!(
                validate_document(&invalid)
                    .expect_err("future data must fail closed")
                    .reason(),
                StorageReason::StorageIntegrityFailed
            );
        }
    }
}
