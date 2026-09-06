//! Explicit city search and bounded current-weather context.

use crate::{
    ipc::{ApiError, InternalReason, PublicField, RequestHash, canonical_request_hash},
    llm::{ProgramContextExtras, ProgramContextSource},
    program::{ProgramFuture, ProgramProviderError, ProviderProgramInput},
    providers::{Clock, ProviderFailure, ProviderFailureCategory, WeatherLocation},
    providers::{ProviderCallContext, ProviderHealthProbe, ProviderTestInput},
    storage::{
        NewProviderOutcome, ProviderOutcomeStatus, ProviderRequestKind, ProviderUsageProvider,
        Repository, StorageError, StorageReason, StoredWeatherCache, StoredWeatherLocation,
    },
};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock, RwLockWriteGuard};
use uuid::Uuid;

const GEOCODING_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
const CURRENT_VARIABLES: &str =
    "temperature_2m,apparent_temperature,precipitation,weather_code,is_day";
const TEMPERATURE_UNIT: &str = "celsius";
const PRECIPITATION_UNIT: &str = "mm";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
const FORECAST_TIMEOUT: Duration = Duration::from_secs(10);
const CANDIDATE_TTL_MS: i64 = 10 * 60 * 1_000;
const WEATHER_FRESH_MS: i64 = 30 * 60 * 1_000;
const WEATHER_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_RESPONSE_BYTES: usize = 512 * 1_024;
const IDEMPOTENCY_CAPACITY: usize = 256;

pub type WeatherFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherLocationCandidate {
    pub candidate_id: Uuid,
    pub city: String,
    pub region: Option<String>,
    pub country: String,
    pub country_code: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchWeatherLocationsRequest {
    pub client_request_id: Uuid,
    pub query: String,
    pub limit: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchWeatherLocationsResponse {
    pub request_id: Uuid,
    pub candidates: Vec<WeatherLocationCandidate>,
    pub expires_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectWeatherLocationRequest {
    pub client_request_id: Uuid,
    pub candidate_id: Uuid,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectWeatherLocationResponse {
    pub request_id: Uuid,
    pub location: WeatherLocation,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CurrentWeather {
    observed_at: String,
    temperature_c: f64,
    apparent_temperature_c: f64,
    precipitation_mm: f64,
    weather_code: u16,
    is_day: bool,
}

#[derive(Clone)]
pub(crate) struct UnsignedCandidate {
    city: String,
    region: Option<String>,
    country: String,
    country_code: String,
    latitude: f64,
    longitude: f64,
    timezone: String,
}

pub(crate) trait WeatherProvider: Send + Sync {
    fn search<'a>(
        &'a self,
        query: &'a str,
        limit: u8,
    ) -> WeatherFuture<'a, Result<(Vec<UnsignedCandidate>, u64), ProviderFailure>>;

    fn current<'a>(
        &'a self,
        location: &'a WeatherLocation,
    ) -> WeatherFuture<'a, Result<(CurrentWeather, u64), ProviderFailure>>;
}

pub(crate) struct OpenMeteoWeatherProvider {
    client: reqwest::Client,
}

impl OpenMeteoWeatherProvider {
    pub(crate) fn new() -> Result<Self, ProviderFailure> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(SEARCH_TIMEOUT)
            .user_agent(concat!("CyberKindred/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
        Ok(Self { client })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: url::Url,
        timeout: Duration,
    ) -> Result<(T, u64), ProviderFailure> {
        let started_at = Instant::now();
        let mut response = self
            .client
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|error| map_reqwest_error(&error))?;
        let status = response.status();
        if status.as_u16() == 429 {
            return Err(ProviderFailure::rate_limited(None));
        }
        if !status.is_success() {
            return Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| map_reqwest_error(&error))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(ProviderFailure::new(
                    ProviderFailureCategory::InvalidResponse,
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let parsed = serde_json::from_slice(&body)
            .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
        Ok((parsed, elapsed_millis(started_at.elapsed())))
    }
}

impl WeatherProvider for OpenMeteoWeatherProvider {
    fn search<'a>(
        &'a self,
        query: &'a str,
        limit: u8,
    ) -> WeatherFuture<'a, Result<(Vec<UnsignedCandidate>, u64), ProviderFailure>> {
        Box::pin(async move {
            let url = geocoding_url(query, limit)?;
            let (response, latency) = self
                .get_json::<GeocodingResponse>(url, SEARCH_TIMEOUT)
                .await?;
            let candidates = response
                .results
                .unwrap_or_default()
                .into_iter()
                .take(usize::from(limit))
                .map(validate_geocoding_result)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((candidates, latency))
        })
    }

    fn current<'a>(
        &'a self,
        location: &'a WeatherLocation,
    ) -> WeatherFuture<'a, Result<(CurrentWeather, u64), ProviderFailure>> {
        Box::pin(async move {
            let url = forecast_url(location)?;
            let (response, latency) = self
                .get_json::<ForecastResponse>(url, FORECAST_TIMEOUT)
                .await?;
            let current = validate_current_weather(&response, &location.timezone)?;
            Ok((current, latency))
        })
    }
}

#[derive(Deserialize)]
struct GeocodingResponse {
    results: Option<Vec<GeocodingResult>>,
}

#[derive(Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    country_code: String,
    timezone: String,
    country: String,
    admin1: Option<String>,
}

