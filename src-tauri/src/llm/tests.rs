use std::{
    collections::{HashSet, VecDeque},
    future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    ContextTurn, ContextTurnRole, OpenAiProgramCredential, OpenAiProgramPlanProvider,
    ProgramCallContextFactory, ProgramContextExtras, ProgramContextSource, ProgramCredentialSource,
    ResponsesFuture, ResponsesHttpRequest, ResponsesHttpResponse, ResponsesTransport,
    ResponsesTransportError,
    context::{ContextBuilder, MAX_CONTEXT_CHARS, MAX_PROVIDER_INPUT_TOKENS},
};
use crate::{
    contracts::{
        ProgramPlan, ProgramPlanMode, ProgramPlanSegmentsItem, ProgramPlanTrackSegment,
        ProgramPlanVoiceSegment, ProgramPlanVoiceSegmentTrigger,
    },
    program::{
        ProgramCandidate, ProgramFuture, ProgramPlanProvider, ProgramProviderError,
        ProviderProgramInput,
    },
    providers::{CancellationFlag, ProviderCallContext},
    storage::{CanonicalOrigin, SecretValue},
};

const TEST_SECRET: &str = "sk-task016-hermetic-secret";

struct FakeCredentialSource {
    result: ProgramProviderError,
    succeed: bool,
}

impl FakeCredentialSource {
    const fn success() -> Self {
        Self {
            result: ProgramProviderError::Unavailable,
            succeed: true,
        }
    }

    const fn failure(result: ProgramProviderError) -> Self {
        Self {
            result,
            succeed: false,
        }
    }
}

impl ProgramCredentialSource for FakeCredentialSource {
    fn load(&self) -> ProgramFuture<'_, Result<OpenAiProgramCredential, ProgramProviderError>> {
        let result = self.result;
        let succeed = self.succeed;
        Box::pin(async move {
            if !succeed {
                return Err(result);
            }
            OpenAiProgramCredential::new(
                CanonicalOrigin::parse("https://api.openai.example")
                    .map_err(|_| ProgramProviderError::InvalidResponse)?,
                "gpt-task-016".to_owned(),
                SecretValue::new(TEST_SECRET.to_owned())
                    .map_err(|_| ProgramProviderError::InvalidResponse)?,
            )
        })
    }
}

struct FakeContextSource(ProgramContextExtras);

impl ProgramContextSource for FakeContextSource {
    fn load<'a>(
        &'a self,
        _input: &'a ProviderProgramInput,
    ) -> ProgramFuture<'a, Result<ProgramContextExtras, ProgramProviderError>> {
        let extras = self.0.clone();
        Box::pin(async move { Ok(extras) })
    }
}

struct FixedCallContextFactory {
    cancellation: CancellationFlag,
    deadline_offset: Duration,
    expired: bool,
}

impl ProgramCallContextFactory for FixedCallContextFactory {
    fn create(&self, _program_id: &str) -> ProviderCallContext {
        let now = Instant::now();
        ProviderCallContext {
            correlation_id: Uuid::now_v7(),
            locale: "zh-CN",
            deadline: if self.expired {
                now - self.deadline_offset
            } else {
                now + self.deadline_offset
            },
            cancellation: self.cancellation.clone(),
        }
    }
}

enum FakeReply {
    Response(ResponsesHttpResponse),
    Error(ResponsesTransportError),
    Pending,
}

#[derive(Clone)]
struct CapturedRequest {
    endpoint: String,
    body: Value,
}

struct FakeTransport {
    replies: Mutex<VecDeque<FakeReply>>,
    requests: Mutex<Vec<CapturedRequest>>,
}

impl FakeTransport {
    fn new(replies: Vec<FakeReply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests
            .lock()
            .map_or_else(|_| Vec::new(), |value| value.clone())
    }
}

