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

use crate::auth::Auth;
use crate::error::ProviderError;
use crate::message::{CompletionRequest, ContentBlock, Message, Role};
use crate::model::{Effort, ModelInfo, ProviderId, TokenEstimate};
use crate::provider::Provider;
use crate::sse::SseEvent;
use crate::stream::{CompletionStream, StopReason, StreamEvent};
use crate::wire::{check_response, sse_completion_stream, SseMapper, StreamTimeouts};
use crate::ToolCallId;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// A client for the Anthropic Messages API (and any endpoint speaking its wire
/// format — the profile decides the base URL and auth, P-8).
pub struct AnthropicProvider {
    client: reqwest::Client,
    auth: Auth,
    base_url: String,
    model_info: ModelInfo,
}

impl AnthropicProvider {
    /// Build a client. `base_url` has no trailing slash (e.g.
    /// `https://api.anthropic.com`); pass the mock server's URL in tests.
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

    /// Convenience constructor using the public API base URL.
    #[must_use]
    pub fn with_default_url(client: reqwest::Client, auth: Auth, model_info: ModelInfo) -> Self {
        Self::new(client, auth, DEFAULT_BASE_URL, model_info)
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
        let body = build_body(
            &request,
            self.model_info.max_output_tokens,
            &self.model_info.effort_levels,
        );
        let builder = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("anthropic-version", ANTHROPIC_VERSION);
        let response = self
            .auth
            .apply(builder)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Connect(e.to_string()))?;
        let response = check_response(response).await?;
        Ok(sse_completion_stream(
            response,
            AnthropicMapper::default(),
            StreamTimeouts::default(),
        ))
    }

    fn count_tokens(&self, text: &str) -> TokenEstimate {
        crate::model::estimate_tokens(text)
    }
}

/// The target `thinking.budget_tokens` for each effort level, before clamping
/// (Tech Spec §4.6). Initial values; tune with use — owner-approved 2026-07-10.
fn effort_budget_target(effort: Effort) -> u32 {
    match effort {
        Effort::Low => 2_048,
        Effort::Medium => 8_192,
        Effort::High => 16_384,
        Effort::Max => 32_768,
    }
}

/// The Anthropic thinking budget for `effort`, clamped to fit the model's
/// output allowance. Anthropic requires `1024 <= budget_tokens < max_tokens`;
/// we leave a 1024-token reserve so the answer always has room. `None` when
/// `max_tokens` is too small to fit any valid thinking block — the adapter then
/// omits thinking entirely (a no-op, never an error — P-9).
fn thinking_budget(effort: Effort, max_tokens: u32) -> Option<u32> {
    const RESERVE: u32 = 1_024;
    const MIN: u32 = 1_024;
    let ceiling = max_tokens.checked_sub(RESERVE)?;
    if ceiling < MIN {
        return None;
    }
    Some(effort_budget_target(effort).clamp(MIN, ceiling))
}