#[derive(Deserialize)]
struct ForecastResponse {
    timezone: String,
    current: ForecastCurrent,
}

#[derive(Deserialize)]
struct ForecastCurrent {
    time: String,
    temperature_2m: f64,
    apparent_temperature: f64,
    precipitation: f64,
    weather_code: u16,
    is_day: u8,
}

fn geocoding_url(query: &str, limit: u8) -> Result<url::Url, ProviderFailure> {
    let mut url = url::Url::parse(GEOCODING_ENDPOINT)
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
    url.query_pairs_mut()
        .append_pair("name", query)
        .append_pair("count", &limit.to_string())
        .append_pair("language", "zh")
        .append_pair("format", "json");
    Ok(url)
}

fn forecast_url(location: &WeatherLocation) -> Result<url::Url, ProviderFailure> {
    validate_location(location)?;
    let mut url = url::Url::parse(FORECAST_ENDPOINT)
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
    url.query_pairs_mut()
        .append_pair("latitude", &format!("{:.4}", location.latitude))
        .append_pair("longitude", &format!("{:.4}", location.longitude))
        .append_pair("timezone", &location.timezone)
        .append_pair("current", CURRENT_VARIABLES)
        .append_pair("temperature_unit", TEMPERATURE_UNIT)
        .append_pair("precipitation_unit", PRECIPITATION_UNIT);
    Ok(url)
}

fn validate_geocoding_result(
    result: GeocodingResult,
) -> Result<UnsignedCandidate, ProviderFailure> {
    let candidate = UnsignedCandidate {
        city: result.name,
        region: result.admin1,
        country: result.country,
        country_code: result.country_code.to_ascii_uppercase(),
        latitude: result.latitude,
        longitude: result.longitude,
        timezone: result.timezone,
    };
    validate_unsigned_candidate(&candidate)?;
    Ok(candidate)
}

