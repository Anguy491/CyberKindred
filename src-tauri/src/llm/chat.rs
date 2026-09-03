//! Stateless `OpenAI` Responses adapter for user-triggered M4 chat.

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    providers::{CancellationFlag, ProviderCallContext},
    storage::{ChatContextSnapshot, StoredMemoryKind},
    understanding::{
        ChatFuture, ChatProvider, ChatProviderError, ChatProviderOutput, ProposedMemory,
    },
};

use super::{
    ProgramCredentialSource, ReqwestResponsesTransport, ResponsesHttpRequest, ResponsesTransport,
    ResponsesTransportError,
};

const RESPONSES_PATH: &str = "/v1/responses";
const PROMPT_VERSION: &str = "chat-v1";
const MAX_OUTPUT_TOKENS: u16 = 2_000;

pub(crate) struct OpenAiChatProvider {
    credentials: Arc<dyn ProgramCredentialSource>,
    transport: Arc<dyn ResponsesTransport>,
}

impl OpenAiChatProvider {
    pub(crate) fn new(
        credentials: Arc<dyn ProgramCredentialSource>,
    ) -> Result<Self, ChatProviderError> {
        let transport = ReqwestResponsesTransport::new().map_err(map_transport)?;
        Ok(Self {
            credentials,
            transport: Arc::new(transport),
        })
    }
}

impl ChatProvider for OpenAiChatProvider {
    fn respond<'a>(
        &'a self,
        context: &'a ChatContextSnapshot,
        call: &'a ProviderCallContext,
    ) -> ChatFuture<'a> {
        Box::pin(async move {
            ensure_active(call)?;
            let credential = self.credentials.load().await.map_err(|error| match error {
                crate::program::ProgramProviderError::Cancelled => ChatProviderError::Cancelled,
                _ => ChatProviderError::Unavailable,
            })?;
            ensure_active(call)?;
            let input = build_input(context)?;
            let body = serde_json::to_vec(&ResponsesRequest {
                model: credential.model_id(),
                store: false,
                background: false,
                input,
                text: TextConfiguration {
                    format: JsonSchemaFormat {
                        r#type: "json_schema",
                        name: "cyberkindred_chat_v1",
                        strict: true,
                        schema: response_schema(),
                    },
                },
                tools: [],
                tool_choice: "none",
                parallel_tool_calls: false,
                max_output_tokens: MAX_OUTPUT_TOKENS,
                truncation: "disabled",
            })
            .map_err(|_| ChatProviderError::InvalidResponse)?;
            let request = ResponsesHttpRequest::new(
                format!("{}{RESPONSES_PATH}", credential.origin().as_str()),
                body,
            )
            .map_err(map_transport)?;
            let remaining = remaining(call)?;
            let pending = self.transport.send(request, credential.secret(), remaining);
            tokio::pin!(pending);
            let response = tokio::select! {
                result = &mut pending => result.map_err(map_transport)?,
                () = wait_for_cancellation(&call.cancellation) => return Err(ChatProviderError::Cancelled),
                () = tokio::time::sleep(remaining) => return Err(ChatProviderError::Unavailable),
            };
            ensure_active(call)?;
            if !(200..=299).contains(&response.status()) {
                return Err(ChatProviderError::Unavailable);
            }
            let output_text = extract_output_text(response.body())?;
            let output: WireOutput = serde_json::from_str(&output_text)
                .map_err(|_| ChatProviderError::InvalidResponse)?;
            validate_output(&output)?;
            Ok(ChatProviderOutput {
                text: output.text,
                proposed_memories: output
                    .proposed_memories
                    .into_iter()
                    .map(|proposal| ProposedMemory {
                        kind: match proposal.kind {
                            WireMemoryKind::Preference => StoredMemoryKind::Preference,
                            WireMemoryKind::Routine => StoredMemoryKind::Routine,
                            WireMemoryKind::Boundary => StoredMemoryKind::Boundary,
                            WireMemoryKind::Biographical => StoredMemoryKind::Biographical,
                        },
                        content: proposal.content,
                        confidence: proposal.confidence,
                    })
                    .collect(),
                provider: "openai",
                model: credential.model_id().to_owned(),
                prompt_version: PROMPT_VERSION,
            })
        })
    }
}

