//! Privacy-preserving metadata provider boundary.
//!
//! The transport accepts closed request types so local paths, audio, user
//! identity, and playback history cannot accidentally cross the provider
//! boundary. Concrete HTTP wiring belongs in the application composition root;
//! tests inject a hermetic transport.

use crate::providers::{Clock, ProviderCallContext, ProviderFailure, ProviderFailureCategory};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    hash::{Hash, Hasher},
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;
use uuid::Uuid;

const REQUEST_INTERVAL_MS: i64 = 1_000;
const NEGATIVE_CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
const COVER_CACHE_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const MAX_COVER_BYTES: usize = 5 * 1_024 * 1_024;
const MAX_IMAGE_DIMENSION: u32 = 4_096;
const METADATA_DEADLINE: Duration = Duration::from_secs(15);
const COVER_DEADLINE: Duration = Duration::from_secs(30);
const MAX_METADATA_RESPONSE_BYTES: usize = 1_048_576;

pub type MetadataFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn invalid_response() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::InvalidResponse)
}

fn unavailable() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::Unavailable)
}

fn timeout() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::Timeout)
}

fn normalize_text(
    value: Option<String>,
    max_chars: usize,
) -> Result<Option<String>, ProviderFailure> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.chars().any(char::is_control) {
        return Err(invalid_response());
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Ok(None);
    }
    if normalized.chars().count() > max_chars {
        return Err(invalid_response());
    }
    Ok(Some(normalized))
}

fn comparison_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The complete `MusicBrainz` disclosure surface.
#[derive(Clone, Debug, Eq)]
pub struct MetadataQuery {
    title: Option<String>,
    artists: Vec<String>,
    album: Option<String>,
    duration_ms: Option<u64>,
}

impl PartialEq for MetadataQuery {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title
            && self.artists == other.artists
            && self.album == other.album
            && self.duration_ms == other.duration_ms
    }
}

impl Hash for MetadataQuery {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.title.hash(state);
        self.artists.hash(state);
        self.album.hash(state);
        self.duration_ms.hash(state);
    }
}

impl MetadataQuery {
    /// Builds and normalizes an allowlisted metadata query.
    ///
    /// # Errors
    ///
    /// Returns `InvalidResponse` for control characters, oversized values, too
    /// many artists, an implausible duration, or a query with no title/artist.
    pub fn new(
        title: Option<String>,
        artists: Vec<String>,
        album: Option<String>,
        duration_ms: Option<u64>,
    ) -> Result<Self, ProviderFailure> {
        if artists.len() > 10 || duration_ms.is_some_and(|value| value == 0 || value > 86_400_000) {
            return Err(invalid_response());
        }
        let title = normalize_text(title, 300)?;
        let album = normalize_text(album, 300)?;
        let artists = artists
            .into_iter()
            .map(|artist| normalize_text(Some(artist), 200))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if title.is_none() && artists.is_empty() {
            return Err(invalid_response());
        }
        Ok(Self {
            title,
            artists,
            album,
            duration_ms,
        })
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn artists(&self) -> &[String] {
        &self.artists
    }

    #[must_use]
    pub fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }

    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }
}

/// Validated User-Agent required for every metadata request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataUserAgent(String);