fn validate_current_weather(
    response: &ForecastResponse,
    expected_timezone: &str,
) -> Result<CurrentWeather, ProviderFailure> {
    if response.timezone != expected_timezone
        || !response.current.temperature_2m.is_finite()
        || !response.current.apparent_temperature.is_finite()
        || !response.current.precipitation.is_finite()
        || response.current.precipitation < 0.0
        || response.current.is_day > 1
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    let timezone = expected_timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    let local = chrono::NaiveDateTime::parse_from_str(&response.current.time, "%Y-%m-%dT%H:%M")
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    let observed = timezone
        .from_local_datetime(&local)
        .earliest()
        .ok_or_else(|| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?
        .with_timezone(&Utc);
    Ok(CurrentWeather {
        observed_at: observed.to_rfc3339_opts(SecondsFormat::Millis, true),
        temperature_c: response.current.temperature_2m,
        apparent_temperature_c: response.current.apparent_temperature,
        precipitation_mm: response.current.precipitation,
        weather_code: response.current.weather_code,
        is_day: response.current.is_day == 1,
    })
}

#[derive(Clone)]
struct CandidateRecord {
    candidate: WeatherLocationCandidate,
    expires_at_ms: i64,
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
            entries.retain(|_, value| value.expires_at > now);
            if let Some(value) = entries.get(&client_request_id) {
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
                    expires_at: now + Duration::from_mins(10),
                    result: Mutex::new(None),
                });
                entries.insert(client_request_id, Arc::clone(&value));
                value
            }
        };
        let mut cached = entry.result.lock().await;
        if let Some(result) = cached.as_ref() {
            return result.clone();
        }
        let result = operation().await;
        *cached = Some(result.clone());
        result
    }
}

pub(crate) struct WeatherService {
    repository: Repository,
    provider: Arc<dyn WeatherProvider>,
    clock: Arc<dyn Clock>,
    candidates: Mutex<HashMap<Uuid, CandidateRecord>>,
    lifecycle: RwLock<()>,
    accepting: AtomicBool,
    search_requests: AsyncIdempotency<SearchWeatherLocationsResponse>,
    select_requests: AsyncIdempotency<SelectWeatherLocationResponse>,
}

pub(crate) struct CompositeProviderHealthProbe {
    primary: Arc<dyn ProviderHealthProbe>,
    weather: Arc<WeatherService>,
}

impl CompositeProviderHealthProbe {
    pub(crate) fn new(primary: Arc<dyn ProviderHealthProbe>, weather: Arc<WeatherService>) -> Self {
        Self { primary, weather }
    }
}

impl ProviderHealthProbe for CompositeProviderHealthProbe {
    fn test<'a>(
        &'a self,
        input: ProviderTestInput<'a>,
        context: &'a ProviderCallContext,
    ) -> crate::providers::ProviderFuture<'a, Result<u64, ProviderFailure>> {
        match input {
            ProviderTestInput::Weather => self.weather.test(ProviderTestInput::Weather, context),
            other => self.primary.test(other, context),
        }
    }
}

