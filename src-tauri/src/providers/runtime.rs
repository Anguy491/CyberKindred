//! Production-safe provider adapters used by the M2 configuration surface.
//!
//! Construction and catalog reads are side-effect free. The only network path
//! in this module is an explicit `OpenAI` connection probe. Candidate-secret
//! validation is restricted to `GET /v1/models`; an explicit LLM test uses one
//! stateless `POST /v1/responses` structured-output request. Audio generation
//! and playback remain unavailable until their M3 adapters are present.

use super::{
    CandidateSecretValidator, ProviderCallContext, ProviderFailure, ProviderFailureCategory,
    ProviderHealthProbe, ProviderTestInput, SecretValidationInput, VoicePreviewInput,
    VoicePreviewer, VoiceView,
};
use crate::storage::{CanonicalOrigin, SecretValue};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

use super::traits::ProviderFuture;

const OPENAI_MODELS_PATH: &str = "/v1/models";
const OPENAI_RESPONSES_PATH: &str = "/v1/responses";
const LLM_PROBE_INPUT: &str = "Return the fixed provider readiness object.";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_PROVIDER_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_MODEL_COUNT: usize = 4_096;
const MAX_MODEL_ID_CHARS: usize = 512;
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// Candidate-secret validator, explicit provider health probe, and safe M2
/// voice placeholder backed by a single locked-down HTTP client.
pub struct ProviderRuntime {
    transport: Arc<dyn ProviderTransport>,
}

impl ProviderRuntime {
    /// Builds the backend-only HTTPS client without making a network request.
    ///
    /// # Errors
    ///
    /// Returns an unavailable provider failure if the TLS client cannot be
    /// constructed. No upstream error detail is retained or exposed.
    pub fn new() -> Result<Self, ProviderFailure> {
        let transport = ReqwestProviderTransport::new()?;
        Ok(Self {
            transport: Arc::new(transport),
        })
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn ProviderTransport>) -> Self {
        Self { transport }
    }

    async fn probe_models(
        &self,
        origin: &CanonicalOrigin,
        secret: &SecretValue,
        context: &ProviderCallContext,
    ) -> Result<u64, ProviderFailure> {
        let request = ProviderHttpRequest {
            method: HttpMethod::Get,
            endpoint: fixed_endpoint(origin, OPENAI_MODELS_PATH),
            body: None,
        };
        let (response, latency_ms) = self.send_once(request, secret, context).await?;
        validate_success_status(&response)?;
        validate_models_body(&response.body)?;
        Ok(latency_ms)
    }

    async fn probe_responses(
        &self,
        origin: &CanonicalOrigin,
        secret: &SecretValue,
        model_id: &str,
        context: &ProviderCallContext,
    ) -> Result<u64, ProviderFailure> {
        validate_model_id(model_id)?;
        let body = serde_json::to_vec(&LlmProbeRequest::new(model_id))
            .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
        let request = ProviderHttpRequest {
            method: HttpMethod::Post,
            endpoint: fixed_endpoint(origin, OPENAI_RESPONSES_PATH),
            body: Some(body),
        };
        let (response, latency_ms) = self.send_once(request, secret, context).await?;
        validate_success_status(&response)?;
        validate_responses_body(&response.body)?;
        Ok(latency_ms)
    }

    async fn send_once(
        &self,
        request: ProviderHttpRequest,
        secret: &SecretValue,
        context: &ProviderCallContext,
    ) -> Result<(ProviderTransportResponse, u64), ProviderFailure> {
        if context.cancellation.is_cancelled() {
            return Err(cancelled_failure());
        }
        let Some(remaining) = context
            .deadline
            .checked_duration_since(std::time::Instant::now())
        else {
            return Err(ProviderFailure::new(ProviderFailureCategory::Timeout));
        };
        if remaining.is_zero() {
            return Err(ProviderFailure::new(ProviderFailureCategory::Timeout));
        }

        let started_at = std::time::Instant::now();
        let transport_request = self.transport.send(request, secret, remaining);
        tokio::pin!(transport_request);
        let transport_result = tokio::select! {
            result = &mut transport_request => result,
            () = wait_for_cancellation(&context.cancellation) => {
                return Err(cancelled_failure());
            }
            () = tokio::time::sleep(remaining) => {
                return Err(ProviderFailure::new(ProviderFailureCategory::Timeout));
            }
        };
        if context.cancellation.is_cancelled() {
            return Err(cancelled_failure());
        }
        let response = transport_result.map_err(TransportFailure::into_provider_failure)?;
        Ok((response, duration_millis(started_at.elapsed())))
    }
}

