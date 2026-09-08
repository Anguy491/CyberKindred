use super::*;
use crate::{
    providers::SystemClock,
    storage::{AppPaths, Storage, StoredWeatherCache, StoredWeatherLocation},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

struct FakeProvider {
    searches: AtomicUsize,
    currents: AtomicUsize,
}

impl FakeProvider {
    fn new() -> Self {
        Self {
            searches: AtomicUsize::new(0),
            currents: AtomicUsize::new(0),
        }
    }
}

impl WeatherProvider for FakeProvider {
    fn search<'a>(
        &'a self,
        _query: &'a str,
        _limit: u8,
    ) -> WeatherFuture<'a, Result<(Vec<UnsignedCandidate>, u64), ProviderFailure>> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok((
                vec![UnsignedCandidate {
                    city: "Sydney".to_owned(),
                    region: Some("New South Wales".to_owned()),
                    country: "Australia".to_owned(),
                    country_code: "AU".to_owned(),
                    latitude: -33.8688,
                    longitude: 151.2093,
                    timezone: "Australia/Sydney".to_owned(),
                }],
                7,
            ))
        })
    }

    fn current<'a>(
        &'a self,
        _location: &'a WeatherLocation,
    ) -> WeatherFuture<'a, Result<(CurrentWeather, u64), ProviderFailure>> {
        self.currents.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok((
                CurrentWeather {
                    observed_at: "2026-09-03T02:00:00.000Z".to_owned(),
                    temperature_c: 18.5,
                    apparent_temperature_c: 17.2,
                    precipitation_mm: 0.0,
                    weather_code: 1,
                    is_day: true,
                },
                8,
            ))
        })
    }
}

struct SuspendProvider {
    searches: AtomicUsize,
    started: Notify,
}

impl WeatherProvider for SuspendProvider {
    fn search<'a>(
        &'a self,
        _query: &'a str,
        _limit: u8,
    ) -> WeatherFuture<'a, Result<(Vec<UnsignedCandidate>, u64), ProviderFailure>> {
        let call = self.searches.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        Box::pin(async move {
            if call == 0 {
                std::future::pending().await
            } else {
                Ok((
                    vec![UnsignedCandidate {
                        city: "Sydney".to_owned(),
                        region: Some("New South Wales".to_owned()),
                        country: "Australia".to_owned(),
                        country_code: "AU".to_owned(),
                        latitude: -33.8688,
                        longitude: 151.2093,
                        timezone: "Australia/Sydney".to_owned(),
                    }],
                    7,
                ))
            }
        })
    }

    fn current<'a>(
        &'a self,
        _location: &'a WeatherLocation,
    ) -> WeatherFuture<'a, Result<(CurrentWeather, u64), ProviderFailure>> {
        Box::pin(async { Err(ProviderFailure::new(ProviderFailureCategory::Unavailable)) })
    }
}