impl WeatherService {
    pub(crate) fn new(
        repository: Repository,
        provider: Arc<dyn WeatherProvider>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            provider,
            clock,
            candidates: Mutex::new(HashMap::new()),
            lifecycle: RwLock::new(()),
            accepting: AtomicBool::new(true),
            search_requests: AsyncIdempotency::new(),
            select_requests: AsyncIdempotency::new(),
        }
    }

    pub async fn search_locations(
        &self,
        request: SearchWeatherLocationsRequest,
    ) -> Result<SearchWeatherLocationsResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.search_requests
            .execute(request_id, hash, || self.search_locations_once(request))
            .await
    }

    async fn search_locations_once(
        &self,
        request: SearchWeatherLocationsRequest,
    ) -> Result<SearchWeatherLocationsResponse, ApiError> {
        let _lifecycle = self.lifecycle.read().await;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        validate_search(&request)?;
        let started_at = Instant::now();
        let result = self.provider.search(&request.query, request.limit).await;
        self.record_outcome(
            ProviderRequestKind::LocationSearch,
            elapsed_millis(started_at.elapsed()),
            outcome_status(&result),
            request.client_request_id,
        )
        .await?;
        let (unsigned, _) = result.map_err(|error| error.into_api_error(false))?;
        let expires_at_ms = self.clock.now_ms().saturating_add(CANDIDATE_TTL_MS);
        let mut candidates = self.candidates.lock().await;
        candidates.retain(|_, value| value.expires_at_ms > self.clock.now_ms());
        candidates.clear();
        let signed = unsigned
            .into_iter()
            .map(|candidate| {
                let candidate = WeatherLocationCandidate {
                    candidate_id: Uuid::now_v7(),
                    city: candidate.city,
                    region: candidate.region,
                    country: candidate.country,
                    country_code: candidate.country_code,
                    latitude: candidate.latitude,
                    longitude: candidate.longitude,
                    timezone: candidate.timezone,
                };
                candidates.insert(
                    candidate.candidate_id,
                    CandidateRecord {
                        candidate: candidate.clone(),
                        expires_at_ms,
                    },
                );
                candidate
            })
            .collect();
        Ok(SearchWeatherLocationsResponse {
            request_id: request.client_request_id,
            candidates: signed,
            expires_at: timestamp(expires_at_ms)?,
        })
    }

    pub async fn select_location(
        &self,
        request: SelectWeatherLocationRequest,
    ) -> Result<SelectWeatherLocationResponse, ApiError> {
        let hash = canonical_request_hash(&request)?;
        let request_id = request.client_request_id;
        self.select_requests
            .execute(request_id, hash, || self.select_location_once(request))
            .await
    }

    async fn select_location_once(
        &self,
        request: SelectWeatherLocationRequest,
    ) -> Result<SelectWeatherLocationResponse, ApiError> {
        let _lifecycle = self.lifecycle.read().await;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(ApiError::from_reason(InternalReason::ResourceBusy));
        }
        let candidate = {
            let mut candidates = self.candidates.lock().await;
            candidates.retain(|_, value| value.expires_at_ms > self.clock.now_ms());
            candidates
                .get(&request.candidate_id)
                .cloned()
                .ok_or_else(|| ApiError::from_reason(InternalReason::InvalidCandidate))?
        };
        let location = WeatherLocation {
            city: candidate.candidate.city,
            region: candidate.candidate.region,
            country: candidate.candidate.country,
            country_code: candidate.candidate.country_code,
            latitude: candidate.candidate.latitude,
            longitude: candidate.candidate.longitude,
            timezone: candidate.candidate.timezone,
        };
        let revision = self
            .repository
            .select_weather_location(
                request.expected_revision,
                &stored_location(&location),
                self.clock.now_ms(),
            )
            .await
            .map_err(|error| map_storage_error(&error))?;
        Ok(SelectWeatherLocationResponse {
            request_id: request.client_request_id,
            location,
            revision,
        })
    }

    async fn weather_summary(&self) -> Option<String> {
        let _lifecycle = self.lifecycle.read().await;
        if !self.accepting.load(Ordering::Acquire) {
            return None;
        }
        let settings = self.repository.load_provider_settings().await.ok()?;
        if !settings.weather_enabled {
            return None;
        }
        let location = settings.weather_location.map(weather_location)?;
        let now_ms = self.clock.now_ms();
        let key = weather_cache_key(&location);
        if let Ok(Some(cache)) = self.repository.load_weather_cache(&key).await
            && cache.expires_at_ms > now_ms
            && let Ok(weather) = serde_json::from_str::<CurrentWeather>(&cache.weather_json)
            && validate_cached_weather(&weather)
        {
            return Some(format_weather_summary(&location.city, &weather));
        }

        let started_at = Instant::now();
        let result = self.provider.current(&location).await;
        let _ = self
            .record_outcome(
                ProviderRequestKind::CurrentWeather,
                elapsed_millis(started_at.elapsed()),
                outcome_status(&result),
                Uuid::now_v7(),
            )
            .await;
        let (weather, _) = result.ok()?;
        let weather_json = serde_json::to_string(&weather).ok()?;
        let fetched_at_ms = now_ms;
        let cache = StoredWeatherCache {
            cache_key: key,
            location: stored_location(&location),
            request_shape_hash: request_shape_hash(),
            weather_json,
            fetched_at_ms,
            expires_at_ms: fetched_at_ms.saturating_add(WEATHER_FRESH_MS),
            delete_after_ms: fetched_at_ms.saturating_add(WEATHER_RETENTION_MS),
        };
        self.repository.save_weather_cache(&cache).await.ok()?;
        Some(format_weather_summary(&location.city, &weather))
    }

    pub(crate) async fn lock_private_lifecycle(&self) -> RwLockWriteGuard<'_, ()> {
        self.lifecycle.write().await
    }

    pub(crate) async fn clear_private_candidates(&self) {
        self.candidates.lock().await.clear();
    }

    pub(crate) fn begin_reset(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    async fn record_outcome(
        &self,
        request_kind: ProviderRequestKind,
        latency_ms: u64,
        status: ProviderOutcomeStatus,
        correlation_id: Uuid,
    ) -> Result<(), ApiError> {
        let outcome = NewProviderOutcome::new(
            ProviderUsageProvider::OpenMeteo,
            request_kind,
            None,
            latency_ms,
            status,
            correlation_id,
            self.clock.now_ms(),
        )
        .map_err(|error| map_storage_error(&error))?;
        self.repository
            .record_provider_outcome(outcome)
            .await
            .map(|_| ())
            .map_err(|error| map_storage_error(&error))
    }
}

