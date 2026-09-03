use std::{future::Future, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::{ContractRegistry, ProgramPlan},
    program::{ProgramFuture, ProgramPlanProvider, ProgramProviderError, ProviderProgramInput},
    providers::{CancellationFlag, ProviderCallContext},
    storage::{CanonicalOrigin, SecretValue},
};

use super::{
    context::{
        ContextBuilder, MAX_PROVIDER_INPUT_TOKENS, MAX_PROVIDER_OUTPUT_TOKENS,
        ProgramContextSource, ResponseInputMessage,
    },
    transport::{
        ReqwestResponsesTransport, ResponsesHttpRequest, ResponsesHttpResponse, ResponsesTransport,
        ResponsesTransportError,
    },
};

const RESPONSES_PATH: &str = "/v1/responses";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_MODEL_ID_CHARS: usize = 512;
const PROGRAM_PLAN_SCHEMA: &str =
    include_str!("../../../docs/contracts/schemas/program-plan.schema.json");

/// Current origin/model/secret bundle loaded at call time. It deliberately
/// implements neither `Clone` nor `Debug` because it owns secret material.
pub struct OpenAiProgramCredential {
    origin: CanonicalOrigin,
    model_id: String,
    secret: SecretValue,
}

impl OpenAiProgramCredential {
    /// Builds a call-scoped BYOK bundle from already-authorized settings and
    /// credential storage values.
    ///
    /// # Errors
    ///
    /// Returns `InvalidResponse` for an unsafe or empty model identifier.
    pub fn new(
        origin: CanonicalOrigin,
        model_id: String,
        secret: SecretValue,
    ) -> Result<Self, ProgramProviderError> {
        validate_model_id(&model_id)?;
        Ok(Self {
            origin,
            model_id,
            secret,
        })
    }

    pub(super) const fn origin(&self) -> &CanonicalOrigin {
        &self.origin
    }

    pub(super) fn model_id(&self) -> &str {
        &self.model_id
    }

    pub(super) const fn secret(&self) -> &SecretValue {
        &self.secret
    }
}

/// Loads the current verified origin/model and its exact origin-scoped BYOK
/// credential. The source must not cache or log the returned secret.
pub trait ProgramCredentialSource: Send + Sync {
    fn load(&self) -> ProgramFuture<'_, Result<OpenAiProgramCredential, ProgramProviderError>>;
}

/// Supplies one deadline and cancellation token for a program-planning call.
/// The provider always caps the returned deadline to sixty seconds.
pub trait ProgramCallContextFactory: Send + Sync {
    fn create(&self, program_id: &str) -> ProviderCallContext;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProgramCallContextFactory;

impl ProgramCallContextFactory for SystemProgramCallContextFactory {
    fn create(&self, _program_id: &str) -> ProviderCallContext {
        ProviderCallContext::new(CALL_TIMEOUT)
    }
}

/// Stateless `OpenAI` Responses implementation of the domain planning boundary.
/// It never persists provider bodies and has no playback or storage authority.
pub struct OpenAiProgramPlanProvider {
    credentials: Arc<dyn ProgramCredentialSource>,
    context_source: Arc<dyn ProgramContextSource>,
    call_contexts: Arc<dyn ProgramCallContextFactory>,
    transport: Arc<dyn ResponsesTransport>,
}

impl OpenAiProgramPlanProvider {
    /// Creates the production rustls transport. Construction is offline and
    /// performs no credential read or provider request.
    ///
    /// # Errors
    ///
    /// Returns `Unavailable` if the HTTPS client cannot be constructed.
    pub fn new(
        credentials: Arc<dyn ProgramCredentialSource>,
        context_source: Arc<dyn ProgramContextSource>,
        call_contexts: Arc<dyn ProgramCallContextFactory>,
    ) -> Result<Self, ProgramProviderError> {
        let transport = ReqwestResponsesTransport::new().map_err(map_transport_error)?;
        Ok(Self::with_transport(
            credentials,
            context_source,
            call_contexts,
            Arc::new(transport),
        ))
    }