async fn service_fixture() -> (
    tempfile::TempDir,
    Storage,
    Arc<FakeProvider>,
    WeatherService,
) {
    let temp = tempfile::tempdir().expect("temp root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("paths");
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let provider = Arc::new(FakeProvider::new());
    let service = WeatherService::new(
        storage.repository(),
        provider.clone(),
        Arc::new(SystemClock),
    );
    (temp, storage, provider, service)
}

#[test]
fn weather_provider_forecast_request_contains_only_fixed_shape_and_rounded_location() {
    let location = WeatherLocation {
        city: "Sydney".to_owned(),
        region: None,
        country: "Australia".to_owned(),
        country_code: "AU".to_owned(),
        latitude: -33.868_812,
        longitude: 151.209_295,
        timezone: "Australia/Sydney".to_owned(),
    };
    let url = forecast_url(&location).expect("url");
    let pairs = url.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(
        pairs.get("latitude").map(std::convert::AsRef::as_ref),
        Some("-33.8688")
    );
    assert_eq!(
        pairs.get("longitude").map(std::convert::AsRef::as_ref),
        Some("151.2093")
    );
    assert_eq!(
        pairs.get("timezone").map(std::convert::AsRef::as_ref),
        Some("Australia/Sydney")
    );
    assert_eq!(
        pairs.get("current").map(std::convert::AsRef::as_ref),
        Some(CURRENT_VARIABLES)
    );
    assert_eq!(pairs.len(), 6);
    for forbidden in ["name", "city", "region", "country"] {
        assert!(!pairs.contains_key(forbidden));
    }
}

#[tokio::test]
async fn weather_provider_search_requires_explicit_valid_query_and_is_idempotent() {
    let (_temp, storage, provider, service) = service_fixture().await;
    let invalid = service
        .search_locations(SearchWeatherLocationsRequest {
            client_request_id: Uuid::now_v7(),
            query: "S".to_owned(),
            limit: 5,
        })
        .await
        .expect_err("single character must fail");
    assert_eq!(invalid.error_id, crate::ipc::ErrorId::RequestInvalid);
    assert_eq!(provider.searches.load(Ordering::SeqCst), 0);

    let request = SearchWeatherLocationsRequest {
        client_request_id: Uuid::now_v7(),
        query: "Sydney".to_owned(),
        limit: 5,
    };
    let first = service
        .search_locations(request.clone())
        .await
        .expect("search");
    let retry = service.search_locations(request).await.expect("retry");
    assert_eq!(first, retry);
    assert_eq!(provider.searches.load(Ordering::SeqCst), 1);
    storage.close().await;
}

#[tokio::test]
async fn offline_recovery_cancels_inflight_weather_and_requires_a_fresh_request() {
    let temp = tempfile::tempdir().expect("temp root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("paths");
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let provider = Arc::new(SuspendProvider {
        searches: AtomicUsize::new(0),
        started: Notify::new(),
    });
    let service = Arc::new(WeatherService::new(
        storage.repository(),
        provider.clone(),
        Arc::new(SystemClock),
    ));
    let inflight = tokio::spawn({
        let service = Arc::clone(&service);
        async move {
            service
                .search_locations(SearchWeatherLocationsRequest {
                    client_request_id: Uuid::now_v7(),
                    query: "Sydney".to_owned(),
                    limit: 1,
                })
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), provider.started.notified())
        .await
        .expect("network request started");

    service.prepare_suspend().await;
    let cancelled = tokio::time::timeout(Duration::from_secs(1), inflight)
        .await
        .expect("network request cancelled")
        .expect("search task")
        .expect_err("suspend cannot preserve a network result");
    assert_eq!(cancelled.error_id, crate::ipc::ErrorId::ProviderUnavailable);

    service.resume_after_suspend();
    let fresh = service
        .search_locations(SearchWeatherLocationsRequest {
            client_request_id: Uuid::now_v7(),
            query: "Sydney".to_owned(),
            limit: 1,
        })
        .await
        .expect("fresh request after recovery");
    assert_eq!(fresh.candidates.len(), 1);
    assert_eq!(provider.searches.load(Ordering::SeqCst), 2);
    storage.close().await;
}

#[tokio::test]
async fn weather_provider_selection_persists_only_signed_candidate() {
    let (_temp, storage, _provider, service) = service_fixture().await;
    let search = service
        .search_locations(SearchWeatherLocationsRequest {
            client_request_id: Uuid::now_v7(),
            query: "Sydney".to_owned(),
            limit: 5,
        })
        .await
        .expect("search");
    let selected = service
        .select_location(SelectWeatherLocationRequest {
            client_request_id: Uuid::now_v7(),
            candidate_id: search.candidates[0].candidate_id,
            expected_revision: 0,
        })
        .await
        .expect("select");
    assert_eq!(selected.revision, 1);
    let settings = storage
        .repository()
        .load_provider_settings()
        .await
        .expect("settings");
    assert_eq!(settings.weather_location.expect("location").city, "Sydney");

    let rejected = service
        .select_location(SelectWeatherLocationRequest {
            client_request_id: Uuid::now_v7(),
            candidate_id: Uuid::now_v7(),
            expected_revision: 1,
        })
        .await
        .expect_err("unsigned candidate");
    assert_eq!(rejected.error_id, crate::ipc::ErrorId::RequestInvalid);
    storage.close().await;
}

#[tokio::test]
async fn weather_provider_context_uses_fresh_cache_and_never_refetches() {
    let (_temp, storage, provider, service) = service_fixture().await;
    let search = service
        .search_locations(SearchWeatherLocationsRequest {
            client_request_id: Uuid::now_v7(),
            query: "Sydney".to_owned(),
            limit: 1,
        })
        .await
        .expect("search");
    service
        .select_location(SelectWeatherLocationRequest {
            client_request_id: Uuid::now_v7(),
            candidate_id: search.candidates[0].candidate_id,
            expected_revision: 0,
        })
        .await
        .expect("select");
    let mut settings = storage
        .repository()
        .load_provider_settings()
        .await
        .expect("settings");
    settings.weather_enabled = true;
    storage
        .repository()
        .save_provider_settings(1, &settings, Utc::now().timestamp_millis(), false, None)
        .await
        .expect("enable weather");

    let first = service.weather_summary().await.expect("weather summary");
    let second = service.weather_summary().await.expect("cached summary");
    assert_eq!(first, second);
    assert_eq!(provider.currents.load(Ordering::SeqCst), 1);
    storage.close().await;
}

#[test]
fn weather_provider_rejects_mismatched_timezone_and_invalid_numbers() {
    let response = ForecastResponse {
        timezone: "UTC".to_owned(),
        current: ForecastCurrent {
            time: "2026-09-03T12:00".to_owned(),
            temperature_2m: f64::NAN,
            apparent_temperature: 10.0,
            precipitation: 0.0,
            weather_code: 0,
            is_day: 1,
        },
    };
    let failure = validate_current_weather(&response, "Australia/Sydney").expect_err("invalid");
    assert_eq!(failure.category, ProviderFailureCategory::InvalidResponse);
}

#[tokio::test]
async fn weather_provider_retention_deletes_cache_at_seven_day_boundary() {
    let (_temp, storage, _provider, _service) = service_fixture().await;
    let fetched_at_ms = 1_000;
    let delete_after_ms = fetched_at_ms + WEATHER_RETENTION_MS;
    storage
        .repository()
        .save_weather_cache(&StoredWeatherCache {
            cache_key: "a".repeat(64),
            location: StoredWeatherLocation {
                city: "Sydney".to_owned(),
                region: None,
                country: "Australia".to_owned(),
                country_code: "AU".to_owned(),
                latitude: -33.8688,
                longitude: 151.2093,
                timezone: "Australia/Sydney".to_owned(),
            },
            request_shape_hash: "b".repeat(64),
            weather_json: serde_json::to_string(&CurrentWeather {
                observed_at: "2026-09-03T02:00:00.000Z".to_owned(),
                temperature_c: 18.5,
                apparent_temperature_c: 17.2,
                precipitation_mm: 0.0,
                weather_code: 1,
                is_day: true,
            })
            .expect("weather json"),
            fetched_at_ms,
            expires_at_ms: fetched_at_ms + WEATHER_FRESH_MS,
            delete_after_ms,
        })
        .await
        .expect("cache");
    let before = storage
        .repository()
        .run_retention_batch(delete_after_ms - 1, 500)
        .await
        .expect("before boundary");
    assert_eq!(before.weather_cache_deleted, 0);
    let at = storage
        .repository()
        .run_retention_batch(delete_after_ms, 500)
        .await
        .expect("at boundary");
    assert_eq!(at.weather_cache_deleted, 1);
    storage.close().await;
}