impl ProgramContextSource for WeatherService {
    fn load<'a>(
        &'a self,
        _input: &'a ProviderProgramInput,
    ) -> ProgramFuture<'a, Result<ProgramContextExtras, ProgramProviderError>> {
        Box::pin(async move {
            Ok(ProgramContextExtras {
                weather_summary: self.weather_summary().await,
                ..ProgramContextExtras::default()
            })
        })
    }
}

impl ProviderHealthProbe for WeatherService {
    fn test<'a>(
        &'a self,
        input: ProviderTestInput<'a>,
        _context: &'a ProviderCallContext,
    ) -> crate::providers::ProviderFuture<'a, Result<u64, ProviderFailure>> {
        Box::pin(async move {
            if !matches!(input, ProviderTestInput::Weather) {
                return Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
            }
            let settings = self
                .repository
                .load_provider_settings()
                .await
                .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
            if !settings.weather_enabled {
                return Err(ProviderFailure::new(ProviderFailureCategory::Unavailable));
            }
            let location = settings
                .weather_location
                .map(weather_location)
                .ok_or_else(|| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
            self.provider
                .current(&location)
                .await
                .map(|(_, latency)| latency)
        })
    }
}

pub(crate) mod commands {
    use super::{
        SearchWeatherLocationsRequest, SearchWeatherLocationsResponse,
        SelectWeatherLocationRequest, SelectWeatherLocationResponse, WeatherService,
    };
    use crate::ipc::{ApiError, parse_command_request};
    use std::sync::Arc;
    use tauri::State;

    #[tauri::command]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn api_v1_search_weather_locations(
        request: tauri::ipc::Request<'_>,
        service: State<'_, Arc<WeatherService>>,
    ) -> Result<SearchWeatherLocationsResponse, ApiError> {
        service
            .search_locations(parse_command_request::<SearchWeatherLocationsRequest>(
                &request,
            )?)
            .await
    }

    #[tauri::command]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn api_v1_select_weather_location(
        request: tauri::ipc::Request<'_>,
        service: State<'_, Arc<WeatherService>>,
    ) -> Result<SelectWeatherLocationResponse, ApiError> {
        service
            .select_location(parse_command_request::<SelectWeatherLocationRequest>(
                &request,
            )?)
            .await
    }
}

fn validate_search(request: &SearchWeatherLocationsRequest) -> Result<(), ApiError> {
    let count = request.query.chars().count();
    if !(2..=100).contains(&count)
        || request.query.trim() != request.query
        || request.query.chars().any(char::is_control)
        || !(1..=10).contains(&request.limit)
    {
        return Err(ApiError::from_reason(InternalReason::RequestInvalid));
    }
    Ok(())
}