impl ResponsesTransport for FakeTransport {
    fn send<'a>(
        &'a self,
        request: ResponsesHttpRequest,
        secret: &'a SecretValue,
        _timeout: Duration,
    ) -> ResponsesFuture<'a, Result<ResponsesHttpResponse, ResponsesTransportError>> {
        let secret_present = secret.with_exposed(|value| value == TEST_SECRET);
        let captured = CapturedRequest {
            endpoint: request.endpoint().to_owned(),
            body: serde_json::from_slice(request.body()).unwrap_or(Value::Null),
        };
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(captured);
        }
        let reply = self
            .replies
            .lock()
            .ok()
            .and_then(|mut replies| replies.pop_front())
            .unwrap_or(FakeReply::Error(ResponsesTransportError::Unavailable));
        Box::pin(async move {
            if !secret_present {
                return Err(ResponsesTransportError::InvalidResponse);
            }
            match reply {
                FakeReply::Response(response) => Ok(response),
                FakeReply::Error(error) => Err(error),
                FakeReply::Pending => future::pending().await,
            }
        })
    }
}

fn provider(
    transport: Arc<FakeTransport>,
    extras: ProgramContextExtras,
    cancellation: CancellationFlag,
    deadline_offset: Duration,
    expired: bool,
) -> OpenAiProgramPlanProvider {
    OpenAiProgramPlanProvider::with_transport(
        Arc::new(FakeCredentialSource::success()),
        Arc::new(FakeContextSource(extras)),
        Arc::new(FixedCallContextFactory {
            cancellation,
            deadline_offset,
            expired,
        }),
        transport,
    )
}

fn candidate() -> ProgramCandidate {
    ProgramCandidate {
        track_id: Uuid::now_v7().to_string(),
        title: Some("Quiet Fixture".to_owned()),
        artist: Some("Fixture Artist".to_owned()),
        album: Some("Fixture Album".to_owned()),
        duration_ms: 180_000,
        normalized_tags: vec!["ambient".to_owned()],
        recent_play_penalty: 0,
    }
}

fn input() -> ProviderProgramInput {
    ProviderProgramInput {
        program_id: Uuid::now_v7().to_string(),
        source_id: "local".to_owned(),
        created_at: "2026-09-03T01:02:03.004Z".to_owned(),
        local_hour: 11,
        profile_tags: vec!["quiet mornings".to_owned()],
        approved_memory_tags: vec!["likes ambient music".to_owned()],
        candidates: vec![candidate()],
    }
}

fn valid_plan(input: &ProviderProgramInput, track_id: &str) -> ProgramPlan {
    ProgramPlan {
        schema_version: "1.0.0".to_owned(),
        program_id: input.program_id.clone(),
        source_id: input.source_id.clone(),
        mode: ProgramPlanMode::Local,
        created_at: input.created_at.clone(),
        segments: vec![
            ProgramPlanSegmentsItem::VoiceSegment(ProgramPlanVoiceSegment {
                r#type: "voice".to_owned(),
                segment_id: Uuid::now_v7().to_string(),
                text: "欢迎回来，先听一段安静的音乐。".to_owned(),
                trigger: ProgramPlanVoiceSegmentTrigger::Opening,
            }),
            ProgramPlanSegmentsItem::TrackSegment(ProgramPlanTrackSegment {
                r#type: "track".to_owned(),
                segment_id: Uuid::now_v7().to_string(),
                track_id: track_id.to_owned(),
                segue_text: None,
            }),
        ],
    }
}

fn response_for_text(text: &str) -> ResponsesHttpResponse {
    let body = serde_json::to_vec(&json!({
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text }]
        }]
    }))
    .expect("fixture response JSON");
    ResponsesHttpResponse::new(200, body).expect("bounded fixture response")
}

fn response_for_plan(plan: &ProgramPlan) -> ResponsesHttpResponse {
    response_for_text(&serde_json::to_string(plan).expect("fixture plan JSON"))
}

fn fragment_kinds(body: &Value) -> Vec<String> {
    body["input"]
        .as_array()
        .into_iter()
        .flatten()
        .skip(1)
        .filter_map(|message| message["content"][0]["text"].as_str())
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .filter_map(|fragment| fragment["kind"].as_str().map(ToOwned::to_owned))
        .collect()
}