impl CandidateSecretValidator for ProviderRuntime {
    fn validate<'a>(
        &'a self,
        input: SecretValidationInput<'a>,
        context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<(), ProviderFailure>> {
        Box::pin(async move {
            self.probe_models(input.origin, input.candidate, context)
                .await
                .map(|_| ())
        })
    }
}

impl ProviderHealthProbe for ProviderRuntime {
    fn test<'a>(
        &'a self,
        input: ProviderTestInput<'a>,
        context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<u64, ProviderFailure>> {
        Box::pin(async move {
            match input {
                ProviderTestInput::OpenAi {
                    kind: super::ProviderTestKind::Llm,
                    origin,
                    secret,
                    model_id,
                } => {
                    self.probe_responses(origin, secret, model_id, context)
                        .await
                }
                ProviderTestInput::OpenAi { .. }
                | ProviderTestInput::Metadata
                | ProviderTestInput::Weather => {
                    Err(ProviderFailure::new(ProviderFailureCategory::Unavailable))
                }
            }
        })
    }
}

impl VoicePreviewer for ProviderRuntime {
    fn voices(&self) -> Vec<VoiceView> {
        vec![VoiceView {
            voice_id: "alloy".to_owned(),
            display_name: "Alloy".to_owned(),
            preview_available: false,
        }]
    }

    fn preview<'a>(
        &'a self,
        _input: VoicePreviewInput<'a>,
        _context: &'a ProviderCallContext,
    ) -> ProviderFuture<'a, Result<(), ProviderFailure>> {
        Box::pin(async { Err(ProviderFailure::new(ProviderFailureCategory::Unavailable)) })
    }
}

async fn wait_for_cancellation(cancellation: &super::CancellationFlag) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}

const fn cancelled_failure() -> ProviderFailure {
    // ProviderError v1 has no cancellation category. This path performs no
    // persistence or retry and uses the generic temporary-unavailability class.
    ProviderFailure::new(ProviderFailureCategory::Unavailable)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderHttpRequest {
    method: HttpMethod,
    endpoint: String,
    body: Option<Vec<u8>>,
}

trait ProviderTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        request: ProviderHttpRequest,
        secret: &'a SecretValue,
        timeout: Duration,
    ) -> ProviderFuture<'a, Result<ProviderTransportResponse, TransportFailure>>;
}

struct ReqwestProviderTransport {
    client: reqwest::Client,
}

impl ReqwestProviderTransport {
    fn new() -> Result<Self, ProviderFailure> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("CyberKindred/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderFailure::new(ProviderFailureCategory::Unavailable))?;
        Ok(Self { client })
    }
}

impl ProviderTransport for ReqwestProviderTransport {
    fn send<'a>(
        &'a self,
        request: ProviderHttpRequest,
        secret: &'a SecretValue,
        timeout: Duration,
    ) -> ProviderFuture<'a, Result<ProviderTransportResponse, TransportFailure>> {
        Box::pin(async move {
            let mut builder = match request.method {
                HttpMethod::Get => self.client.get(request.endpoint),
                HttpMethod::Post => self.client.post(request.endpoint),
            };
            if let Some(body) = request.body {
                builder = builder
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body);
            }
            let builder = secret.with_exposed(|value| builder.bearer_auth(value).timeout(timeout));
            let mut response = builder
                .send()
                .await
                .map_err(|error| map_reqwest_error(&error))?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned);
            let body = if response.status().is_success() {
                read_bounded_body(&mut response).await?
            } else {
                // Upstream error bodies may contain echoed request material and
                // are neither read nor retained.
                Vec::new()
            };
            Ok(ProviderTransportResponse {
                status,
                retry_after,
                body,
            })
        })
    }
}

