use super::{MAX_SPEECH_BYTES, SpeechArtifact, SpeechInput, invalid_response};
use crate::{
    providers::{CancellationFlag, ProviderCallContext, ProviderFailure, ProviderFailureCategory},
    storage::{CanonicalOrigin, SecretValue},
};
use serde::Serialize;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

const SPEECH_PATH: &str = "/v1/audio/speech";
const SPEECH_DEADLINE: Duration = Duration::from_secs(45);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub type SpeechFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SpeechSink: Send {
    /// # Errors
    ///
    /// Returns a redacted error if the Rust-owned destination cannot accept
    /// all validated bytes.
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ProviderFailure>;
}

/// The complete, closed disclosure surface for `OpenAI` Speech.
#[derive(Clone)]
pub struct SpeechTransportRequest {
    input: SpeechInput,
}

impl SpeechTransportRequest {
    #[must_use]
    pub const fn input(&self) -> &SpeechInput {
        &self.input
    }
}

pub struct SpeechTransportResponse {
    pub bytes: Vec<u8>,
    pub declared_mime: String,
}

pub trait SpeechTransport: Send + Sync {
    /// # Errors
    ///
    /// Returns only a classified, redacted provider failure.
    fn send<'a>(
        &'a self,
        request: &'a SpeechTransportRequest,
        secret: &'a SecretValue,
        timeout: Duration,
    ) -> SpeechFuture<'a, Result<SpeechTransportResponse, ProviderFailure>>;
}

/// Loads the current origin-scoped credential only for an authorized synth.
pub trait SpeechCredentialSource: Send + Sync {
    /// # Errors
    ///
    /// Returns Authentication when no validated key exists, or a redacted
    /// Unavailable failure when the credential store cannot be read.
    fn load(&self) -> Result<SecretValue, ProviderFailure>;
}

pub trait TtsProvider: Send + Sync {
    /// # Errors
    ///
    /// Returns a redacted provider failure and never retries automatically.
    fn synthesize<'a>(
        &'a self,
        input: SpeechInput,
        sink: &'a mut dyn SpeechSink,
        context: &'a ProviderCallContext,
    ) -> SpeechFuture<'a, Result<SpeechArtifact, ProviderFailure>>;
}

/// Production provider that keeps credential access and HTTP transport behind
/// injectable Rust-only boundaries.
pub struct OpenAiTtsProvider {
    transport: Arc<dyn SpeechTransport>,
    credentials: Arc<dyn SpeechCredentialSource>,
}

impl OpenAiTtsProvider {
    #[must_use]
    pub fn new(
        transport: Arc<dyn SpeechTransport>,
        credentials: Arc<dyn SpeechCredentialSource>,
    ) -> Self {
        Self {
            transport,
            credentials,
        }
    }

    async fn synthesize_inner(
        &self,
        input: SpeechInput,
        sink: &mut dyn SpeechSink,
        context: &ProviderCallContext,
    ) -> Result<SpeechArtifact, ProviderFailure> {
        ensure_context(context)?;
        let started_at = std::time::Instant::now();
        let secret = self.credentials.load()?;
        ensure_context(context)?;
        let caller_remaining = context
            .deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(timeout)?;
        let provider_remaining = SPEECH_DEADLINE
            .checked_sub(started_at.elapsed())
            .ok_or_else(timeout)?;
        let request = SpeechTransportRequest {
            input: input.clone(),
        };
        let response = await_context(
            self.transport
                .send(&request, &secret, caller_remaining.min(provider_remaining)),
            context,
        )
        .await??;
        ensure_context(context)?;
        validate_mp3(&response.bytes, &response.declared_mime)?;
        let artifact = SpeechArtifact::new(&input, response.bytes.len())?;
        sink.write_all(&response.bytes)?;
        ensure_context(context)?;
        Ok(artifact)
    }
}

impl TtsProvider for OpenAiTtsProvider {
    fn synthesize<'a>(
        &'a self,
        input: SpeechInput,
        sink: &'a mut dyn SpeechSink,
        context: &'a ProviderCallContext,
    ) -> SpeechFuture<'a, Result<SpeechArtifact, ProviderFailure>> {
        Box::pin(self.synthesize_inner(input, sink, context))
    }
}

/// Fixed-host HTTPS transport. Construction does not issue a request.
pub struct ReqwestSpeechTransport {
    client: reqwest::Client,
    endpoint: String,
}

impl ReqwestSpeechTransport {
    /// # Errors
    ///
    /// Returns a redacted unavailable error when the locked-down client cannot
    /// be constructed.
    pub fn new(origin: &CanonicalOrigin) -> Result<Self, ProviderFailure> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("CyberKindred/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| unavailable())?;
        Ok(Self {
            client,
            endpoint: format!("{}{SPEECH_PATH}", origin.as_str()),
        })
    }

