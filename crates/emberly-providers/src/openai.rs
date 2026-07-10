//! OpenAI-compatible Chat Completions client (Tech Spec §4.2). A thin,
//! first-party `reqwest` client with a configurable base URL — which
//! transitively covers Ollama, vLLM, OpenRouter, and private deployments
//! (P-2). No wire type escapes this module (P-1).

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::auth::Auth;
use crate::error::ProviderError;
use crate::message::{CompletionRequest, ContentBlock, Message, Role};
use crate::model::{ModelInfo, ProviderId, TokenEstimate, TokenUsage};
use crate::provider::Provider;
use crate::sse::SseEvent;
use crate::stream::{CompletionStream, StopReason, StreamEvent};
use crate::wire::{check_response, sse_completion_stream, SseMapper};
use crate::ToolCallId;

/// A client for any OpenAI-compatible `/chat/completions` endpoint.
pub struct OpenAiProvider {
    client: reqwest::Client,
    auth: Auth,
    /// API base including the version segment, e.g. `https://api.openai.com/v1`
    /// (or an Ollama/vLLM base). `/chat/completions` is appended.
    base_url: String,
    model_info: ModelInfo,
}

impl OpenAiProvider {
    #[must_use]
    pub fn new(
        client: reqwest::Client,
        auth: Auth,
        base_url: impl Into<String>,
        model_info: ModelInfo,
    ) -> Self {
        Self {
            client,
            auth,
            base_url: base_url.into(),
            model_info,
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("openai-compat")
    }

    fn model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionStream, ProviderError> {
        let body = build_body(&request);
        let builder = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        let response = self
            .auth
            .apply(builder)
            .send()
            .await
            .map_err(|e| ProviderError::Connect(e.to_string()))?;
        let response = check_response(response).await?;
        Ok(sse_completion_stream(response, OpenAiMapper::default()))
    }

    fn count_tokens(&self, text: &str) -> TokenEstimate {
        let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        TokenEstimate {
            tokens: chars.div_ceil(4),
            approximate: true,
        }
    }
}

fn build_body(request: &CompletionRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = &request.system {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.extend(request.messages.iter().map(message_to_openai));

    let mut body = json!({
        "model": request.model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": messages,
    });
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|t| json!({
                "type": "function",
                "function": { "name": t.name, "description": t.description, "parameters": t.input_schema }
            }))
            .collect();
    }
    if let Some(max) = request.max_output_tokens {
        // Newer OpenAI models (gpt-5.x, o-series) reject the legacy `max_tokens`
        // and require `max_completion_tokens`; current OpenAI-compatible servers
        // accept it too (or ignore unknown fields).
        body["max_completion_tokens"] = json!(max);
    }
    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }
    body
}

/// Map a normalized message to an OpenAI chat message.
fn message_to_openai(message: &Message) -> Value {
    match message.role {
        Role::Tool => {
            // A tool result: role "tool" with the correlating id.
            let (call_id, content) = tool_result_fields(message);
            json!({ "role": "tool", "tool_call_id": call_id, "content": content })
        }
        Role::Assistant => {
            let text = joined_text(message);
            let tool_calls: Vec<Value> = message
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolUse { id, name, input } => Some(json!({
                        "id": id.0,
                        "type": "function",
                        "function": { "name": name, "arguments": input.to_string() }
                    })),
                    _ => None,
                })
                .collect();
            let mut msg = json!({ "role": "assistant" });
            // content may be null when only tool calls are present.
            msg["content"] = if text.is_empty() {
                Value::Null
            } else {
                json!(text)
            };
            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls);
            }
            msg
        }
        Role::System => json!({ "role": "system", "content": joined_text(message) }),
        Role::User => json!({ "role": "user", "content": joined_text(message) }),
    }
}

fn joined_text(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn tool_result_fields(message: &Message) -> (String, String) {
    for block in &message.content {
        if let ContentBlock::ToolResult {
            call_id, content, ..
        } = block
        {
            return (call_id.0.clone(), content.clone());
        }
    }
    (String::new(), String::new())
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    }
}

/// Translates OpenAI streaming `chat.completion.chunk` events.
#[derive(Default)]
struct OpenAiMapper {
    /// tool-call stream index → id (later chunks omit the id).
    tool_ids: HashMap<u64, ToolCallId>,
}

impl SseMapper for OpenAiMapper {
    fn map(&mut self, event: SseEvent) -> Vec<Result<StreamEvent, ProviderError>> {
        if event.data.trim() == "[DONE]" {
            return Vec::new();
        }
        let Ok(data): Result<Value, _> = serde_json::from_str(&event.data) else {
            return Vec::new();
        };
        let mut out = Vec::new();

        // Usage (sent as a final chunk with empty choices when include_usage).
        if let Some(usage) = data.get("usage").filter(|u| !u.is_null()) {
            out.push(Ok(StreamEvent::Usage {
                usage: TokenUsage {
                    input: usage
                        .get("prompt_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    output: usage
                        .get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                },
            }));
        }

        let Some(choice) = data.get("choices").and_then(|c| c.get(0)) else {
            return out;
        };
        let delta = &choice["delta"];

        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                out.push(Ok(StreamEvent::TextDelta {
                    text: text.to_string(),
                }));
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let function = &call["function"];
                if let std::collections::hash_map::Entry::Vacant(entry) = self.tool_ids.entry(index)
                {
                    let id =
                        ToolCallId::new(call.get("id").and_then(Value::as_str).unwrap_or_default());
                    let name = function
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    entry.insert(id.clone());
                    out.push(Ok(StreamEvent::ToolCallStart { id, name }));
                }
                if let (Some(id), Some(args)) = (
                    self.tool_ids.get(&index),
                    function.get("arguments").and_then(Value::as_str),
                ) {
                    if !args.is_empty() {
                        out.push(Ok(StreamEvent::ToolCallDelta {
                            id: id.clone(),
                            args_delta: args.to_string(),
                        }));
                    }
                }
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            for id in self.tool_ids.values() {
                out.push(Ok(StreamEvent::ToolCallEnd { id: id.clone() }));
            }
            out.push(Ok(StreamEvent::Done {
                stop_reason: map_finish_reason(reason),
            }));
        }

        out
    }
}