    #[must_use]
    pub fn with_transport(
        credentials: Arc<dyn ProgramCredentialSource>,
        context_source: Arc<dyn ProgramContextSource>,
        call_contexts: Arc<dyn ProgramCallContextFactory>,
        transport: Arc<dyn ResponsesTransport>,
    ) -> Self {
        Self {
            credentials,
            context_source,
            call_contexts,
            transport,
        }
    }

    async fn generate(
        &self,
        input: &ProviderProgramInput,
    ) -> Result<ProgramPlan, ProgramProviderError> {
        let supplied_context = self.call_contexts.create(&input.program_id);
        let context = cap_call_context(supplied_context);
        ensure_active(&context)?;
        let extras = await_stage(self.context_source.load(input), &context).await?;
        let first_context = ContextBuilder::build(input, extras.clone(), false)?;
        if first_context.estimated_input_tokens > MAX_PROVIDER_INPUT_TOKENS {
            return Err(ProgramProviderError::InvalidResponse);
        }
        let credential = await_stage(self.credentials.load(), &context).await?;
        let first = self
            .send_request(&credential, first_context.input, &context)
            .await?;
        let output = extract_output_text(&first)?;
        if let Ok(plan) = parse_structured_program(&output) {
            return Ok(plan);
        }
        ensure_active(&context)?;
        let repair_context = ContextBuilder::build(input, extras, true)?;
        if repair_context.estimated_input_tokens > MAX_PROVIDER_INPUT_TOKENS {
            return Err(ProgramProviderError::InvalidResponse);
        }
        let repaired = self
            .send_request(&credential, repair_context.input, &context)
            .await?;
        let repaired_output = extract_output_text(&repaired)?;
        parse_structured_program(&repaired_output)
            .map_err(|()| ProgramProviderError::InvalidResponse)
    }

    async fn send_request(
        &self,
        credential: &OpenAiProgramCredential,
        input: Vec<ResponseInputMessage>,
        context: &ProviderCallContext,
    ) -> Result<ResponsesHttpResponse, ProgramProviderError> {
        ensure_active(context)?;
        let schema: serde_json::Value = serde_json::from_str(PROGRAM_PLAN_SCHEMA)
            .map_err(|_| ProgramProviderError::InvalidResponse)?;
        let body = serde_json::to_vec(&ResponsesRequest {
            model: &credential.model_id,
            store: false,
            background: false,
            input,
            text: TextConfiguration {
                format: JsonSchemaFormat {
                    r#type: "json_schema",
                    name: "cyberkindred_program_plan_v1",
                    strict: true,
                    schema,
                },
            },
            tools: [],
            tool_choice: "none",
            parallel_tool_calls: false,
            max_output_tokens: MAX_PROVIDER_OUTPUT_TOKENS,
            truncation: "disabled",
        })
        .map_err(|_| ProgramProviderError::InvalidResponse)?;
        let endpoint = format!("{}{RESPONSES_PATH}", credential.origin.as_str());
        let request = ResponsesHttpRequest::new(endpoint, body).map_err(map_transport_error)?;
        let remaining = remaining(context)?;
        let transport = self.transport.send(request, &credential.secret, remaining);
        tokio::pin!(transport);
        let result = tokio::select! {
            result = &mut transport => result.map_err(map_transport_error),
            () = wait_for_cancellation(&context.cancellation) => {
                Err(ProgramProviderError::Cancelled)
            }
            () = tokio::time::sleep(remaining) => {
                Err(ProgramProviderError::Timeout)
            }
        }?;
        ensure_active(context)?;
        classify_status(result.status())?;
        Ok(result)
    }
}

impl ProgramPlanProvider for OpenAiProgramPlanProvider {
    fn generate_program<'a>(
        &'a self,
        input: &'a ProviderProgramInput,
    ) -> ProgramFuture<'a, Result<ProgramPlan, ProgramProviderError>> {
        Box::pin(async move { self.generate(input).await })
    }
}