impl MetadataUserAgent {
    /// # Errors
    ///
    /// Rejects unsafe version text or a contact value that is not HTTPS or a
    /// syntactically plausible `mailto:` address.
    pub fn new(version: &str, contact: &str) -> Result<Self, ProviderFailure> {
        let valid_version = !version.is_empty()
            && version.len() <= 64
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'));
        let valid_contact = if contact.starts_with("https://") {
            Url::parse(contact).is_ok_and(|url| url.host_str().is_some())
        } else if let Some(address) = contact.strip_prefix("mailto:") {
            address.len() <= 254
                && !address.contains(char::is_whitespace)
                && address.split_once('@').is_some_and(|(left, right)| {
                    !left.is_empty() && right.contains('.') && !right.ends_with('.')
                })
        } else {
            false
        };
        if !valid_version || !valid_contact || contact.len() > 512 {
            return Err(invalid_response());
        }
        Ok(Self(format!("CyberKindred/{version} ({contact})")))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataMatch {
    pub recording_mbid: Uuid,
    pub release_mbid: Option<Uuid>,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub tags: Vec<String>,
    pub confidence: f64,
}

impl MetadataMatch {
    /// Validates a normalized transport result before it can enter cache.
    ///
    /// # Errors
    ///
    /// Returns `InvalidResponse` for malformed text, excessive collections, or
    /// a non-finite/out-of-range confidence score.
    pub fn validate(mut self) -> Result<Self, ProviderFailure> {
        if self.recording_mbid.is_nil()
            || self.release_mbid.is_some_and(|value| value.is_nil())
            || self.artists.is_empty()
            || self.artists.len() > 10
            || self.tags.len() > 100
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
        {
            return Err(invalid_response());
        }
        self.title = normalize_text(Some(self.title), 300)?.ok_or_else(invalid_response)?;
        self.album = normalize_text(self.album, 300)?;
        self.artists = self
            .artists
            .into_iter()
            .map(|value| normalize_text(Some(value), 200)?.ok_or_else(invalid_response))
            .collect::<Result<Vec<_>, _>>()?;
        self.tags = self
            .tags
            .into_iter()
            .map(|value| normalize_text(Some(value), 100)?.ok_or_else(invalid_response))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CoverSize {
    Px500,
}

impl CoverSize {
    #[must_use]
    pub const fn pixels(self) -> u16 {
        match self {
            Self::Px500 => 500,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MusicBrainzRequest {
    query: MetadataQuery,
    user_agent: MetadataUserAgent,
}

impl MusicBrainzRequest {
    #[must_use]
    pub const fn query(&self) -> &MetadataQuery {
        &self.query
    }

    #[must_use]
    pub const fn user_agent(&self) -> &MetadataUserAgent {
        &self.user_agent
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverArtRequest {
    release_mbid: Uuid,
    size: CoverSize,
    user_agent: MetadataUserAgent,
}

impl CoverArtRequest {
    #[must_use]
    pub const fn release_mbid(&self) -> Uuid {
        self.release_mbid
    }

    #[must_use]
    pub const fn size(&self) -> CoverSize {
        self.size
    }

    #[must_use]
    pub const fn user_agent(&self) -> &MetadataUserAgent {
        &self.user_agent
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MetadataTransportResult {
    Matches(Vec<MetadataMatch>),
    NotFound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoverTransportResult {
    Found {
        bytes: Vec<u8>,
        declared_mime: String,
        source_url: String,
        etag: Option<String>,
        attribution: Option<String>,
    },
    NotFound,
}

/// Injectable network boundary. Implementations must use fixed provider hosts,
/// GET only, and must not log response bodies or request contents.
pub trait MetadataTransport: Send + Sync {
    fn search<'a>(
        &'a self,
        request: &'a MusicBrainzRequest,
        timeout: Duration,
    ) -> MetadataFuture<'a, Result<MetadataTransportResult, ProviderFailure>>;

    fn fetch_cover<'a>(
        &'a self,
        request: &'a CoverArtRequest,
        timeout: Duration,
    ) -> MetadataFuture<'a, Result<CoverTransportResult, ProviderFailure>>;
}

/// Fixed-host production transport. It disables reqwest redirects globally so
/// every CAA hop can be validated before the next request is issued.
pub struct ReqwestMetadataTransport {
    client: reqwest::Client,
}

impl ReqwestMetadataTransport {
    /// # Errors
    ///
    /// Returns an unavailable provider failure if the TLS-only client cannot be
    /// constructed.
    pub fn new() -> Result<Self, ProviderFailure> {
        reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(|client| Self { client })
            .map_err(|_| unavailable())
    }

    async fn search_inner(
        &self,
        request: &MusicBrainzRequest,
        timeout_duration: Duration,
    ) -> Result<MetadataTransportResult, ProviderFailure> {
        let mut endpoint =
            Url::parse("https://musicbrainz.org/ws/2/recording").map_err(|_| invalid_response())?;
        endpoint
            .query_pairs_mut()
            .append_pair("query", &musicbrainz_query(&request.query))
            .append_pair("fmt", "json")
            .append_pair("limit", "10");
        let response = self
            .client
            .get(endpoint)
            .header(reqwest::header::USER_AGENT, request.user_agent.as_str())
            .timeout(timeout_duration)
            .send()
            .await
            .map_err(|error| map_reqwest_error(&error))?;
        match response.status() {
            reqwest::StatusCode::OK => {
                let body = read_bounded(response, MAX_METADATA_RESPONSE_BYTES).await?;
                parse_musicbrainz_response(&body)
            }
            reqwest::StatusCode::NOT_FOUND => Ok(MetadataTransportResult::NotFound),
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(rate_limit_failure(response.headers())),
            status if status.is_server_error() => Err(unavailable()),
            _ => Err(invalid_response()),
        }
    }

    async fn cover_inner(
        &self,
        request: &CoverArtRequest,
        timeout_duration: Duration,
    ) -> Result<CoverTransportResult, ProviderFailure> {
        let transport_deadline = Instant::now()
            .checked_add(timeout_duration)
            .ok_or_else(timeout)?;
        let mut current = Url::parse(&format!(
            "https://coverartarchive.org/release/{}/front-{}",
            request.release_mbid,
            request.size.pixels()
        ))
        .map_err(|_| invalid_response())?;
        for redirect_count in 0..=3 {
            validate_cover_url(&current)?;
            let remaining = transport_deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(timeout)?;
            let response = self
                .client
                .get(current.clone())
                .header(reqwest::header::USER_AGENT, request.user_agent.as_str())
                .timeout(remaining)
                .send()
                .await
                .map_err(|error| map_reqwest_error(&error))?;
            if response.status().is_redirection() {
                if redirect_count == 3 {
                    return Err(invalid_response());
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(invalid_response)?;
                current = current.join(location).map_err(|_| invalid_response())?;
                validate_cover_url(&current)?;
                continue;
            }
            return match response.status() {
                reqwest::StatusCode::OK => {
                    let declared_mime = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.split(';').next())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(invalid_response)?
                        .to_owned();
                    let etag = response
                        .headers()
                        .get(reqwest::header::ETAG)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let bytes = read_bounded(response, MAX_COVER_BYTES).await?;
                    Ok(CoverTransportResult::Found {
                        bytes,
                        declared_mime,
                        source_url: current.to_string(),
                        etag,
                        attribution: Some("Cover Art Archive".to_owned()),
                    })
                }
                reqwest::StatusCode::NOT_FOUND => Ok(CoverTransportResult::NotFound),
                reqwest::StatusCode::TOO_MANY_REQUESTS => {
                    Err(rate_limit_failure(response.headers()))
                }
                status if status.is_server_error() => Err(unavailable()),
                _ => Err(invalid_response()),
            };
        }
        Err(invalid_response())
    }
}

impl MetadataTransport for ReqwestMetadataTransport {
    fn search<'a>(
        &'a self,
        request: &'a MusicBrainzRequest,
        timeout: Duration,
    ) -> MetadataFuture<'a, Result<MetadataTransportResult, ProviderFailure>> {
        Box::pin(self.search_inner(request, timeout))
    }

    fn fetch_cover<'a>(
        &'a self,
        request: &'a CoverArtRequest,
        timeout: Duration,
    ) -> MetadataFuture<'a, Result<CoverTransportResult, ProviderFailure>> {
        Box::pin(self.cover_inner(request, timeout))
    }
}

fn musicbrainz_query(query: &MetadataQuery) -> String {
    fn clause(field: &str, value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{field}:\"{escaped}\"")
    }

    let mut clauses = Vec::with_capacity(4);
    if let Some(title) = query.title() {
        clauses.push(clause("recording", title));
    }
    if !query.artists().is_empty() {
        clauses.push(clause("artist", &query.artists().join(" ")));
    }
    if let Some(album) = query.album() {
        clauses.push(clause("release", album));
    }
    if let Some(duration_ms) = query.duration_ms() {
        clauses.push(format!("dur:{duration_ms}"));
    }
    clauses.join(" AND ")
}

fn validate_cover_url(url: &Url) -> Result<(), ProviderFailure> {
    let host = url.host_str().ok_or_else(invalid_response)?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !(host == "coverartarchive.org"
            || host == "archive.org"
            || host.ends_with(".archive.org"))
    {
        return Err(invalid_response());
    }
    Ok(())
}

async fn read_bounded(
    mut response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, ProviderFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(invalid_response());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| map_reqwest_error(&error))?
    {
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return Err(invalid_response());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn map_reqwest_error(error: &reqwest::Error) -> ProviderFailure {
    if error.is_timeout() {
        timeout()
    } else {
        unavailable()
    }
}

fn rate_limit_failure(headers: &reqwest::header::HeaderMap) -> ProviderFailure {
    let retry_after_ms = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1_000));
    ProviderFailure::rate_limited(retry_after_ms)
}

#[derive(Deserialize)]
struct MusicBrainzResponse {
    #[serde(default)]
    recordings: Vec<MusicBrainzRecording>,
}

#[derive(Deserialize)]
struct MusicBrainzRecording {
    id: String,
    title: String,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<MusicBrainzArtistCredit>,
    #[serde(default)]
    releases: Vec<MusicBrainzRelease>,
    #[serde(default)]
    tags: Vec<MusicBrainzTag>,
    #[serde(default, alias = "ext:score")]
    score: Option<f64>,
}

#[derive(Deserialize)]
struct MusicBrainzArtistCredit {
    name: String,
}

#[derive(Deserialize)]
struct MusicBrainzRelease {
    id: String,
    title: String,
}

#[derive(Deserialize)]
struct MusicBrainzTag {
    name: String,
}

fn parse_musicbrainz_response(body: &[u8]) -> Result<MetadataTransportResult, ProviderFailure> {
    let response: MusicBrainzResponse =
        serde_json::from_slice(body).map_err(|_| invalid_response())?;
    if response.recordings.len() > 10 {
        return Err(invalid_response());
    }
    let matches = response
        .recordings
        .into_iter()
        .map(|recording| {
            let release = recording.releases.into_iter().next();
            MetadataMatch {
                recording_mbid: Uuid::parse_str(&recording.id).map_err(|_| invalid_response())?,
                release_mbid: release
                    .as_ref()
                    .map(|value| Uuid::parse_str(&value.id).map_err(|_| invalid_response()))
                    .transpose()?,
                title: recording.title,
                artists: recording
                    .artist_credit
                    .into_iter()
                    .map(|artist| artist.name)
                    .collect(),
                album: release.map(|value| value.title),
                tags: recording.tags.into_iter().map(|tag| tag.name).collect(),
                confidence: recording.score.unwrap_or(0.0) / 100.0,
            }
            .validate()
        })
        .collect::<Result<Vec<_>, _>>()?;
    if matches.is_empty() {
        Ok(MetadataTransportResult::NotFound)
    } else {
        Ok(MetadataTransportResult::Matches(matches))
    }
}

pub trait MetadataSleeper: Send + Sync {
    fn sleep(&self, duration: Duration) -> MetadataFuture<'_, ()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TokioMetadataSleeper;

impl MetadataSleeper for TokioMetadataSleeper {
    fn sleep(&self, duration: Duration) -> MetadataFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// One shared instance serializes `MusicBrainz` and CAA calls process-wide.
pub struct GlobalMetadataRateLimiter {
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn MetadataSleeper>,
    last_request_ms: tokio::sync::Mutex<Option<i64>>,
}

impl GlobalMetadataRateLimiter {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, sleeper: Arc<dyn MetadataSleeper>) -> Self {
        Self {
            clock,
            sleeper,
            last_request_ms: tokio::sync::Mutex::new(None),
        }
    }

    async fn acquire(&self, context: &ProviderCallContext) -> Result<(), ProviderFailure> {
        let mut last_request = self.last_request_ms.lock().await;
        ensure_context(context)?;
        if let Some(previous_ms) = *last_request {
            let now_ms = self.clock.now_ms();
            let wait_ms = previous_ms.saturating_add(REQUEST_INTERVAL_MS) - now_ms;
            if wait_ms > 0 {
                let duration = Duration::from_millis(u64::try_from(wait_ms).unwrap_or(u64::MAX));
                await_context(self.sleeper.sleep(duration), context).await?;
            }
        }
        ensure_context(context)?;
        let now_ms = self.clock.now_ms();
        *last_request = Some(last_request.map_or(now_ms, |previous| {
            now_ms.max(previous.saturating_add(REQUEST_INTERVAL_MS))
        }));
        Ok(())
    }
}

async fn cancellation_wait(context: &ProviderCallContext) {
    while !context.cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn ensure_context(context: &ProviderCallContext) -> Result<(), ProviderFailure> {
    if context.cancellation.is_cancelled() {
        return Err(unavailable());
    }
    if Instant::now() >= context.deadline {
        return Err(timeout());
    }
    Ok(())
}

async fn await_context<T>(
    future: MetadataFuture<'_, T>,
    context: &ProviderCallContext,
) -> Result<T, ProviderFailure> {
    ensure_context(context)?;
    let remaining = context
        .deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(timeout)?;
    tokio::select! {
        result = future => Ok(result),
        () = tokio::time::sleep(remaining) => Err(timeout()),
        () = cancellation_wait(context) => Err(unavailable()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheSource {
    Live,
    Cache,
    CacheFallback,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataLookup {
    pub matches: Vec<MetadataMatch>,
    pub fetched_at_ms: i64,
    pub source: CacheSource,
}

#[derive(Clone)]
enum MatchOutcome {
    Matches(Vec<MetadataMatch>),
    NoMatch,
}

#[derive(Clone)]
struct CachedMatch {
    outcome: MatchOutcome,
    fetched_at_ms: i64,
    expires_at_ms: Option<i64>,
    last_access: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverArtifact {
    pub source_url: String,
    pub etag: Option<String>,
    pub declared_mime: String,
    pub attribution: Option<String>,
    pub fetched_at_ms: i64,
    pub byte_length: usize,
}

#[derive(Clone)]
struct CachedCover {
    artifact: CoverArtifact,
    bytes: Option<Arc<[u8]>>,
    expires_at_ms: i64,
    last_access: u64,
}

#[derive(Default)]
struct CacheState {
    matches: HashMap<MetadataQuery, CachedMatch>,
    covers: HashMap<(Uuid, CoverSize), CachedCover>,
    access_sequence: u64,
    cover_bytes: usize,
}

/// Bounded, in-memory cache. Persistent storage can implement the same policy
/// when TASK-013 is composed with the repository layer.
pub struct MetadataCache {
    max_match_entries: usize,
    max_cover_entries: usize,
    max_cover_bytes: usize,
    state: Mutex<CacheState>,
}

/// Serializable persistence handoff for `MusicBrainz` positive/no-match facts.
/// Import always re-runs the same query and candidate validation as live data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataCacheFact {
    provider: String,
    title: Option<String>,
    artists: Vec<String>,
    album: Option<String>,
    duration_ms: Option<u64>,
    matches: Vec<MetadataMatch>,
    fetched_at_ms: i64,
    expires_at_ms: Option<i64>,
}

impl MetadataCache {
    #[must_use]
    pub fn new(max_match_entries: usize, max_cover_entries: usize, max_cover_bytes: usize) -> Self {
        Self {
            max_match_entries: max_match_entries.clamp(1, 65_536),
            max_cover_entries: max_cover_entries.clamp(1, 4_096),
            max_cover_bytes: max_cover_bytes.clamp(1, 512 * 1_024 * 1_024),
            state: Mutex::new(CacheState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn match_entry(&self, query: &MetadataQuery, now_ms: i64) -> Option<CachedMatch> {
        let mut state = self.lock();
        if state
            .matches
            .get(query)
            .and_then(|entry| entry.expires_at_ms)
            .is_some_and(|expires_at| expires_at <= now_ms)
        {
            state.matches.remove(query);
            return None;
        }
        state.access_sequence = state.access_sequence.saturating_add(1);
        let sequence = state.access_sequence;
        state.matches.get_mut(query).map(|entry| {
            entry.last_access = sequence;
            entry.clone()
        })
    }

    fn insert_match(&self, query: MetadataQuery, mut entry: CachedMatch) {
        let mut state = self.lock();
        state.access_sequence = state.access_sequence.saturating_add(1);
        entry.last_access = state.access_sequence;
        state.matches.insert(query, entry);
        while state.matches.len() > self.max_match_entries {
            let oldest = state
                .matches
                .iter()
                .min_by_key(|(_, value)| value.last_access)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                state.matches.remove(&key);
            } else {
                break;
            }
        }
    }

    fn cover_entry(&self, key: (Uuid, CoverSize), now_ms: i64) -> Option<CachedCover> {
        let mut state = self.lock();
        if state
            .covers
            .get(&key)
            .is_some_and(|entry| entry.expires_at_ms <= now_ms)
        {
            if let Some(expired) = state.covers.remove(&key) {
                state.cover_bytes = state
                    .cover_bytes
                    .saturating_sub(expired.bytes.as_ref().map_or(0, |bytes| bytes.len()));
            }
            return None;
        }
        state.access_sequence = state.access_sequence.saturating_add(1);
        let sequence = state.access_sequence;
        state.covers.get_mut(&key).map(|entry| {
            entry.last_access = sequence;
            entry.clone()
        })
    }

    fn insert_cover(&self, key: (Uuid, CoverSize), mut entry: CachedCover) {
        let mut state = self.lock();
        state.access_sequence = state.access_sequence.saturating_add(1);
        entry.last_access = state.access_sequence;
        if let Some(previous) = state.covers.remove(&key) {
            state.cover_bytes = state
                .cover_bytes
                .saturating_sub(previous.bytes.as_ref().map_or(0, |bytes| bytes.len()));
        }
        state.cover_bytes = state
            .cover_bytes
            .saturating_add(entry.bytes.as_ref().map_or(0, |bytes| bytes.len()));
        state.covers.insert(key, entry);
        while state.covers.len() > self.max_cover_entries
            || state.cover_bytes > self.max_cover_bytes
        {
            let oldest = state
                .covers
                .iter()
                .min_by_key(|(_, value)| value.last_access)
                .map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                if let Some(removed) = state.covers.remove(&oldest) {
                    state.cover_bytes = state
                        .cover_bytes
                        .saturating_sub(removed.bytes.as_ref().map_or(0, |bytes| bytes.len()));
                }
            } else {
                break;
            }
        }
    }

    pub fn invalidate_match(&self, query: &MetadataQuery) {
        self.lock().matches.remove(query);
    }

    #[must_use]
    pub fn snapshot_match_facts(&self) -> Vec<MetadataCacheFact> {
        let state = self.lock();
        let mut facts = state
            .matches
            .iter()
            .map(|(query, entry)| MetadataCacheFact {
                provider: "musicbrainz".to_owned(),
                title: query.title.clone(),
                artists: query.artists.clone(),
                album: query.album.clone(),
                duration_ms: query.duration_ms,
                matches: match &entry.outcome {
                    MatchOutcome::Matches(values) => values.clone(),
                    MatchOutcome::NoMatch => Vec::new(),
                },
                fetched_at_ms: entry.fetched_at_ms,
                expires_at_ms: entry.expires_at_ms,
            })
            .collect::<Vec<_>>();
        facts.sort_by(|left, right| {
            left.fetched_at_ms
                .cmp(&right.fetched_at_ms)
                .then_with(|| left.title.cmp(&right.title))
        });
        facts
    }

    /// Restores validated persistence facts without issuing provider calls.
    ///
    /// # Errors
    ///
    /// Rejects malformed queries, candidates, or cache policy timestamps.
    pub fn restore_match_facts(
        &self,
        facts: Vec<MetadataCacheFact>,
    ) -> Result<(), ProviderFailure> {
        let mut validated = Vec::with_capacity(facts.len());
        for fact in facts {
            if fact.provider != "musicbrainz" {
                return Err(invalid_response());
            }
            let query = MetadataQuery::new(fact.title, fact.artists, fact.album, fact.duration_ms)?;
            let matches = fact
                .matches
                .into_iter()
                .map(MetadataMatch::validate)
                .collect::<Result<Vec<_>, _>>()?;
            let (outcome, expires_at_ms) = if matches.is_empty() {
                let expires_at_ms = fact.expires_at_ms.ok_or_else(invalid_response)?;
                if expires_at_ms != fact.fetched_at_ms.saturating_add(NEGATIVE_CACHE_TTL_MS) {
                    return Err(invalid_response());
                }
                (MatchOutcome::NoMatch, Some(expires_at_ms))
            } else {
                if fact.expires_at_ms.is_some()
                    || matches.iter().any(|candidate| candidate.confidence < 0.70)
                {
                    return Err(invalid_response());
                }
                (MatchOutcome::Matches(matches), None)
            };
            validated.push((
                query,
                CachedMatch {
                    outcome,
                    fetched_at_ms: fact.fetched_at_ms,
                    expires_at_ms,
                    last_access: 0,
                },
            ));
        }
        for (query, entry) in validated {
            self.insert_match(query, entry);
        }
        Ok(())
    }
}

pub trait CoverSink: Send {
    /// # Errors
    ///
    /// Returns a redacted failure if the Rust-owned destination cannot accept
    /// all validated bytes.
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ProviderFailure>;
}

pub trait MetadataProvider: Send + Sync {
    fn match_recording<'a>(
        &'a self,
        input: MetadataQuery,
        context: &'a ProviderCallContext,
    ) -> MetadataFuture<'a, Result<Vec<MetadataMatch>, ProviderFailure>>;

    fn fetch_cover<'a>(
        &'a self,
        release_mbid: Uuid,
        size: CoverSize,
        sink: &'a mut dyn CoverSink,
        context: &'a ProviderCallContext,
    ) -> MetadataFuture<'a, Result<Option<CoverArtifact>, ProviderFailure>>;
}

pub struct CachedMetadataProvider {
    transport: Arc<dyn MetadataTransport>,
    user_agent: MetadataUserAgent,
    limiter: Arc<GlobalMetadataRateLimiter>,
    cache: Arc<MetadataCache>,
    clock: Arc<dyn Clock>,
}

impl CachedMetadataProvider {
    #[must_use]
    pub fn new(
        transport: Arc<dyn MetadataTransport>,
        user_agent: MetadataUserAgent,
        limiter: Arc<GlobalMetadataRateLimiter>,
        cache: Arc<MetadataCache>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            transport,
            user_agent,
            limiter,
            cache,
            clock,
        }
    }

    /// Uses a fresh positive/no-match cache entry or performs one bounded call.
    ///
    /// # Errors
    ///
    /// Returns a redacted provider failure for cancellation, timeout, network
    /// failure, or an invalid provider response.
    pub async fn lookup_recording(
        &self,
        query: MetadataQuery,
        context: &ProviderCallContext,
    ) -> Result<MetadataLookup, ProviderFailure> {
        ensure_context(context)?;
        let now_ms = self.clock.now_ms();
        if let Some(cached) = self.cache.match_entry(&query, now_ms) {
            return Ok(lookup_from_cache(cached, CacheSource::Cache));
        }
        self.request_recording(query, context).await
    }

    /// Refreshes from the provider and falls back to a fresh cached fact.
    ///
    /// # Errors
    ///
    /// Returns the redacted live failure only when no usable cache exists.
    pub async fn refresh_recording(
        &self,
        query: MetadataQuery,
        context: &ProviderCallContext,
    ) -> Result<MetadataLookup, ProviderFailure> {
        let fallback = self.cache.match_entry(&query, self.clock.now_ms());
        match self.request_recording(query, context).await {
            Ok(result) => Ok(result),
            Err(error) if ensure_context(context).is_ok() => fallback
                .map(|cached| lookup_from_cache(cached, CacheSource::CacheFallback))
                .ok_or(error),
            Err(error) => Err(error),
        }
    }

    async fn request_recording(
        &self,
        query: MetadataQuery,
        context: &ProviderCallContext,
    ) -> Result<MetadataLookup, ProviderFailure> {
        let started_at = Instant::now();
        self.limiter.acquire(context).await?;
        let remaining = bounded_remaining(context, METADATA_DEADLINE, started_at)?;
        let request = MusicBrainzRequest {
            query: query.clone(),
            user_agent: self.user_agent.clone(),
        };
        let response = await_context(self.transport.search(&request, remaining), context).await??;
        ensure_context(context)?;
        let fetched_at_ms = self.clock.now_ms();
        let matches = match response {
            MetadataTransportResult::Matches(values) => values
                .into_iter()
                .map(MetadataMatch::validate)
                .collect::<Result<Vec<_>, _>>()?,
            MetadataTransportResult::NotFound => Vec::new(),
        };
        let retained = matches
            .iter()
            .filter(|candidate| candidate.confidence >= 0.70)
            .cloned()
            .collect::<Vec<_>>();
        let (outcome, expires_at_ms) = if retained.is_empty() {
            (
                MatchOutcome::NoMatch,
                Some(fetched_at_ms.saturating_add(NEGATIVE_CACHE_TTL_MS)),
            )
        } else {
            (MatchOutcome::Matches(retained.clone()), None)
        };
        self.cache.insert_match(
            query,
            CachedMatch {
                outcome,
                fetched_at_ms,
                expires_at_ms,
                last_access: 0,
            },
        );
        Ok(MetadataLookup {
            matches: retained,
            fetched_at_ms,
            source: CacheSource::Live,
        })
    }

    async fn fetch_cover_lookup(
        &self,
        release_mbid: Uuid,
        size: CoverSize,
        context: &ProviderCallContext,
    ) -> Result<Option<(CoverArtifact, Arc<[u8]>)>, ProviderFailure> {
        ensure_context(context)?;
        if release_mbid.is_nil() {
            return Err(invalid_response());
        }
        let key = (release_mbid, size);
        if let Some(cached) = self.cache.cover_entry(key, self.clock.now_ms()) {
            return Ok(cached.bytes.map(|bytes| (cached.artifact, bytes)));
        }
        let started_at = Instant::now();
        self.limiter.acquire(context).await?;
        let remaining = bounded_remaining(context, COVER_DEADLINE, started_at)?;
        let request = CoverArtRequest {
            release_mbid,
            size,
            user_agent: self.user_agent.clone(),
        };
        let response =
            await_context(self.transport.fetch_cover(&request, remaining), context).await??;
        ensure_context(context)?;
        let fetched_at_ms = self.clock.now_ms();
        match response {
            CoverTransportResult::NotFound => {
                self.cache.insert_cover(
                    key,
                    CachedCover {
                        artifact: CoverArtifact {
                            source_url: String::new(),
                            etag: None,
                            declared_mime: String::new(),
                            attribution: None,
                            fetched_at_ms,
                            byte_length: 0,
                        },
                        bytes: None,
                        expires_at_ms: fetched_at_ms.saturating_add(NEGATIVE_CACHE_TTL_MS),
                        last_access: 0,
                    },
                );
                Ok(None)
            }
            CoverTransportResult::Found {
                bytes,
                declared_mime,
                source_url,
                etag,
                attribution,
            } => {
                let artifact = validate_cover(
                    &bytes,
                    declared_mime,
                    source_url,
                    etag,
                    attribution,
                    fetched_at_ms,
                )?;
                let bytes = Arc::<[u8]>::from(bytes);
                self.cache.insert_cover(
                    key,
                    CachedCover {
                        artifact: artifact.clone(),
                        bytes: Some(Arc::clone(&bytes)),
                        expires_at_ms: fetched_at_ms.saturating_add(COVER_CACHE_TTL_MS),
                        last_access: 0,
                    },
                );
                Ok(Some((artifact, bytes)))
            }
        }
    }
}

impl MetadataProvider for CachedMetadataProvider {
    fn match_recording<'a>(
        &'a self,
        input: MetadataQuery,
        context: &'a ProviderCallContext,
    ) -> MetadataFuture<'a, Result<Vec<MetadataMatch>, ProviderFailure>> {
        Box::pin(async move {
            self.lookup_recording(input, context)
                .await
                .map(|lookup| lookup.matches)
        })
    }

    fn fetch_cover<'a>(
        &'a self,
        release_mbid: Uuid,
        size: CoverSize,
        sink: &'a mut dyn CoverSink,
        context: &'a ProviderCallContext,
    ) -> MetadataFuture<'a, Result<Option<CoverArtifact>, ProviderFailure>> {
        Box::pin(async move {
            let Some((artifact, bytes)) =
                self.fetch_cover_lookup(release_mbid, size, context).await?
            else {
                return Ok(None);
            };
            ensure_context(context)?;
            sink.write_all(&bytes)?;
            Ok(Some(artifact))
        })
    }
}

fn bounded_remaining(
    context: &ProviderCallContext,
    provider_deadline: Duration,
    started_at: Instant,
) -> Result<Duration, ProviderFailure> {
    let caller_remaining = context
        .deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(timeout)?;
    let provider_remaining = provider_deadline
        .checked_sub(started_at.elapsed())
        .ok_or_else(timeout)?;
    Ok(caller_remaining.min(provider_remaining))
}

fn lookup_from_cache(cached: CachedMatch, source: CacheSource) -> MetadataLookup {
    let matches = match cached.outcome {
        MatchOutcome::Matches(values) => values,
        MatchOutcome::NoMatch => Vec::new(),
    };
    MetadataLookup {
        matches,
        fetched_at_ms: cached.fetched_at_ms,
        source,
    }
}

fn validate_cover(
    bytes: &[u8],
    declared_mime: String,
    source_url: String,
    etag: Option<String>,
    attribution: Option<String>,
    fetched_at_ms: i64,
) -> Result<CoverArtifact, ProviderFailure> {
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES {
        return Err(invalid_response());
    }
    let parsed_url = Url::parse(&source_url).map_err(|_| invalid_response())?;
    let host = parsed_url.host_str().ok_or_else(invalid_response)?;
    if parsed_url.scheme() != "https"
        || !(host == "coverartarchive.org"
            || host == "archive.org"
            || host.ends_with(".archive.org"))
    {
        return Err(invalid_response());
    }
    let inferred = infer::get(bytes).ok_or_else(invalid_response)?.mime_type();
    if !matches!(inferred, "image/jpeg" | "image/png" | "image/webp") || declared_mime != inferred {
        return Err(invalid_response());
    }
    let (width, height) = image_dimensions(bytes, inferred).ok_or_else(invalid_response)?;
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(invalid_response());
    }
    if etag
        .as_ref()
        .is_some_and(|value| value.len() > 512 || value.chars().any(char::is_control))
        || attribution.as_ref().is_some_and(|value| {
            value.chars().count() > 1_000 || value.chars().any(char::is_control)
        })
    {
        return Err(invalid_response());
    }
    Ok(CoverArtifact {
        source_url,
        etag,
        declared_mime,
        attribution,
        fetched_at_ms,
        byte_length: bytes.len(),
    })
}

fn image_dimensions(bytes: &[u8], mime: &str) -> Option<(u32, u32)> {
    match mime {
        "image/png" if bytes.len() >= 24 => Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        )),
        "image/jpeg" => jpeg_dimensions(bytes),
        "image/webp" if bytes.len() >= 30 && &bytes[12..16] == b"VP8X" => {
            let width =
                u32::from(bytes[24]) | (u32::from(bytes[25]) << 8) | (u32::from(bytes[26]) << 16);
            let height =
                u32::from(bytes[27]) | (u32::from(bytes[28]) << 8) | (u32::from(bytes[29]) << 16);
            Some((width + 1, height + 1))
        }
        _ => None,
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0..2] != [0xff, 0xd8] {
        return None;
    }
    let mut offset = 2;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        if matches!(marker, 0xd8 | 0xd9) {
            continue;
        }
        if offset + 2 > bytes.len() {
            return None;
        }
        let segment_length = usize::from(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]));
        if segment_length < 2 || offset + segment_length > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if segment_length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]));
            let width = u32::from(u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]));
            return Some((width, height));
        }
        offset += segment_length;
    }
    None
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichedMetadata {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub provider: &'static str,
    pub confidence: f64,
    pub fetched_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    Matched,
    Review,
    Unmatched,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataProvenance {
    pub provider: &'static str,
    pub fetched_at_ms: i64,
    pub source: CacheSource,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichmentView {
    pub original: OriginalMetadata,
    pub enriched: Option<EnrichedMetadata>,
    pub match_status: MatchStatus,
    pub provenance: MetadataProvenance,
}

/// Builds API-015 enrichment fields without mutating source tags.
#[must_use]
pub fn build_enrichment_view(
    original: &OriginalMetadata,
    lookup: &MetadataLookup,
) -> EnrichmentView {
    let best = lookup.matches.iter().max_by(|left, right| {
        left.confidence
            .total_cmp(&right.confidence)
            .then_with(|| right.recording_mbid.cmp(&left.recording_mbid))
    });
    let exact = best.is_some_and(|candidate| {
        original
            .title
            .as_ref()
            .is_some_and(|title| comparison_text(title) == comparison_text(&candidate.title))
            && original.artist.as_ref().is_some_and(|artist| {
                candidate
                    .artists
                    .iter()
                    .any(|value| comparison_text(artist) == comparison_text(value))
            })
    });
    let (enriched, match_status) = match best {
        Some(candidate) if candidate.confidence >= 0.90 && exact => (
            Some(enriched_from_match(candidate, lookup.fetched_at_ms)),
            MatchStatus::Matched,
        ),
        Some(candidate) if candidate.confidence >= 0.70 => (
            Some(enriched_from_match(candidate, lookup.fetched_at_ms)),
            MatchStatus::Review,
        ),
        _ => (None, MatchStatus::Unmatched),
    };
    EnrichmentView {
        original: original.clone(),
        enriched,
        match_status,
        provenance: MetadataProvenance {
            provider: "musicbrainz",
            fetched_at_ms: lookup.fetched_at_ms,
            source: lookup.source,
        },
    }
}

fn enriched_from_match(candidate: &MetadataMatch, fetched_at_ms: i64) -> EnrichedMetadata {
    let fetched_at = match DateTime::<Utc>::from_timestamp_millis(fetched_at_ms) {
        Some(value) => value.to_rfc3339_opts(SecondsFormat::Millis, true),
        None => DateTime::<Utc>::UNIX_EPOCH.to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    EnrichedMetadata {
        title: candidate.title.clone(),
        artist: candidate.artists.join(", "),
        album: candidate.album.clone(),
        provider: "musicbrainz",
        confidence: candidate.confidence,
        fetched_at,
    }
}

#[cfg(test)]
mod tests;
