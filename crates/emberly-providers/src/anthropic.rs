//! Anthropic Messages API client (Tech Spec §4.2). A thin, first-party
//! `reqwest` client — no vendor SDK (P-4). Maps the normalized request to the
//! Messages wire format and its SSE stream back to normalized `StreamEvent`s;
//! no Anthropic wire type escapes this module (P-1).
//!
//! The `reqwest::Client` is injected, so the TLS/crypto backend is chosen at
//! wiring time (Phase 3 group 7) and tests run over plain HTTP.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ProviderError;
use crate::message::{CompletionRequest, ContentBlock, Message, Role};
use crate::model::{ModelInfo, ProviderId, TokenEstimate};
use crate::provider::Provider;
use crate::sse::SseEvent;
use crate::stream::{CompletionStream, StopReason, StreamEvent};
use crate::wire::{check_response, sse_completion_stream, SseMapper};
use crate::ToolCallId;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// A client for the Anthropic Messages API.
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model_info: ModelInfo,
}

impl AnthropicProvider {
    /// Build a client. `base_url` has no trailing slash (e.g.
    /// `https://api.anthropic.com`); pass the mock server's URL in tests.
    #[must_use]
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model_info: ModelInfo,
    ) -> Self {
        Self {
            client,
            api_key: api_key.into(),
            base_url: base_url.into(),
            model_info,
        }
    }

    /// Convenience constructor using the public API base URL.
    #[must_use]
    pub fn with_default_url(
        client: reqwest::Client,
        api_key: impl Into<String>,
        model_info: ModelInfo,
    ) -> Self {
        Self::new(client, api_key, DEFAULT_BASE_URL, model_info)
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    fn model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionStream, ProviderError> {
        let body = build_body(&request, self.model_info.max_output_tokens);
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Connect(e.to_string()))?;
        let response = check_response(response).await?;
        Ok(sse_completion_stream(response, AnthropicMapper::default()))
    }

    fn count_tokens(&self, text: &str) -> TokenEstimate {
        let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        TokenEstimate {
            tokens: chars.div_ceil(4),
            approximate: true,
        }
    }
}

/// Build the Messages request body from the normalized request.
fn build_body(request: &CompletionRequest, default_max_tokens: u32) -> Value {
    let max_tokens = request
        .max_output_tokens
        .unwrap_or(default_max_tokens)
        .max(1);
    let mut body = json!({
        "model": request.model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": request.messages.iter().map(message_to_anthropic).collect::<Vec<_>>(),
    });
    if let Some(system) = &request.system {
        body["system"] = json!(system);
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.input_schema }))
            .collect();
    }
    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }
    body
}

/// Map a normalized message to an Anthropic message. Tool results are carried
/// as user-role `tool_result` blocks (Anthropic has no `tool` role).
fn message_to_anthropic(message: &Message) -> Value {
    let role = match message.role {
        Role::Assistant => "assistant",
        // System is sent top-level; User and Tool both map to "user" here.
        _ => "user",
    };
    let content: Vec<Value> = message.content.iter().map(block_to_anthropic).collect();
    json!({ "role": role, "content": content })
}

fn block_to_anthropic(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
        ContentBlock::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id.0, "name": name, "input": input })
        }
        ContentBlock::ToolResult {
            call_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": call_id.0,
            "content": content,
            "is_error": is_error,
        }),
    }
}

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::Stop,
        other => StopReason::Other(other.to_string()),
    }
}

/// Translates Anthropic's `content_block_*` / `message_*` SSE events.
#[derive(Default)]
struct AnthropicMapper {
    /// content-block index → tool-call id, for routing `input_json_delta`.
    tool_ids: HashMap<u64, ToolCallId>,
    input_tokens: u64,
    stop_reason: StopReason,
}

impl SseMapper for AnthropicMapper {
    fn map(&mut self, event: SseEvent) -> Vec<Result<StreamEvent, ProviderError>> {
        let Ok(data): Result<Value, _> = serde_json::from_str(&event.data) else {
            return Vec::new();
        };
        let event_type = data.get("type").and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();

        match event_type {
            "message_start" => {
                if let Some(tokens) = data
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.input_tokens = tokens;
                }
            }
            "content_block_start" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let block = &data["content_block"];
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let id = ToolCallId::new(
                        block.get("id").and_then(Value::as_str).unwrap_or_default(),
                    );
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.tool_ids.insert(index, id.clone());
                    out.push(Ok(StreamEvent::ToolCallStart { id, name }));
                }
            }
            "content_block_delta" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let delta = &data["delta"];
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            out.push(Ok(StreamEvent::TextDelta {
                                text: text.to_string(),
                            }));
                        }
                    }
                    Some("input_json_delta") => {
                        if let (Some(id), Some(partial)) = (
                            self.tool_ids.get(&index),
                            delta.get("partial_json").and_then(Value::as_str),
                        ) {
                            out.push(Ok(StreamEvent::ToolCallDelta {
                                id: id.clone(),
                                args_delta: partial.to_string(),
                            }));
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                if let Some(id) = self.tool_ids.get(&index) {
                    out.push(Ok(StreamEvent::ToolCallEnd { id: id.clone() }));
                }
            }
            "message_delta" => {
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = map_stop_reason(reason);
                }
                if let Some(output) = data.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    out.push(Ok(StreamEvent::Usage {
                        usage: crate::TokenUsage {
                            input: self.input_tokens,
                            output,
                        },
                    }));
                }
            }
            "message_stop" => {
                out.push(Ok(StreamEvent::Done {
                    stop_reason: self.stop_reason.clone(),
                }));
            }
            "error" => {
                let message = data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("anthropic stream error")
                    .to_string();
                out.push(Err(ProviderError::Api { message }));
            }
            _ => {} // ping, unknown
        }
        out
    }
}