#[derive(Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    store: bool,
    background: bool,
    input: Vec<InputMessage>,
    text: TextConfiguration,
    tools: [serde_json::Value; 0],
    tool_choice: &'static str,
    parallel_tool_calls: bool,
    max_output_tokens: u16,
    truncation: &'static str,
}

#[derive(Serialize)]
struct InputMessage {
    role: &'static str,
    content: Vec<InputContent>,
}

#[derive(Serialize)]
struct InputContent {
    r#type: &'static str,
    text: String,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireOutput {
    text: String,
    proposed_memories: Vec<WireProposal>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireProposal {
    kind: WireMemoryKind,
    content: String,
    confidence: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireMemoryKind {
    Preference,
    Routine,
    Boundary,
    Biographical,
}

#[derive(Deserialize)]
struct Envelope {
    object: String,
    status: String,
    output: Vec<OutputItem>,
}
#[derive(Deserialize)]
struct OutputItem {
    r#type: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    content: Option<Vec<OutputContent>>,
}
#[derive(Deserialize)]
struct OutputContent {
    r#type: String,
    #[serde(default)]
    text: Option<String>,
}

fn build_input(context: &ChatContextSnapshot) -> Result<Vec<InputMessage>, ChatProviderError> {
    let mut input = vec![message(
        "developer",
        "你是 CyberKindred 的安静温暖中文电台伙伴。只回答用户当前请求；不得声称已播放、控制设备或保存记忆。你无法访问用户的屏幕、麦克风、设备定位或 Apple Music 内部推荐；用户请求这些能力时必须明确说明限制，并邀请其用文字描述或使用当前可见控件。不得冒充人类、医生或危机热线，不得诱导用户产生依赖。可提出最多三条简短记忆候选，但只有用户审批后才会使用。不得为精确地址、凭据、财务账号、医疗诊断、性取向、宗教或政治立场创建记忆提案。不要复述隐私数据。",
    )];
    if !context.initial_preferences.is_empty() || !context.approved_memories.is_empty() {
        let facts = json!({
            "initialPreferences": context.initial_preferences,
            "approvedMemories": context.approved_memories.iter().map(|memory| &memory.content).collect::<Vec<_>>(),
        });
        input.push(message(
            "developer",
            &format!("已批准的本地上下文（提案和停用记忆已排除）：{facts}"),
        ));
    }
    for turn in &context.turns {
        let role = match turn.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            _ => return Err(ChatProviderError::InvalidResponse),
        };
        input.push(message(role, &turn.text));
    }
    Ok(input)
}

fn message(role: &'static str, text: &str) -> InputMessage {
    InputMessage {
        role,
        content: vec![InputContent {
            r#type: "input_text",
            text: text.to_owned(),
        }],
    }
}

fn response_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text", "proposedMemories"],
        "properties": {
            "text": { "type": "string", "minLength": 1, "maxLength": 2000 },
            "proposedMemories": {
                "type": "array", "maxItems": 3,
                "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["kind", "content", "confidence"],
                    "properties": {
                        "kind": { "type": "string", "enum": ["preference", "routine", "boundary", "biographical"] },
                        "content": { "type": "string", "minLength": 1, "maxLength": 500 },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                    }
                }
            }
        }
    })
}

fn validate_output(output: &WireOutput) -> Result<(), ChatProviderError> {
    if output.text.is_empty()
        || output.text.chars().count() > 2_000
        || output.proposed_memories.len() > 3
        || output.proposed_memories.iter().any(|proposal| {
            proposal.content.is_empty()
                || proposal.content.chars().count() > 500
                || proposal.content.chars().any(char::is_control)
                || !(0.0..=1.0).contains(&proposal.confidence)
        })
    {
        Err(ChatProviderError::InvalidResponse)
    } else {
        Ok(())
    }
}