async fn await_stage<T, F>(
    future: F,
    context: &ProviderCallContext,
) -> Result<T, ProgramProviderError>
where
    F: Future<Output = Result<T, ProgramProviderError>>,
{
    ensure_active(context)?;
    let remaining = remaining(context)?;
    tokio::pin!(future);
    let result = tokio::select! {
        result = &mut future => result,
        () = wait_for_cancellation(&context.cancellation) => {
            Err(ProgramProviderError::Cancelled)
        }
        () = tokio::time::sleep(remaining) => {
            Err(ProgramProviderError::Timeout)
        }
    }?;
    ensure_active(context)?;
    Ok(result)
}

fn cap_call_context(context: ProviderCallContext) -> ProviderCallContext {
    ProviderCallContext {
        correlation_id: context.correlation_id,
        locale: context.locale,
        deadline: context
            .deadline
            .min(std::time::Instant::now() + CALL_TIMEOUT),
        cancellation: context.cancellation,
    }
}

fn ensure_active(context: &ProviderCallContext) -> Result<(), ProgramProviderError> {
    if context.cancellation.is_cancelled() {
        Err(ProgramProviderError::Cancelled)
    } else if remaining(context).is_err() {
        Err(ProgramProviderError::Timeout)
    } else {
        Ok(())
    }
}

fn remaining(context: &ProviderCallContext) -> Result<Duration, ProgramProviderError> {
    context
        .deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(ProgramProviderError::Timeout)
}

async fn wait_for_cancellation(cancellation: &CancellationFlag) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}

#[derive(Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    store: bool,
    background: bool,
    input: Vec<ResponseInputMessage>,
    text: TextConfiguration,
    tools: [serde_json::Value; 0],
    tool_choice: &'static str,
    parallel_tool_calls: bool,
    max_output_tokens: u16,
    truncation: &'static str,
}

#[derive(Serialize)]
struct TextConfiguration {
    format: JsonSchemaFormat,
}

#[derive(Serialize)]
struct JsonSchemaFormat {
    r#type: &'static str,
    name: &'static str,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Deserialize)]
struct ResponsesEnvelope {
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

fn extract_output_text(response: &ResponsesHttpResponse) -> Result<String, ProgramProviderError> {
    let parsed: ResponsesEnvelope = serde_json::from_slice(response.body())
        .map_err(|_| ProgramProviderError::InvalidResponse)?;
    if parsed.object != "response" || parsed.status != "completed" {
        return Err(ProgramProviderError::InvalidResponse);
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
            return Err(ProgramProviderError::InvalidResponse);
        }
    }
    let message = assistant_message.ok_or(ProgramProviderError::InvalidResponse)?;
    let Some([content]) = message.content.as_deref() else {
        return Err(ProgramProviderError::InvalidResponse);
    };
    if content.r#type != "output_text" {
        return Err(ProgramProviderError::InvalidResponse);
    }
    content
        .text
        .clone()
        .ok_or(ProgramProviderError::InvalidResponse)
}

fn parse_structured_program(text: &str) -> Result<ProgramPlan, ()> {
    let plan: ProgramPlan = serde_json::from_str(text).map_err(|_| ())?;
    let document = serde_json::to_value(&plan).map_err(|_| ())?;
    ContractRegistry::new()
        .map_err(|_| ())?
        .validate("program-plan", &document)
        .map_err(|_| ())?;
    Ok(plan)
}

fn validate_model_id(model_id: &str) -> Result<(), ProgramProviderError> {
    if model_id.is_empty()
        || model_id.chars().count() > MAX_MODEL_ID_CHARS
        || model_id.trim() != model_id
        || model_id.chars().any(char::is_control)
    {
        Err(ProgramProviderError::InvalidResponse)
    } else {
        Ok(())
    }
}

fn classify_status(status: u16) -> Result<(), ProgramProviderError> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(ProgramProviderError::Authentication),
        429 => Err(ProgramProviderError::RateLimited),
        300..=399 | 500..=599 => Err(ProgramProviderError::Unavailable),
        _ => Err(ProgramProviderError::InvalidResponse),
    }
}

const fn map_transport_error(error: ResponsesTransportError) -> ProgramProviderError {
    match error {
        ResponsesTransportError::Timeout => ProgramProviderError::Timeout,
        ResponsesTransportError::Unavailable => ProgramProviderError::Unavailable,
        ResponsesTransportError::InvalidResponse => ProgramProviderError::InvalidResponse,
    }
}
