use std::{future::Future, pin::Pin, time::Duration};

use crate::storage::SecretValue;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST_BYTES: usize = 512 * 1024;
pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub type ResponsesFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Closed POST request created only by the LLM adapter. It deliberately does
/// not implement `Debug` because its body contains minimized user context.
pub struct ResponsesHttpRequest {
    endpoint: String,
    body: Vec<u8>,
}

impl ResponsesHttpRequest {
    pub(super) fn new(endpoint: String, body: Vec<u8>) -> Result<Self, ResponsesTransportError> {
        if body.is_empty() || body.len() > MAX_REQUEST_BYTES {
            return Err(ResponsesTransportError::InvalidResponse);
        }
        Ok(Self { endpoint, body })
    }

    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// Bounded response projection. Non-success responses must carry an empty body.
/// It deliberately does not implement `Debug` to discourage body logging.
pub struct ResponsesHttpResponse {
    status: u16,
    body: Vec<u8>,
}

impl ResponsesHttpResponse {
    /// Builds a fake/custom transport response while preserving the production
    /// response-size and non-success-body rules.
    ///
    /// # Errors
    ///
    /// Returns `InvalidResponse` for an oversized body or a non-empty error body.
    pub fn new(status: u16, body: Vec<u8>) -> Result<Self, ResponsesTransportError> {
        if body.len() > MAX_RESPONSE_BYTES || (!(200..=299).contains(&status) && !body.is_empty()) {
            return Err(ResponsesTransportError::InvalidResponse);
        }
        Ok(Self { status, body })
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponsesTransportError {
    Timeout,
    Unavailable,
    InvalidResponse,
}

/// Injected stateless Responses transport. Implementations receive a
/// zeroizing secret separately from the body and must never retain either.
pub trait ResponsesTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        request: ResponsesHttpRequest,
        secret: &'a SecretValue,
        timeout: Duration,
    ) -> ResponsesFuture<'a, Result<ResponsesHttpResponse, ResponsesTransportError>>;
}

/// Backend-only HTTPS implementation. Construction performs no network call.
pub struct ReqwestResponsesTransport {
    client: reqwest::Client,
}

impl ReqwestResponsesTransport {
    /// Creates a rustls-only, redirect-disabled HTTP client.
    ///
    /// # Errors
    ///
    /// Returns `Unavailable` if the client cannot be constructed.
    pub fn new() -> Result<Self, ResponsesTransportError> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("CyberKindred/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ResponsesTransportError::Unavailable)?;
        Ok(Self { client })
    }
}

impl ResponsesTransport for ReqwestResponsesTransport {
    fn send<'a>(
        &'a self,
        request: ResponsesHttpRequest,
        secret: &'a SecretValue,
        timeout: Duration,
    ) -> ResponsesFuture<'a, Result<ResponsesHttpResponse, ResponsesTransportError>> {
        Box::pin(async move {
            let builder = self
                .client
                .post(request.endpoint)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request.body);
            let builder = secret.with_exposed(|value| builder.bearer_auth(value).timeout(timeout));
            let mut response = builder
                .send()
                .await
                .map_err(|error| map_reqwest_error(&error))?;
            let status = response.status().as_u16();
            if !(200..=299).contains(&status) {
                return ResponsesHttpResponse::new(status, Vec::new());
            }
            let body = read_bounded_body(&mut response).await?;
            ResponsesHttpResponse::new(status, body)
        })
    }
}

async fn read_bounded_body(
    response: &mut reqwest::Response,
) -> Result<Vec<u8>, ResponsesTransportError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ResponsesTransportError::InvalidResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| map_reqwest_error(&error))?
    {
        let next_length = body
            .len()
            .checked_add(chunk.len())
            .ok_or(ResponsesTransportError::InvalidResponse)?;
        if next_length > MAX_RESPONSE_BYTES {
            return Err(ResponsesTransportError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn map_reqwest_error(error: &reqwest::Error) -> ResponsesTransportError {
    if error.is_timeout() {
        ResponsesTransportError::Timeout
    } else {
        ResponsesTransportError::Unavailable
    }
}