async fn read_bounded_body(response: &mut reqwest::Response) -> Result<Vec<u8>, TransportFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64)
    {
        return Err(TransportFailure::InvalidResponse);
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
            .ok_or(TransportFailure::InvalidResponse)?;
        if next_length > MAX_PROVIDER_RESPONSE_BYTES {
            return Err(TransportFailure::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn map_reqwest_error(error: &reqwest::Error) -> TransportFailure {
    if error.is_timeout() {
        TransportFailure::Timeout
    } else {
        TransportFailure::Unavailable
    }
}

#[derive(Clone)]
struct ProviderTransportResponse {
    status: u16,
    retry_after: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone, Copy)]
enum TransportFailure {
    Timeout,
    Unavailable,
    InvalidResponse,
}

impl TransportFailure {
    const fn into_provider_failure(self) -> ProviderFailure {
        match self {
            Self::Timeout => ProviderFailure::new(ProviderFailureCategory::Timeout),
            Self::Unavailable => ProviderFailure::new(ProviderFailureCategory::Unavailable),
            Self::InvalidResponse => ProviderFailure::new(ProviderFailureCategory::InvalidResponse),
        }
    }
}

#[derive(Deserialize)]
struct ModelsResponse {
    object: String,
    data: Vec<ModelDescriptor>,
}

#[derive(Deserialize)]
struct ModelDescriptor {
    id: String,
    object: String,
}

#[derive(Serialize)]
struct LlmProbeRequest<'a> {
    model: &'a str,
    store: bool,
    input: &'static str,
    reasoning: ProbeReasoningConfiguration,
    text: ProbeTextConfiguration,
    max_output_tokens: u16,
}

impl<'a> LlmProbeRequest<'a> {
    fn new(model: &'a str) -> Self {
        Self {
            model,
            store: false,
            input: LLM_PROBE_INPUT,
            reasoning: ProbeReasoningConfiguration { effort: "none" },
            text: ProbeTextConfiguration {
                format: ProbeJsonSchemaFormat {
                    r#type: "json_schema",
                    name: "cyberkindred_provider_probe_v1",
                    strict: true,
                    schema: ProbeJsonSchema {
                        r#type: "object",
                        properties: ProbeJsonSchemaProperties {
                            ready: ProbeReadySchema {
                                r#type: "boolean",
                                r#enum: [true],
                            },
                        },
                        required: ["ready"],
                        additional_properties: false,
                    },
                },
            },
            max_output_tokens: 32,
        }
    }
}

#[derive(Serialize)]
struct ProbeReasoningConfiguration {
    effort: &'static str,
}

#[derive(Serialize)]
struct ProbeTextConfiguration {
    format: ProbeJsonSchemaFormat,
}

#[derive(Serialize)]
struct ProbeJsonSchemaFormat {
    r#type: &'static str,
    name: &'static str,
    strict: bool,
    schema: ProbeJsonSchema,
}

#[derive(Serialize)]
struct ProbeJsonSchema {
    r#type: &'static str,
    properties: ProbeJsonSchemaProperties,
    required: [&'static str; 1],
    #[serde(rename = "additionalProperties")]
    additional_properties: bool,
}

#[derive(Serialize)]
struct ProbeJsonSchemaProperties {
    ready: ProbeReadySchema,
}

#[derive(Serialize)]
struct ProbeReadySchema {
    r#type: &'static str,
    #[serde(rename = "enum")]
    r#enum: [bool; 1],
}

#[derive(Deserialize)]
struct ResponsesProbeResponse {
    object: String,
    status: String,
    output: Vec<ResponsesOutputItem>,
}

#[derive(Deserialize)]
struct ResponsesOutputItem {
    r#type: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    content: Option<Vec<ResponsesContentItem>>,
}

#[derive(Deserialize)]
struct ResponsesContentItem {
    r#type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeReadiness {
    ready: bool,
}

fn fixed_endpoint(origin: &CanonicalOrigin, path: &'static str) -> String {
    format!("{}{path}", origin.as_str())
}

fn validate_success_status(response: &ProviderTransportResponse) -> Result<(), ProviderFailure> {
    match response.status {
        200..=299 => Ok(()),
        401 | 403 => Err(ProviderFailure::new(
            ProviderFailureCategory::Authentication,
        )),
        429 => Err(ProviderFailure::rate_limited(
            response
                .retry_after
                .as_deref()
                .and_then(parse_retry_after_ms),
        )),
        300..=399 | 500..=599 => Err(ProviderFailure::new(ProviderFailureCategory::Unavailable)),
        _ => Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        )),
    }
}