#[tokio::test]
async fn context_builder_orders_data_fragments_and_redacts_paths_and_secret_like_values() {
    let mut request = input();
    request
        .profile_tags
        .push("C:\\Users\\private\\music".to_owned());
    request
        .approved_memory_tags
        .push("sk-abcdefghijk-sensitive".to_owned());
    request.candidates[0].title = Some("file:///private/track.flac".to_owned());
    let extras = ProgramContextExtras {
        weather_summary: Some("晴，18°C".to_owned()),
        session_summary: Some("上次听了轻柔音乐。".to_owned()),
        recent_turns: (0..14)
            .map(|index| ContextTurn {
                role: if index % 2 == 0 {
                    ContextTurnRole::User
                } else {
                    ContextTurnRole::Assistant
                },
                text: format!("turn {index}"),
            })
            .collect(),
    };
    let built = ContextBuilder::build(&request, extras, false).expect("bounded context");
    assert!(built.estimated_input_tokens <= MAX_PROVIDER_INPUT_TOKENS);
    let encoded = serde_json::to_string(&built.input).expect("context JSON");
    assert!(encoded.chars().count() <= MAX_CONTEXT_CHARS + 2_000);
    assert!(!encoded.contains("C:\\\\Users"));
    assert!(!encoded.contains("sk-abcdefghijk"));
    assert!(!encoded.contains("file:///private"));
    let value = serde_json::to_value(&built.input).expect("context value");
    let body = json!({ "input": value });
    assert_eq!(
        fragment_kinds(&body),
        [
            "user_profile",
            "approved_memories",
            "weather",
            "session_summary",
            "recent_turns",
            "current_request",
            "candidates",
        ]
    );
    assert_eq!(body["input"][0]["role"], "developer");
    assert_eq!(body["input"][1]["role"], "user");
}

#[tokio::test]
async fn llm_provider_sends_closed_stateless_responses_shape_and_returns_structured_plan() {
    let request = input();
    let plan = valid_plan(&request, &request.candidates[0].track_id);
    let transport = Arc::new(FakeTransport::new(vec![FakeReply::Response(
        response_for_plan(&plan),
    )]));
    let provider = provider(
        Arc::clone(&transport),
        ProgramContextExtras::default(),
        CancellationFlag::default(),
        Duration::from_secs(60),
        false,
    );
    let actual = provider
        .generate_program(&request)
        .await
        .expect("structured plan");
    assert_eq!(actual, plan);

    let captures = transport.requests();
    assert_eq!(captures.len(), 1);
    assert_eq!(
        captures[0].endpoint,
        "https://api.openai.example/v1/responses"
    );
    let body = &captures[0].body;
    let keys = body
        .as_object()
        .expect("request object")
        .keys()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    assert_eq!(
        keys,
        HashSet::from([
            "model",
            "store",
            "background",
            "input",
            "text",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "max_output_tokens",
            "truncation",
        ])
    );
    assert_eq!(body["store"], false);
    assert_eq!(body["background"], false);
    assert_eq!(body["tools"], json!([]));
    assert_eq!(body["tool_choice"], "none");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["max_output_tokens"], 4_000);
    assert_eq!(body["truncation"], "disabled");
    assert_eq!(body["text"]["format"]["type"], "json_schema");
    assert_eq!(body["text"]["format"]["strict"], true);
    let encoded = serde_json::to_string(body).expect("captured request");
    assert!(!encoded.contains(TEST_SECRET));
}