    async fn send_inner(
        &self,
        request: &SpeechTransportRequest,
        secret: &SecretValue,
        timeout_duration: Duration,
    ) -> Result<SpeechTransportResponse, ProviderFailure> {
        let body = serde_json::to_vec(&OpenAiSpeechRequest::from(request.input()))
            .map_err(|_| invalid_response())?;
        let builder = self
            .client
            .post(&self.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        let builder = secret.with_exposed(|value| {
            builder
                .bearer_auth(value)
                .timeout(timeout_duration.min(SPEECH_DEADLINE))
        });
        let mut response = builder
            .send()
            .await
            .map_err(|error| map_reqwest_error(&error))?;
        match response.status() {
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
                let bytes = read_bounded(&mut response).await?;
                Ok(SpeechTransportResponse {
                    bytes,
                    declared_mime,
                })
            }
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(
                ProviderFailure::new(ProviderFailureCategory::Authentication),
            ),
            reqwest::StatusCode::TOO_MANY_REQUESTS => Err(rate_limit_failure(response.headers())),
            status if status.is_server_error() => Err(unavailable()),
            _ => Err(invalid_response()),
        }
    }
}

impl SpeechTransport for ReqwestSpeechTransport {
    fn send<'a>(
        &'a self,
        request: &'a SpeechTransportRequest,
        secret: &'a SecretValue,
        timeout: Duration,
    ) -> SpeechFuture<'a, Result<SpeechTransportResponse, ProviderFailure>> {
        Box::pin(self.send_inner(request, secret, timeout))
    }
}

#[derive(Serialize)]
struct OpenAiSpeechRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'static str,
    speed: f32,
}

impl<'a> From<&'a SpeechInput> for OpenAiSpeechRequest<'a> {
    fn from(input: &'a SpeechInput) -> Self {
        Self {
            model: input.model_id(),
            input: input.text(),
            voice: input.voice_id(),
            response_format: input.format().as_str(),
            speed: input.speed(),
        }
    }
}

async fn read_bounded(response: &mut reqwest::Response) -> Result<Vec<u8>, ProviderFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SPEECH_BYTES as u64)
    {
        return Err(invalid_response());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| map_reqwest_error(&error))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_SPEECH_BYTES {
            return Err(invalid_response());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn validate_mp3(bytes: &[u8], declared_mime: &str) -> Result<(), ProviderFailure> {
    let frame_offset = if bytes.starts_with(b"ID3") {
        id3_payload_offset(bytes)
    } else {
        Some(0)
    };
    if bytes.len() > MAX_SPEECH_BYTES
        || !matches!(declared_mime, "audio/mpeg" | "audio/mp3")
        || !frame_offset.is_some_and(|offset| valid_mp3_frame(bytes, offset))
    {
        return Err(invalid_response());
    }
    Ok(())
}

fn id3_payload_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 10 || bytes[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    let size = (usize::from(bytes[6]) << 21)
        | (usize::from(bytes[7]) << 14)
        | (usize::from(bytes[8]) << 7)
        | usize::from(bytes[9]);
    let footer = if bytes[5] & 0x10 == 0 { 0 } else { 10 };
    10usize.checked_add(size)?.checked_add(footer)
}

fn valid_mp3_frame(bytes: &[u8], offset: usize) -> bool {
    let Some(header) = bytes.get(offset..offset.saturating_add(4)) else {
        return false;
    };
    header[0] == 0xff
        && header[1] & 0xe0 == 0xe0
        && header[1] >> 3 & 0x03 != 0x01
        && header[1] >> 1 & 0x03 != 0
        && header[2] >> 4 != 0
        && header[2] >> 4 != 0x0f
        && header[2] >> 2 & 0x03 != 0x03
}

fn rate_limit_failure(headers: &reqwest::header::HeaderMap) -> ProviderFailure {
    let retry_after_ms = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1_000));
    ProviderFailure::rate_limited(retry_after_ms)
}

fn map_reqwest_error(error: &reqwest::Error) -> ProviderFailure {
    if error.is_timeout() {
        timeout()
    } else {
        unavailable()
    }
}

fn ensure_context(context: &ProviderCallContext) -> Result<(), ProviderFailure> {
    if context.cancellation.is_cancelled() {
        return Err(cancelled());
    }
    if std::time::Instant::now() >= context.deadline {
        return Err(timeout());
    }
    Ok(())
}

async fn await_context<T>(
    future: SpeechFuture<'_, T>,
    context: &ProviderCallContext,
) -> Result<T, ProviderFailure> {
    ensure_context(context)?;
    let remaining = context
        .deadline
        .checked_duration_since(std::time::Instant::now())
        .ok_or_else(timeout)?;
    tokio::select! {
        value = future => Ok(value),
        () = tokio::time::sleep(remaining) => Err(timeout()),
        () = wait_for_cancellation(&context.cancellation) => Err(cancelled()),
    }
}

pub(crate) async fn wait_for_cancellation(cancellation: &CancellationFlag) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn timeout() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::Timeout)
}

fn unavailable() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::Unavailable)
}

fn cancelled() -> ProviderFailure {
    ProviderFailure::new(ProviderFailureCategory::Unavailable)
}

#[cfg(test)]
pub(crate) fn serialized_request_for_test(input: &SpeechInput) -> Vec<u8> {
    serde_json::to_vec(&OpenAiSpeechRequest::from(input)).unwrap_or_default()
}
