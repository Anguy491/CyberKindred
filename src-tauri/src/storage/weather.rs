use super::{Repository, StorageError, StorageReason, StoredWeatherLocation};

const SETTINGS_SCHEMA_VERSION: i64 = 1;
const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredWeatherCache {
    pub cache_key: String,
    pub location: StoredWeatherLocation,
    pub request_shape_hash: String,
    pub weather_json: String,
    pub fetched_at_ms: i64,
    pub expires_at_ms: i64,
    pub delete_after_ms: i64,
}

impl Repository {
    pub(crate) async fn select_weather_location(
        &self,
        expected_revision: u64,
        location: &StoredWeatherLocation,
        updated_at_ms: i64,
    ) -> Result<u64, StorageError> {
        validate_location(location)?;
        let mut transaction = self.writer.begin().await.map_err(|_| write_error())?;
        let current_encoded: Option<String> = sqlx::query_scalar(
            "SELECT value_json FROM app_settings WHERE key = 'ui.settings_revision'",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| read_error())?;
        let current_revision = current_encoded
            .as_deref()
            .map(serde_json::from_str::<u64>)
            .transpose()
            .map_err(|_| integrity_error())?
            .unwrap_or(0);
        if current_revision != expected_revision {
            return Err(StorageError::new(StorageReason::RevisionConflict));
        }
        let next_revision = current_revision
            .checked_add(1)
            .filter(|value| *value <= MAX_JS_SAFE_INTEGER)
            .ok_or_else(write_error)?;

        sqlx::query(
            "INSERT INTO user_profile(id, locale, city, region, country, country_code, city_lat, city_lon, timezone, routine_json, program_preferences_json, profile_revision, created_at_ms, updated_at_ms) VALUES('current', 'zh-CN', ?, ?, ?, ?, ?, ?, ?, '{}', '{}', 1, ?, ?) ON CONFLICT(id) DO UPDATE SET city = excluded.city, region = excluded.region, country = excluded.country, country_code = excluded.country_code, city_lat = excluded.city_lat, city_lon = excluded.city_lon, timezone = excluded.timezone, profile_revision = user_profile.profile_revision + 1, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&location.city)
        .bind(&location.region)
        .bind(&location.country)
        .bind(&location.country_code)
        .bind(location.latitude)
        .bind(location.longitude)
        .bind(&location.timezone)
        .bind(updated_at_ms)
        .bind(updated_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        let encoded = serde_json::to_string(&next_revision).map_err(|_| write_error())?;
        sqlx::query(
            "INSERT INTO app_settings(key, value_json, schema_version, updated_at_ms) VALUES('ui.settings_revision', ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, schema_version = excluded.schema_version, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(encoded)
        .bind(SETTINGS_SCHEMA_VERSION)
        .bind(updated_at_ms)
        .execute(&mut *transaction)
        .await
        .map_err(|_| write_error())?;
        transaction.commit().await.map_err(|_| write_error())?;
        Ok(next_revision)
    }

    pub(crate) async fn load_weather_cache(
        &self,
        cache_key: &str,
    ) -> Result<Option<StoredWeatherCache>, StorageError> {
        type Row = (
            String,
            String,
            Option<String>,
            String,
            String,
            f64,
            f64,
            String,
            String,
            String,
            i64,
            i64,
            i64,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT cache_key, city, region, country, country_code, latitude, longitude, timezone, request_shape_hash, weather_json, fetched_at_ms, expires_at_ms, delete_after_ms FROM weather_cache WHERE cache_key = ?",
        )
        .bind(cache_key)
        .fetch_optional(&self.writer)
        .await
        .map_err(|_| read_error())?;
        row.map(|row| {
            let cache = StoredWeatherCache {
                cache_key: row.0,
                location: StoredWeatherLocation {
                    city: row.1,
                    region: row.2,
                    country: row.3,
                    country_code: row.4,
                    latitude: row.5,
                    longitude: row.6,
                    timezone: row.7,
                },
                request_shape_hash: row.8,
                weather_json: row.9,
                fetched_at_ms: row.10,
                expires_at_ms: row.11,
                delete_after_ms: row.12,
            };
            validate_cache(&cache)?;
            Ok(cache)
        })
        .transpose()
    }

    pub(crate) async fn save_weather_cache(
        &self,
        cache: &StoredWeatherCache,
    ) -> Result<(), StorageError> {
        validate_cache(cache)?;
        sqlx::query(
            "INSERT INTO weather_cache(cache_key, city, region, country, country_code, latitude, longitude, timezone, request_shape_hash, weather_json, fetched_at_ms, expires_at_ms, delete_after_ms) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(cache_key) DO UPDATE SET city = excluded.city, region = excluded.region, country = excluded.country, country_code = excluded.country_code, latitude = excluded.latitude, longitude = excluded.longitude, timezone = excluded.timezone, request_shape_hash = excluded.request_shape_hash, weather_json = excluded.weather_json, fetched_at_ms = excluded.fetched_at_ms, expires_at_ms = excluded.expires_at_ms, delete_after_ms = excluded.delete_after_ms",
        )
        .bind(&cache.cache_key)
        .bind(&cache.location.city)
        .bind(&cache.location.region)
        .bind(&cache.location.country)
        .bind(&cache.location.country_code)
        .bind(cache.location.latitude)
        .bind(cache.location.longitude)
        .bind(&cache.location.timezone)
        .bind(&cache.request_shape_hash)
        .bind(&cache.weather_json)
        .bind(cache.fetched_at_ms)
        .bind(cache.expires_at_ms)
        .bind(cache.delete_after_ms)
        .execute(&self.writer)
        .await
        .map_err(|_| write_error())?;
        Ok(())
    }
}

fn validate_location(location: &StoredWeatherLocation) -> Result<(), StorageError> {
    if location.city.is_empty()
        || location.city.chars().count() > 100
        || location.country.is_empty()
        || location.country.chars().count() > 100
        || location.country_code.len() != 2
        || !location
            .country_code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
        || !location.latitude.is_finite()
        || !(-90.0..=90.0).contains(&location.latitude)
        || !location.longitude.is_finite()
        || !(-180.0..=180.0).contains(&location.longitude)
        || location.timezone.is_empty()
        || location.timezone.len() > 100
        || location
            .region
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.chars().count() > 100)
    {
        return Err(integrity_error());
    }
    Ok(())
}

fn validate_cache(cache: &StoredWeatherCache) -> Result<(), StorageError> {
    validate_location(&cache.location)?;
    if cache.cache_key.len() != 64
        || cache.request_shape_hash.len() != 64
        || !cache.cache_key.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !cache
            .request_shape_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || cache.fetched_at_ms < 0
        || cache.expires_at_ms < cache.fetched_at_ms
        || cache.delete_after_ms < cache.expires_at_ms
        || serde_json::from_str::<serde_json::Value>(&cache.weather_json).is_err()
    {
        return Err(integrity_error());
    }
    Ok(())
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