/// Build the Messages request body from the normalized request. `effort_levels`
/// is the active model's declared support (empty ⇒ no reasoning control, so an
/// `effort` on the request is silently ignored — P-9).
fn build_body(
    request: &CompletionRequest,
    default_max_tokens: u32,
    effort_levels: &[Effort],
) -> Value {
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
    // Reasoning effort → thinking block, only if the model declares support and
    // a valid budget fits (P-9, Tech Spec §4.6).
    let thinking_enabled = request
        .effort
        .filter(|_| !effort_levels.is_empty())
        .and_then(|effort| thinking_budget(effort, max_tokens))
        .map(|budget| {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        })
        .is_some();
    // Extended thinking requires the default temperature; omit any override when
    // thinking is on, otherwise the API rejects the request.
    if let Some(temp) = request.temperature {
        if !thinking_enabled {
            body["temperature"] = json!(temp);
        }
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
        // Replay a captured thinking block verbatim so multi-turn thinking
        // works (P-10). Redacted blocks carry their opaque `data` in the
        // signature slot and go back as `redacted_thinking`.
        ContentBlock::Reasoning {
            text,
            signature,
            redacted,
        } => {
            if *redacted {
                json!({ "type": "redacted_thinking", "data": signature.clone().unwrap_or_default() })
            } else {
                json!({
                    "type": "thinking",
                    "thinking": text,
                    "signature": signature.clone().unwrap_or_default(),
                })
            }
        }
        // Map an image to the Anthropic base64 `source` shape (P-11, Tech Spec
        // §4.2). Renders the block wherever it sits — user content or nested in
        // a tool_result content array.
        ContentBlock::Image { media_type, data } => {
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data,
                }
            })
        }
        // Map a document to the Anthropic base64 `document` source shape
        // (P-12, Tech Spec §4.2). Pulled forward from group 4: `ContentBlock`
        // is not `#[non_exhaustive]`, so this match must cover `Document` for
        // the crate to compile at all once the variant exists.
        ContentBlock::Document { media_type, data } => {
            json!({
                "type": "document",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data,
                }
            })
        }
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
    /// content-block index → accumulated `signature_delta` for a thinking
    /// block. Presence marks the index as a (non-redacted) thinking block, so
    /// its signature is flushed as a `ReasoningSignature` at block stop (P-10).
    thinking_sigs: HashMap<u64, String>,
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
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
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
                    Some("thinking") => {
                        // Mark this index as a thinking block; the signature
                        // accumulates via `signature_delta` and flushes at stop.
                        self.thinking_sigs.insert(index, String::new());
                    }
                    Some("redacted_thinking") => {
                        // Encrypted reasoning: no text to stream. Surface a
                        // placeholder so it is never silently dropped (P-10) and
                        // preserve the opaque `data` for verbatim replay.
                        let data_blob = block
                            .get("data")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        out.push(Ok(StreamEvent::ReasoningDelta {
                            text: "[redacted reasoning]".to_string(),
                        }));
                        out.push(Ok(StreamEvent::ReasoningSignature {
                            signature: data_blob,
                            redacted: true,
                        }));
                    }
                    _ => {}
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
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            out.push(Ok(StreamEvent::ReasoningDelta {
                                text: text.to_string(),
                            }));
                        }
                    }
                    Some("signature_delta") => {
                        if let (Some(acc), Some(sig)) = (
                            self.thinking_sigs.get_mut(&index),
                            delta.get("signature").and_then(Value::as_str),
                        ) {
                            acc.push_str(sig);
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
                // Flush a thinking block's accumulated signature for replay.
                if let Some(signature) = self.thinking_sigs.remove(&index) {
                    out.push(Ok(StreamEvent::ReasoningSignature {
                        signature,
                        redacted: false,
                    }));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with_effort(effort: Option<Effort>) -> CompletionRequest {
        let mut r = CompletionRequest::new("claude-x");
        r.effort = effort;
        r
    }

    fn budget_of(body: &Value) -> Option<u64> {
        body.get("thinking")
            .and_then(|t| t.get("budget_tokens"))
            .and_then(Value::as_u64)
    }

    #[test]
    fn maps_each_level_to_its_budget_when_supported() {
        // A large output allowance so no clamping occurs.
        let levels = Effort::ALL.to_vec();
        for (effort, want) in [
            (Effort::Low, 2_048),
            (Effort::Medium, 8_192),
            (Effort::High, 16_384),
            (Effort::Max, 32_768),
        ] {
            let body = build_body(&req_with_effort(Some(effort)), 64_000, &levels);
            assert_eq!(budget_of(&body), Some(want), "level {effort}");
        }
    }

    #[test]
    fn omits_thinking_when_no_effort() {
        let body = build_body(&req_with_effort(None), 64_000, &Effort::ALL);
        assert_eq!(body.get("thinking"), None);
    }

    #[test]
    fn unsupported_model_is_a_noop_even_with_effort() {
        // Empty effort_levels ⇒ the model has no reasoning control (P-9).
        let body = build_body(&req_with_effort(Some(Effort::High)), 64_000, &[]);
        assert_eq!(body.get("thinking"), None);
    }

    #[test]
    fn budget_is_clamped_to_the_output_allowance() {
        // max_tokens 8192 ⇒ ceiling 7168; Max's 32768 target clamps down.
        let body = build_body(&req_with_effort(Some(Effort::Max)), 8_192, &Effort::ALL);
        assert_eq!(budget_of(&body), Some(7_168));
    }

    #[test]
    fn tiny_allowance_omits_thinking() {
        // max_tokens < 2048 can't fit a valid block ⇒ no-op, not an error.
        let body = build_body(&req_with_effort(Some(Effort::Low)), 1_500, &Effort::ALL);
        assert_eq!(body.get("thinking"), None);
    }

    #[test]
    fn temperature_dropped_when_thinking_enabled() {
        let mut r = req_with_effort(Some(Effort::Low));
        r.temperature = Some(0.7);
        let body = build_body(&r, 64_000, &Effort::ALL);
        assert!(body.get("thinking").is_some());
        assert_eq!(
            body.get("temperature"),
            None,
            "thinking forbids a temp override"
        );
        // Without thinking, the temperature passes through (f32→JSON widens to
        // f64, so compare with a tolerance rather than for exact equality).
        let body2 = build_body(&r, 64_000, &[]);
        let temp = body2.get("temperature").and_then(Value::as_f64);
        assert!(
            matches!(temp, Some(t) if (t - 0.7).abs() < 1e-6),
            "got {temp:?}"
        );
    }

    /// Drive `data` frames through the mapper, collecting only the `Ok` events.
    fn run_mapper(frames: &[&str]) -> Vec<StreamEvent> {
        let mut mapper = AnthropicMapper::default();
        let mut out = Vec::new();
        for data in frames {
            let event = SseEvent {
                event: None,
                data: (*data).to_string(),
            };
            out.extend(mapper.map(event).into_iter().filter_map(Result::ok));
        }
        out
    }

    #[test]
    fn thinking_stream_yields_reasoning_then_signature_then_text() {
        let events = run_mapper(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" think"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig123"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"answer"}}"#,
        ]);
        assert_eq!(
            events,
            vec![
                StreamEvent::ReasoningDelta {
                    text: "Let me".into()
                },
                StreamEvent::ReasoningDelta {
                    text: " think".into()
                },
                StreamEvent::ReasoningSignature {
                    signature: "sig123".into(),
                    redacted: false,
                },
                StreamEvent::TextDelta {
                    text: "answer".into()
                },
            ]
        );
    }

    #[test]
    fn redacted_thinking_surfaces_placeholder_and_preserves_data() {
        let events = run_mapper(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"ENCRYPTED"}}"#,
        ]);
        assert_eq!(
            events,
            vec![
                StreamEvent::ReasoningDelta {
                    text: "[redacted reasoning]".into()
                },
                StreamEvent::ReasoningSignature {
                    signature: "ENCRYPTED".into(),
                    redacted: true,
                },
            ]
        );
    }

    #[test]
    fn plain_text_stream_emits_no_reasoning() {
        let events = run_mapper(&[
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
        ]);
        assert!(!events.iter().any(|e| matches!(
            e,
            StreamEvent::ReasoningDelta { .. } | StreamEvent::ReasoningSignature { .. }
        )));
    }

    #[test]
    fn reasoning_block_replays_as_thinking_with_signature() {
        let block = ContentBlock::Reasoning {
            text: "prior thought".into(),
            signature: Some("sig999".into()),
            redacted: false,
        };
        let wire = block_to_anthropic(&block);
        assert_eq!(wire.get("type").and_then(Value::as_str), Some("thinking"));
        assert_eq!(
            wire.get("thinking").and_then(Value::as_str),
            Some("prior thought")
        );
        assert_eq!(
            wire.get("signature").and_then(Value::as_str),
            Some("sig999")
        );
    }

    #[test]
    fn redacted_reasoning_block_replays_as_redacted_thinking() {
        let block = ContentBlock::Reasoning {
            text: String::new(),
            signature: Some("ENCRYPTED".into()),
            redacted: true,
        };
        let wire = block_to_anthropic(&block);
        assert_eq!(
            wire.get("type").and_then(Value::as_str),
            Some("redacted_thinking")
        );
        assert_eq!(wire.get("data").and_then(Value::as_str), Some("ENCRYPTED"));
    }

    #[test]
    fn image_block_maps_to_anthropic_base64_source() {
        // P-11, Tech Spec §4.2: the Image variant maps to the Anthropic
        // `image` block with a `base64` source shape.
        let block = ContentBlock::Image {
            media_type: "image/png".into(),
            data: "iVBOR".into(),
        };
        let wire = block_to_anthropic(&block);
        assert_eq!(wire.get("type").and_then(Value::as_str), Some("image"));
        let source = wire.get("source");
        assert_eq!(
            source.and_then(|s| s.get("type")).and_then(Value::as_str),
            Some("base64")
        );
        assert_eq!(
            source
                .and_then(|s| s.get("media_type"))
                .and_then(Value::as_str),
            Some("image/png")
        );
        assert_eq!(
            source.and_then(|s| s.get("data")).and_then(Value::as_str),
            Some("iVBOR")
        );
    }

    #[test]
    fn document_block_maps_to_anthropic_base64_source() {
        // P-12, Tech Spec §4.2: the Document variant maps to the Anthropic
        // `document` block with a `base64` source shape.
        let block = ContentBlock::Document {
            media_type: "application/pdf".into(),
            data: "JVBERi0".into(),
        };
        let wire = block_to_anthropic(&block);
        assert_eq!(wire.get("type").and_then(Value::as_str), Some("document"));
        let source = wire.get("source");
        assert_eq!(
            source.and_then(|s| s.get("type")).and_then(Value::as_str),
            Some("base64")
        );
        assert_eq!(
            source
                .and_then(|s| s.get("media_type"))
                .and_then(Value::as_str),
            Some("application/pdf")
        );
        assert_eq!(
            source.and_then(|s| s.get("data")).and_then(Value::as_str),
            Some("JVBERi0")
        );
    }
}