fn validate_unsigned_candidate(candidate: &UnsignedCandidate) -> Result<(), ProviderFailure> {
    let location = WeatherLocation {
        city: candidate.city.clone(),
        region: candidate.region.clone(),
        country: candidate.country.clone(),
        country_code: candidate.country_code.clone(),
        latitude: candidate.latitude,
        longitude: candidate.longitude,
        timezone: candidate.timezone.clone(),
    };
    validate_location(&location)
}

fn validate_location(location: &WeatherLocation) -> Result<(), ProviderFailure> {
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
        || location
            .region
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.chars().count() > 100)
        || location.timezone.parse::<chrono_tz::Tz>().is_err()
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    Ok(())
}

fn validate_cached_weather(weather: &CurrentWeather) -> bool {
    !weather.observed_at.is_empty()
        && chrono::DateTime::parse_from_rfc3339(&weather.observed_at).is_ok()
        && weather.temperature_c.is_finite()
        && weather.apparent_temperature_c.is_finite()
        && weather.precipitation_mm.is_finite()
        && weather.precipitation_mm >= 0.0
}

fn format_weather_summary(city: &str, weather: &CurrentWeather) -> String {
    format!(
        "{city}当前天气代码 {}，气温 {:.1}°C，体感 {:.1}°C，降水 {:.1} mm；观测时间 {}。",
        weather.weather_code,
        weather.temperature_c,
        weather.apparent_temperature_c,
        weather.precipitation_mm,
        weather.observed_at
    )
}

fn weather_cache_key(location: &WeatherLocation) -> String {
    let source = format!(
        "{:.4}|{:.4}|{}|{}",
        location.latitude,
        location.longitude,
        location.timezone,
        request_shape_hash()
    );
    hex::encode(Sha256::digest(source.as_bytes()))
}

fn request_shape_hash() -> String {
    let source = format!(
        "current={CURRENT_VARIABLES}&temperature_unit={TEMPERATURE_UNIT}&precipitation_unit={PRECIPITATION_UNIT}"
    );
    hex::encode(Sha256::digest(source.as_bytes()))
}

fn weather_location(stored: StoredWeatherLocation) -> WeatherLocation {
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

fn stored_location(location: &WeatherLocation) -> StoredWeatherLocation {
    StoredWeatherLocation {
        city: location.city.clone(),
        region: location.region.clone(),
        country: location.country.clone(),
        country_code: location.country_code.clone(),
        latitude: location.latitude,
        longitude: location.longitude,
        timezone: location.timezone.clone(),
    }
}

fn outcome_status<T>(result: &Result<T, ProviderFailure>) -> ProviderOutcomeStatus {
    match result {
        Ok(_) => ProviderOutcomeStatus::Success,
        Err(error) => match error.category {
            ProviderFailureCategory::Authentication => ProviderOutcomeStatus::Authentication,
            ProviderFailureCategory::RateLimit => ProviderOutcomeStatus::RateLimit,
            ProviderFailureCategory::Timeout => ProviderOutcomeStatus::Timeout,
            ProviderFailureCategory::Unavailable => ProviderOutcomeStatus::Unavailable,
            ProviderFailureCategory::InvalidResponse => ProviderOutcomeStatus::InvalidResponse,
        },
    }
}

fn map_reqwest_error(error: &reqwest::Error) -> ProviderFailure {
    if error.is_timeout() {
        ProviderFailure::new(ProviderFailureCategory::Timeout)
    } else {
        ProviderFailure::new(ProviderFailureCategory::Unavailable)
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

fn timestamp(timestamp_ms: i64) -> Result<String, ApiError> {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| ApiError::from_reason(InternalReason::UnexpectedInternal))
}

fn elapsed_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
