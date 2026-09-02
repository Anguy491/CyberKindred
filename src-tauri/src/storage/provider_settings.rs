use super::{CanonicalOrigin, Repository, StorageError, StorageReason};

const SETTINGS_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredWeatherLocation {
    pub city: String,
    pub region: Option<String>,
    pub country: String,
    pub country_code: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct StoredProviderSettings {
    pub provider_origin: String,
    pub llm_model_id: String,
    pub tts_model_id: String,
    pub tts_voice_id: String,
    pub metadata_enabled: bool,
    pub weather_enabled: bool,
    pub default_source_id: Option<String>,
    pub narration_density: String,
    pub tts_enabled: bool,
    pub audio_output_device_id: Option<String>,
    pub audio_output_behavior: String,
    pub minimize_to_tray: bool,
    pub launch_at_startup: bool,
    pub notifications_enabled: bool,
    pub weather_location: Option<StoredWeatherLocation>,
    pub revision: u64,
}

impl Default for StoredProviderSettings {
    fn default() -> Self {
        Self {
            provider_origin: "https://api.openai.com".to_owned(),
            llm_model_id: "gpt-5.6-luna".to_owned(),
            tts_model_id: "gpt-4o-mini-tts".to_owned(),
            tts_voice_id: "alloy".to_owned(),
            metadata_enabled: false,
            weather_enabled: false,
            default_source_id: None,
            narration_density: "balanced".to_owned(),
            tts_enabled: false,
            audio_output_device_id: None,
            audio_output_behavior: "follow_system_default".to_owned(),
            minimize_to_tray: false,
            launch_at_startup: false,
            notifications_enabled: false,
            weather_location: None,
            revision: 0,
        }
    }
}

impl Repository {
    pub(crate) async fn load_provider_settings(
        &self,
    ) -> Result<StoredProviderSettings, StorageError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT key, value_json FROM app_settings WHERE key IN ('provider.openai.base_url', 'provider.openai.model', 'provider.openai.tts_model', 'provider.openai.tts_voice', 'provider.openai.tts_enabled', 'ui.metadata_enabled', 'ui.weather_enabled', 'audio.default_source_id', 'program.narration_density', 'audio.output_device_id', 'audio.output_behavior', 'os.tray', 'os.autostart', 'os.notifications', 'ui.settings_revision')",
        )
        .fetch_all(&self.writer)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        let mut settings = StoredProviderSettings::default();
        for (key, encoded) in rows {
            let value: serde_json::Value = serde_json::from_str(&encoded)
                .map_err(|_| StorageError::new(StorageReason::StorageIntegrityFailed))?;
            apply_stored_value(&mut settings, &key, &value)?;
        }
        settings.weather_location = load_weather_location(&self.writer).await?;
        validate_stored_settings(&settings)?;
        Ok(settings)
    }

    pub(crate) async fn save_provider_settings(
        &self,
        expected_revision: u64,
        settings: &StoredProviderSettings,
        updated_at_ms: i64,
        clear_weather_location: bool,
    ) -> Result<u64, StorageError> {
        validate_stored_settings(settings)?;
        let mut transaction = self
            .writer
            .begin()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        let current_encoded: Option<String> = sqlx::query_scalar(
            "SELECT value_json FROM app_settings WHERE key = 'ui.settings_revision'",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
        let current_revision = current_encoded
            .as_deref()
            .map(serde_json::from_str::<u64>)
            .transpose()
            .map_err(|_| StorageError::new(StorageReason::StorageIntegrityFailed))?;
        // Keep runtime persistence code free of panic-associated `unwrap*` calls.
        #[allow(clippy::manual_unwrap_or, clippy::manual_unwrap_or_default)]
        let current_revision = match current_revision {
            Some(value) => value,
            None => 0,
        };
        if current_revision != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let next_revision = current_revision
            .checked_add(1)
            .ok_or_else(|| StorageError::new(StorageReason::StorageWriteFailed))?;
        let values = stored_values(settings, next_revision);
        for (key, value) in values {
            let encoded = serde_json::to_string(&value)
                .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
            sqlx::query(
                "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES(?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
            )
            .bind(key)
            .bind(encoded)
            .bind(SETTINGS_SCHEMA_VERSION)
            .bind(updated_at_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        }
        if clear_weather_location {
            sqlx::query(
                "UPDATE user_profile SET city = NULL, region = NULL, country = NULL, country_code = NULL, city_lat = NULL, city_lon = NULL, updated_at_ms = ? WHERE id = 'current'",
            )
            .bind(updated_at_ms)
            .execute(&mut *transaction)
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        }
        transaction
            .commit()
            .await
            .map_err(|_| StorageError::new(StorageReason::StorageWriteFailed))?;
        Ok(next_revision)
    }
}

fn stored_values(
    settings: &StoredProviderSettings,
    next_revision: u64,
) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (
            "provider.openai.base_url",
            serde_json::json!(settings.provider_origin),
        ),
        (
            "provider.openai.model",
            serde_json::json!(settings.llm_model_id),
        ),
        (
            "provider.openai.tts_model",
            serde_json::json!(settings.tts_model_id),
        ),
        (
            "provider.openai.tts_voice",
            serde_json::json!(settings.tts_voice_id),
        ),
        (
            "provider.openai.tts_enabled",
            serde_json::json!(settings.tts_enabled),
        ),
        (
            "ui.metadata_enabled",
            serde_json::json!(settings.metadata_enabled),
        ),
        (
            "ui.weather_enabled",
            serde_json::json!(settings.weather_enabled),
        ),
        (
            "audio.default_source_id",
            serde_json::json!(settings.default_source_id),
        ),
        (
            "program.narration_density",
            serde_json::json!(settings.narration_density),
        ),
        (
            "audio.output_device_id",
            serde_json::json!(settings.audio_output_device_id),
        ),
        (
            "audio.output_behavior",
            serde_json::json!(settings.audio_output_behavior),
        ),
        ("os.tray", serde_json::json!(settings.minimize_to_tray)),
        (
            "os.autostart",
            serde_json::json!(settings.launch_at_startup),
        ),
        (
            "os.notifications",
            serde_json::json!(settings.notifications_enabled),
        ),
        ("ui.settings_revision", serde_json::json!(next_revision)),
    ]
}