fn extract_output_text(body: &[u8]) -> Result<String, ChatProviderError> {
    let parsed: Envelope =
        serde_json::from_slice(body).map_err(|_| ChatProviderError::InvalidResponse)?;
    if parsed.object != "response" || parsed.status != "completed" {
        return Err(ChatProviderError::InvalidResponse);
    }
    let messages = parsed
        .output
        .into_iter()
        .filter(|item| item.r#type != "reasoning")
        .collect::<Vec<_>>();
    let [message] = messages.as_slice() else {
        return Err(ChatProviderError::InvalidResponse);
    };
    if message.r#type != "message"
        || message.role.as_deref() != Some("assistant")
        || message.status.as_deref() != Some("completed")
    {
        return Err(ChatProviderError::InvalidResponse);
    }
    let Some([content]) = message.content.as_deref() else {
        return Err(ChatProviderError::InvalidResponse);
    };
    if content.r#type != "output_text" {
        return Err(ChatProviderError::InvalidResponse);
    }
    content
        .text
        .clone()
        .ok_or(ChatProviderError::InvalidResponse)
}

fn ensure_active(call: &ProviderCallContext) -> Result<(), ChatProviderError> {
    if call.cancellation.is_cancelled() {
        Err(ChatProviderError::Cancelled)
    } else {
        remaining(call).map(|_| ())
    }
}
fn remaining(call: &ProviderCallContext) -> Result<Duration, ChatProviderError> {
    call.deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(ChatProviderError::Unavailable)
}
async fn wait_for_cancellation(flag: &CancellationFlag) {
    while !flag.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
const fn map_transport(error: ResponsesTransportError) -> ChatProviderError {
    match error {
        ResponsesTransportError::InvalidResponse => ChatProviderError::InvalidResponse,
        ResponsesTransportError::Timeout | ResponsesTransportError::Unavailable => {
            ChatProviderError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{ContextMemory, ContextTurn};
    use uuid::Uuid;

    #[test]
    fn chat_context_contains_only_the_supplied_approved_projection() {
        let context = ChatContextSnapshot {
            display_name: "小岚".to_owned(),
            initial_preferences: vec!["夜间".to_owned()],
            approved_memories: vec![ContextMemory {
                memory_id: Uuid::now_v7(),
                content: "喜欢环境音乐".to_owned(),
            }],
            turns: vec![ContextTurn {
                role: "user".to_owned(),
                text: "来点安静的".to_owned(),
            }],
        };
        let encoded =
            serde_json::to_string(&build_input(&context).expect("input")).expect("serialize input");
        assert!(encoded.contains("喜欢环境音乐"));
        assert!(encoded.contains("来点安静的"));
        assert!(!encoded.contains("proposal"));
        assert!(!encoded.contains("path"));
        assert!(!encoded.contains("小岚"));
        for boundary in ["屏幕", "麦克风", "设备定位", "Apple Music 内部推荐"] {
            assert!(encoded.contains(boundary));
        }
        for sensitive in [
            "精确地址",
            "凭据",
            "财务账号",
            "医疗诊断",
            "性取向",
            "宗教",
            "政治立场",
        ] {
            assert!(encoded.contains(sensitive));
        }
    }

    #[test]
    fn chat_output_rejects_oversized_or_excessive_memory_content() {
        let output = WireOutput {
            text: "好的".to_owned(),
            proposed_memories: (0..4)
                .map(|_| WireProposal {
                    kind: WireMemoryKind::Preference,
                    content: "安静".to_owned(),
                    confidence: 0.9,
                })
                .collect(),
        };
        assert_eq!(
            validate_output(&output),
            Err(ChatProviderError::InvalidResponse)
        );
    }
}