fn validate_models_body(body: &[u8]) -> Result<(), ProviderFailure> {
    if body.len() > MAX_PROVIDER_RESPONSE_BYTES {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    let parsed: ModelsResponse = serde_json::from_slice(body)
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    if parsed.object != "list"
        || parsed.data.is_empty()
        || parsed.data.len() > MAX_MODEL_COUNT
        || parsed.data.iter().any(|model| {
            model.object != "model"
                || model.id.is_empty()
                || model.id.chars().count() > MAX_MODEL_ID_CHARS
                || model.id.trim() != model.id
                || model.id.chars().any(char::is_control)
        })
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    Ok(())
}

fn validate_model_id(model_id: &str) -> Result<(), ProviderFailure> {
    if model_id.is_empty()
        || model_id.chars().count() > MAX_MODEL_ID_CHARS
        || model_id.trim() != model_id
        || model_id.chars().any(char::is_control)
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    Ok(())
}

fn validate_responses_body(body: &[u8]) -> Result<(), ProviderFailure> {
    if body.len() > MAX_PROVIDER_RESPONSE_BYTES {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    let parsed: ResponsesProbeResponse = serde_json::from_slice(body)
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    if parsed.object != "response" || parsed.status != "completed" {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }

    let mut assistant_message = None;
    for output in &parsed.output {
        if output.r#type == "reasoning" {
            continue;
        }
        if output.r#type != "message"
            || output.role.as_deref() != Some("assistant")
            || output.status.as_deref() != Some("completed")
            || assistant_message.replace(output).is_some()
        {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidResponse,
            ));
        }
    }
    let output = assistant_message
        .ok_or_else(|| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    let Some([content]) = output.content.as_deref() else {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    };
    if content.r#type != "output_text" {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    let text = content
        .text
        .as_deref()
        .ok_or_else(|| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    let readiness: ProbeReadiness = serde_json::from_str(text)
        .map_err(|_| ProviderFailure::new(ProviderFailureCategory::InvalidResponse))?;
    if !readiness.ready {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::InvalidResponse,
        ));
    }
    Ok(())
}

fn parse_retry_after_ms(value: &str) -> Option<u64> {
    let seconds = value.trim().parse::<u64>().ok()?;
    if seconds <= MAX_SAFE_JSON_INTEGER / 1_000 {
        Some(seconds * 1_000)
    } else {
        None
    }
}