fn apply_stored_value(
    settings: &mut StoredProviderSettings,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), StorageError> {
    match key {
        "provider.openai.base_url" => settings.provider_origin = string_value(value)?,
        "provider.openai.model" => settings.llm_model_id = string_value(value)?,
        "provider.openai.tts_model" => settings.tts_model_id = string_value(value)?,
        "provider.openai.tts_voice" => settings.tts_voice_id = string_value(value)?,
        "provider.openai.tts_enabled" => settings.tts_enabled = bool_value(value)?,
        "ui.metadata_enabled" => settings.metadata_enabled = bool_value(value)?,
        "ui.weather_enabled" => settings.weather_enabled = bool_value(value)?,
        "audio.default_source_id" => settings.default_source_id = optional_string(value)?,
        "program.narration_density" => settings.narration_density = string_value(value)?,
        "audio.output_device_id" => settings.audio_output_device_id = optional_string(value)?,
        "audio.output_behavior" => settings.audio_output_behavior = string_value(value)?,
        "os.tray" => settings.minimize_to_tray = bool_value(value)?,
        "os.autostart" => settings.launch_at_startup = bool_value(value)?,
        "os.notifications" => settings.notifications_enabled = bool_value(value)?,
        "ui.settings_revision" => {
            settings.revision = value
                .as_u64()
                .ok_or_else(|| StorageError::new(StorageReason::StorageIntegrityFailed))?;
        }
        _ => return Err(StorageError::new(StorageReason::StorageIntegrityFailed)),
    }
    Ok(())
}

fn string_value(value: &serde_json::Value) -> Result<String, StorageError> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| StorageError::new(StorageReason::StorageIntegrityFailed))
}

fn bool_value(value: &serde_json::Value) -> Result<bool, StorageError> {
    value
        .as_bool()
        .ok_or_else(|| StorageError::new(StorageReason::StorageIntegrityFailed))
}

fn optional_string(value: &serde_json::Value) -> Result<Option<String>, StorageError> {
    if value.is_null() {
        Ok(None)
    } else {
        string_value(value).map(Some)
    }
}

async fn load_weather_location(
    pool: &sqlx::SqlitePool,
) -> Result<Option<StoredWeatherLocation>, StorageError> {
    type WeatherRow = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        String,
    );
    let row: Option<WeatherRow> = sqlx::query_as(
        "SELECT city, region, country, country_code, city_lat, city_lon, timezone FROM user_profile WHERE id = 'current'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|_| StorageError::new(StorageReason::StorageReadFailed))?;
    let Some((city, region, country, country_code, latitude, longitude, timezone)) = row else {
        return Ok(None);
    };
    match (city, country, country_code, latitude, longitude) {
        (Some(city), Some(country), Some(country_code), Some(latitude), Some(longitude)) => {
            Ok(Some(StoredWeatherLocation {
                city,
                region,
                country,
                country_code,
                latitude,
                longitude,
                timezone,
            }))
        }
        (None, None, None, None, None) => Ok(None),
        _ => Err(StorageError::new(StorageReason::StorageIntegrityFailed)),
    }
}

fn validate_stored_settings(settings: &StoredProviderSettings) -> Result<(), StorageError> {
    CanonicalOrigin::parse(&settings.provider_origin)
        .map_err(|_| StorageError::new(StorageReason::StorageIntegrityFailed))?;
    for value in [
        &settings.llm_model_id,
        &settings.tts_model_id,
        &settings.tts_voice_id,
    ] {
        if value.is_empty() || value.chars().count() > 100 {
            return Err(StorageError::new(StorageReason::StorageIntegrityFailed));
        }
    }
    if !matches!(
        settings.narration_density.as_str(),
        "quiet" | "balanced" | "frequent"
    ) || !matches!(
        settings.audio_output_behavior.as_str(),
        "follow_system_default" | "fixed_device"
    ) || (settings.audio_output_behavior == "fixed_device"
        && settings.audio_output_device_id.is_none())
    {
        return Err(StorageError::new(StorageReason::StorageIntegrityFailed));
    }
    Ok(())
}