#[tokio::test]
async fn llm_provider_retries_structured_invalid_output_exactly_once_with_safe_repair_data() {
    let request = input();
    let plan = valid_plan(&request, &request.candidates[0].track_id);
    let transport = Arc::new(FakeTransport::new(vec![
        FakeReply::Response(response_for_text("not-json")),
        FakeReply::Response(response_for_plan(&plan)),
    ]));
    let provider = provider(
        Arc::clone(&transport),
        ProgramContextExtras::default(),
        CancellationFlag::default(),
        Duration::from_secs(60),
        false,
    );
    assert_eq!(
        provider.generate_program(&request).await.expect("repaired"),
        plan
    );
    let captures = transport.requests();
    assert_eq!(captures.len(), 2);
    assert!(!fragment_kinds(&captures[0].body).contains(&"repair".to_owned()));
    assert_eq!(
        fragment_kinds(&captures[1].body).last().map(String::as_str),
        Some("repair")
    );
    let repair_text = captures[1].body["input"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"][0]["text"].as_str())
        .expect("repair fragment");
    assert!(repair_text.contains("structured_output_invalid"));
    assert!(repair_text.contains(&request.candidates[0].track_id));
    assert!(!repair_text.contains("not-json"));
}

#[tokio::test]
async fn llm_provider_does_not_retry_protocol_or_provider_errors() {
    let request = input();
    for (status, expected) in [
        (401, ProgramProviderError::Authentication),
        (429, ProgramProviderError::RateLimited),
        (503, ProgramProviderError::Unavailable),
    ] {
        let response = ResponsesHttpResponse::new(status, Vec::new()).expect("empty error body");
        let transport = Arc::new(FakeTransport::new(vec![FakeReply::Response(response)]));
        let provider = provider(
            Arc::clone(&transport),
            ProgramContextExtras::default(),
            CancellationFlag::default(),
            Duration::from_secs(60),
            false,
        );
        assert_eq!(provider.generate_program(&request).await, Err(expected));
        assert_eq!(transport.requests().len(), 1);
    }

    let malformed_envelope = ResponsesHttpResponse::new(200, br#"{"object":"other"}"#.to_vec())
        .expect("bounded malformed envelope");
    let transport = Arc::new(FakeTransport::new(vec![FakeReply::Response(
        malformed_envelope,
    )]));
    let provider = provider(
        Arc::clone(&transport),
        ProgramContextExtras::default(),
        CancellationFlag::default(),
        Duration::from_secs(60),
        false,
    );
    assert_eq!(
        provider.generate_program(&request).await,
        Err(ProgramProviderError::InvalidResponse)
    );
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn llm_provider_cancellation_and_deadline_discard_or_prevent_transport_results() {
    let request = input();
    let cancelled = CancellationFlag::default();
    cancelled.cancel();
    let transport = Arc::new(FakeTransport::new(Vec::new()));
    let cancelled_provider = provider(
        transport.clone(),
        ProgramContextExtras::default(),
        cancelled,
        Duration::from_secs(60),
        false,
    );
    assert_eq!(
        cancelled_provider.generate_program(&request).await,
        Err(ProgramProviderError::Cancelled)
    );
    assert!(transport.requests().is_empty());

    let transport = Arc::new(FakeTransport::new(Vec::new()));
    let expired_provider = provider(
        Arc::clone(&transport),
        ProgramContextExtras::default(),
        CancellationFlag::default(),
        Duration::from_millis(1),
        true,
    );
    assert_eq!(
        expired_provider.generate_program(&request).await,
        Err(ProgramProviderError::Timeout)
    );
    assert!(transport.requests().is_empty());

    let transport = Arc::new(FakeTransport::new(vec![FakeReply::Pending]));
    let pending_provider = provider(
        Arc::clone(&transport),
        ProgramContextExtras::default(),
        CancellationFlag::default(),
        Duration::from_millis(20),
        false,
    );
    assert_eq!(
        pending_provider.generate_program(&request).await,
        Err(ProgramProviderError::Timeout)
    );
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn context_builder_rejects_candidate_overflow_before_any_network_or_credential_side_effect() {
    let mut request = input();
    request.candidates = (0..201).map(|_| candidate()).collect();
    let transport = Arc::new(FakeTransport::new(Vec::new()));
    let provider = OpenAiProgramPlanProvider::with_transport(
        Arc::new(FakeCredentialSource::failure(
            ProgramProviderError::Authentication,
        )),
        Arc::new(FakeContextSource(ProgramContextExtras::default())),
        Arc::new(FixedCallContextFactory {
            cancellation: CancellationFlag::default(),
            deadline_offset: Duration::from_secs(60),
            expired: false,
        }),
        transport.clone(),
    );
    assert_eq!(
        provider.generate_program(&request).await,
        Err(ProgramProviderError::InvalidResponse)
    );
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn llm_provider_returns_structurally_valid_unknown_candidate_for_downstream_domain_validation()
 {
    let request = input();
    let unknown = Uuid::now_v7().to_string();
    let plan = valid_plan(&request, &unknown);
    let transport = Arc::new(FakeTransport::new(vec![FakeReply::Response(
        response_for_plan(&plan),
    )]));
    let provider = provider(
        Arc::clone(&transport),
        ProgramContextExtras::default(),
        CancellationFlag::default(),
        Duration::from_secs(60),
        false,
    );
    assert_eq!(provider.generate_program(&request).await, Ok(plan));
    assert_eq!(transport.requests().len(), 1);
}