fn duration_millis(duration: Duration) -> u64 {
    let millis = duration.as_millis();
    if millis > u128::from(u64::MAX) {
        u64::MAX
    } else {
        #[allow(clippy::cast_possible_truncation)]
        {
            millis as u64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderTestKind;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    const VALID_MODELS: &[u8] = br#"{
        "object":"list",
        "data":[{
            "id":"gpt-test",
            "object":"model",
            "created":1,
            "owned_by":"fixture"
        }]
    }"#;

    const VALID_RESPONSE: &[u8] = br#"{
        "object":"response",
        "status":"completed",
        "output":[{
            "type":"message",
            "role":"assistant",
            "status":"completed",
            "content":[{
                "type":"output_text",
                "text":"{\"ready\":true}",
                "annotations":[]
            }]
        }]
    }"#;

    const VALID_RESPONSE_WITH_REASONING: &[u8] = br#"{
        "object":"response",
        "status":"completed",
        "output":[
            {
                "type":"reasoning",
                "id":"reasoning-fixture",
                "summary":[]
            },
            {
                "type":"message",
                "role":"assistant",
                "status":"completed",
                "content":[{
                    "type":"output_text",
                    "text":"{\"ready\":true}",
                    "annotations":[]
                }]
            }
        ]
    }"#;

    struct FakeTransport {
        calls: AtomicUsize,
        requests: Mutex<Vec<ProviderHttpRequest>>,
        result: Result<ProviderTransportResponse, TransportFailure>,
    }

    impl FakeTransport {
        fn response(status: u16, retry_after: Option<&str>, body: &[u8]) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
                result: Ok(ProviderTransportResponse {
                    status,
                    retry_after: retry_after.map(ToOwned::to_owned),
                    body: body.to_vec(),
                }),
            })
        }

        fn failure(failure: TransportFailure) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
                result: Err(failure),
            })
        }
    }

    impl ProviderTransport for FakeTransport {
        fn send<'a>(
            &'a self,
            request: ProviderHttpRequest,
            secret: &'a SecretValue,
            _timeout: Duration,
        ) -> ProviderFuture<'a, Result<ProviderTransportResponse, TransportFailure>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.requests
                .lock()
                .expect("fake request lock")
                .push(request);
            assert!(secret.with_exposed(|value| !value.is_empty()));
            let result = self.result.clone();
            Box::pin(async move { result })
        }
    }

    struct PendingTransport {
        calls: AtomicUsize,
    }

    impl ProviderTransport for PendingTransport {
        fn send<'a>(
            &'a self,
            _request: ProviderHttpRequest,
            _secret: &'a SecretValue,
            _timeout: Duration,
        ) -> ProviderFuture<'a, Result<ProviderTransportResponse, TransportFailure>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(std::future::pending())
        }
    }

    fn origin() -> CanonicalOrigin {
        CanonicalOrigin::parse("https://api.openai.example").expect("canonical fixture origin")
    }

    fn secret() -> SecretValue {
        SecretValue::new("ck-provider-runtime-canary".to_owned()).expect("valid fixture secret")
    }

    #[tokio::test]
    async fn provider_config_llm_probe_uses_one_fixed_structured_request() {
        let fake = FakeTransport::response(200, None, VALID_RESPONSE);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let origin = origin();
        let secret = secret();
        let result = runtime
            .test(
                ProviderTestInput::OpenAi {
                    kind: ProviderTestKind::Llm,
                    origin: &origin,
                    secret: &secret,
                    model_id: "gpt-test-model",
                },
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
        let requests = fake.requests.lock().expect("fake request lock");
        let [request] = requests.as_slice() else {
            panic!("exactly one request expected");
        };
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.endpoint, "https://api.openai.example/v1/responses");
        let body: serde_json::Value =
            serde_json::from_slice(request.body.as_deref().expect("JSON body"))
                .expect("valid request JSON");
        assert_eq!(body["model"], "gpt-test-model");
        assert_eq!(body["store"], false);
        assert_eq!(body["input"], LLM_PROBE_INPUT);
        assert_eq!(body["reasoning"]["effort"], "none");
        assert!(body.get("tools").is_none());
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(
            body["text"]["format"]["schema"]["additionalProperties"],
            false
        );
        assert_eq!(
            body["text"]["format"]["schema"]["properties"]["ready"]["enum"],
            serde_json::json!([true])
        );
    }

    #[tokio::test]
    async fn provider_config_llm_probe_accepts_reasoning_before_the_unique_message() {
        let fake = FakeTransport::response(200, None, VALID_RESPONSE_WITH_REASONING);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let result = runtime
            .probe_responses(
                &origin(),
                &secret(),
                "gpt-test-model",
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn provider_config_candidate_validation_uses_the_same_minimal_probe() {
        let fake = FakeTransport::response(200, None, VALID_MODELS);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let origin = origin();
        let secret = secret();
        let result = runtime
            .validate(
                SecretValidationInput {
                    origin: &origin,
                    candidate: &secret,
                },
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert_eq!(result, Ok(()));
        assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
        let requests = fake.requests.lock().expect("fake request lock");
        let [request] = requests.as_slice() else {
            panic!("exactly one request expected");
        };
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.endpoint, "https://api.openai.example/v1/models");
        assert_eq!(request.body, None);
    }

    #[tokio::test]
    async fn provider_config_classifies_status_without_reading_error_bodies() {
        for (status, retry_after, expected) in [
            (
                401,
                None,
                ProviderFailure::new(ProviderFailureCategory::Authentication),
            ),
            (
                403,
                None,
                ProviderFailure::new(ProviderFailureCategory::Authentication),
            ),
            (429, Some("7"), ProviderFailure::rate_limited(Some(7_000))),
            (
                503,
                None,
                ProviderFailure::new(ProviderFailureCategory::Unavailable),
            ),
        ] {
            let fake = FakeTransport::response(
                status,
                retry_after,
                b"ck-provider-error-body-must-not-escape",
            );
            let runtime = ProviderRuntime::with_transport(fake.clone());
            let origin = origin();
            let secret = secret();
            let result = runtime
                .probe_responses(
                    &origin,
                    &secret,
                    "gpt-test-model",
                    &ProviderCallContext::new(Duration::from_secs(1)),
                )
                .await;
            assert_eq!(result, Err(expected));
            assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
            assert!(!format!("{result:?}").contains("must-not-escape"));
        }
    }

    #[tokio::test]
    async fn provider_config_classifies_transport_failures_without_retry() {
        for (transport_failure, category) in [
            (TransportFailure::Timeout, ProviderFailureCategory::Timeout),
            (
                TransportFailure::Unavailable,
                ProviderFailureCategory::Unavailable,
            ),
        ] {
            let fake = FakeTransport::failure(transport_failure);
            let runtime = ProviderRuntime::with_transport(fake.clone());
            let result = runtime
                .probe_responses(
                    &origin(),
                    &secret(),
                    "gpt-test-model",
                    &ProviderCallContext::new(Duration::from_secs(1)),
                )
                .await;
            assert_eq!(result.map_err(|failure| failure.category), Err(category));
            assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
        }
    }

    #[tokio::test]
    async fn provider_config_deadline_cancels_a_pending_request() {
        let fake = Arc::new(PendingTransport {
            calls: AtomicUsize::new(0),
        });
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let result = runtime
            .probe_responses(
                &origin(),
                &secret(),
                "gpt-test-model",
                &ProviderCallContext::new(Duration::from_millis(5)),
            )
            .await;
        assert_eq!(
            result.map_err(|failure| failure.category),
            Err(ProviderFailureCategory::Timeout)
        );
        assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn provider_config_cancelled_context_makes_no_request() {
        let fake = FakeTransport::response(200, None, VALID_MODELS);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let context = ProviderCallContext::new(Duration::from_secs(1));
        context.cancellation.cancel();
        let result = runtime.probe_models(&origin(), &secret(), &context).await;
        assert_eq!(
            result.map_err(|failure| failure.category),
            Err(ProviderFailureCategory::Unavailable)
        );
        assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn provider_config_rejects_invalid_or_oversized_success_bodies() {
        for body in [
            br#"{"object":"list","data":[]}"#.as_slice(),
            br#"{"object":"wrong","data":[{"id":"x","object":"model"}]}"#.as_slice(),
            br#"{"object":"list","data":[{"id":"x","object":"wrong"}]}"#.as_slice(),
            b"not-json".as_slice(),
        ] {
            let runtime = ProviderRuntime::with_transport(FakeTransport::response(200, None, body));
            let result = runtime
                .probe_models(
                    &origin(),
                    &secret(),
                    &ProviderCallContext::new(Duration::from_secs(1)),
                )
                .await;
            assert_eq!(
                result.map_err(|failure| failure.category),
                Err(ProviderFailureCategory::InvalidResponse)
            );
        }

        let oversized = vec![b'x'; MAX_PROVIDER_RESPONSE_BYTES + 1];
        let runtime =
            ProviderRuntime::with_transport(FakeTransport::response(200, None, &oversized));
        let result = runtime
            .probe_models(
                &origin(),
                &secret(),
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert_eq!(
            result.map_err(|failure| failure.category),
            Err(ProviderFailureCategory::InvalidResponse)
        );
    }

    #[tokio::test]
    async fn provider_config_llm_probe_rejects_unproven_structured_output() {
        for body in [
            br#"{"object":"response","status":"in_progress","output":[]}"#.as_slice(),
            br#"{"object":"response","status":"completed","output":[]}"#.as_slice(),
            br#"{"object":"response","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","text":"no"}]}]}"#.as_slice(),
            br#"{"object":"response","status":"completed","output":[{"type":"reasoning","summary":[]},{"type":"message","role":"assistant","status":"incomplete","content":[{"type":"output_text","text":"{\"ready\":true}"}]}]}"#.as_slice(),
            br#"{"object":"response","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"{\"ready\":false}"}]}]}"#.as_slice(),
            br#"{"object":"response","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"{\"ready\":true,\"extra\":1}"}]}]}"#.as_slice(),
            br#"{"object":"response","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"{\"ready\":true}"}]},{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"{\"ready\":true}"}]}]}"#.as_slice(),
        ] {
            let fake = FakeTransport::response(200, None, body);
            let runtime = ProviderRuntime::with_transport(fake.clone());
            let result = runtime
                .probe_responses(
                    &origin(),
                    &secret(),
                    "gpt-test-model",
                    &ProviderCallContext::new(Duration::from_secs(1)),
                )
                .await;
            assert_eq!(
                result.map_err(|failure| failure.category),
                Err(ProviderFailureCategory::InvalidResponse)
            );
            assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
        }

        let fake = FakeTransport::response(200, None, VALID_RESPONSE);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let result = runtime
            .probe_responses(
                &origin(),
                &secret(),
                " invalid-model ",
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert_eq!(
            result.map_err(|failure| failure.category),
            Err(ProviderFailureCategory::InvalidResponse)
        );
        assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn provider_config_unimplemented_integrations_do_not_use_network() {
        let fake = FakeTransport::response(200, None, VALID_MODELS);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        for input in [ProviderTestInput::Metadata, ProviderTestInput::Weather] {
            let result = runtime
                .test(input, &ProviderCallContext::new(Duration::from_secs(1)))
                .await;
            assert_eq!(
                result.map_err(|failure| failure.category),
                Err(ProviderFailureCategory::Unavailable)
            );
        }
        assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn provider_config_tts_probe_fails_closed_without_using_llm_endpoint() {
        let fake = FakeTransport::response(200, None, VALID_MODELS);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        let origin = origin();
        let secret = secret();
        let result = runtime
            .test(
                ProviderTestInput::OpenAi {
                    kind: ProviderTestKind::Tts,
                    origin: &origin,
                    secret: &secret,
                    model_id: "gpt-4o-mini-tts",
                },
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert_eq!(
            result.map_err(|failure| failure.category),
            Err(ProviderFailureCategory::Unavailable)
        );
        assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn voice_preview_catalog_is_local_and_preview_is_unavailable() {
        let fake = FakeTransport::response(200, None, VALID_MODELS);
        let runtime = ProviderRuntime::with_transport(fake.clone());
        assert_eq!(
            runtime.voices(),
            vec![VoiceView {
                voice_id: "alloy".to_owned(),
                display_name: "Alloy".to_owned(),
                preview_available: false,
            }]
        );
        let origin = origin();
        let secret = secret();
        let result = runtime
            .preview(
                VoicePreviewInput {
                    origin: &origin,
                    secret: &secret,
                    model_id: "gpt-4o-mini-tts",
                    voice_id: "alloy",
                    text: super::super::PREVIEW_PHRASE_V1,
                },
                &ProviderCallContext::new(Duration::from_secs(1)),
            )
            .await;
        assert_eq!(
            result.map_err(|failure| failure.category),
            Err(ProviderFailureCategory::Unavailable)
        );
        assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn provider_config_retry_after_accepts_only_bounded_delta_seconds() {
        assert_eq!(parse_retry_after_ms(" 12 "), Some(12_000));
        assert_eq!(parse_retry_after_ms("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        assert_eq!(
            parse_retry_after_ms("9007199254740"),
            Some(9_007_199_254_740_000)
        );
        assert_eq!(parse_retry_after_ms("9007199254741"), None);
        assert_eq!(parse_retry_after_ms("18446744073709552"), None);
    }
}
