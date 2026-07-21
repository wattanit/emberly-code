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
use crate::model::{Effort, ModelInfo, ProviderId, TokenEstimate, TokenUsage};
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
        let body = build_body(&request, &self.model_info.effort_levels);
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

/// The OpenAI `reasoning_effort` value for an effort level. The field tops out
/// at `high`, so `Max` maps to it (Tech Spec §4.6).
fn reasoning_effort_value(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High | Effort::Max => "high",
    }
}

/// Build the Chat Completions body. `effort_levels` is the active model's
/// declared support (empty ⇒ no reasoning control, so an `effort` on the
/// request is silently ignored — P-9).
fn build_body(request: &CompletionRequest, effort_levels: &[Effort]) -> Value {
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
    // Reasoning effort → `reasoning_effort`, only if the model declares support
    // (P-9, Tech Spec §4.6).
    if let Some(effort) = request.effort.filter(|_| !effort_levels.is_empty()) {
        body["reasoning_effort"] = json!(reasoning_effort_value(effort));
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
        Role::User => json!({ "role": "user", "content": user_content(message) }),
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

/// Build the `content` value for a `user`-role message (P-11/P-12, Tech Spec
/// §4.2). When the message carries any image or document block, returns an
/// array of text + media parts; otherwise returns a plain string for
/// back-compat with every existing text-only turn. OpenAI tool-role messages
/// cannot carry image or document parts.
fn user_content(message: &Message) -> Value {
    let has_media = message.content.iter().any(|block| {
        matches!(
            block,
            ContentBlock::Image { .. } | ContentBlock::Document { .. }
        )
    });
    if !has_media {
        return Value::String(joined_text(message));
    }
    let parts: Vec<Value> = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::Image { media_type, data } => Some(json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{media_type};base64,{data}")
                }
            })),
            // A document maps to the endpoint's file input part (P-12, Tech
            // Spec §4.2). `ContentBlock::Document` carries no filename (HC-2 —
            // the harness never tracks more than media type + bytes), so a
            // fixed, format-matching name is used.
            ContentBlock::Document { media_type, data } => Some(json!({
                "type": "file",
                "file": {
                    "filename": "document.pdf",
                    "file_data": format!("data:{media_type};base64,{data}")
                }
            })),
            _ => None,
        })
        .collect();
    Value::Array(parts)
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

        // Reasoning, where the server exposes it distinctly (P-10). OpenAI-
        // compatible endpoints are inconsistent here — `reasoning_content`
        // (DeepSeek) and `reasoning` are both seen — so accept either. No
        // signature echo is required on this wire.
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
        {
            if !reasoning.is_empty() {
                out.push(Ok(StreamEvent::ReasoningDelta {
                    text: reasoning.to_string(),
                }));
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let function = &call["function"];
                if let std::collections::hash_map::Entry::Vacant(entry) = self.tool_ids.entry(index)
                {
                    // Some OpenAI-compatible backends omit `id` (or send it
                    // empty/null) on every chunk of a parallel tool call, not
                    // just the later ones the spec allows dropping it on. An
                    // empty-string fallback would let two such calls collide
                    // on the same id downstream (engine.rs matches
                    // ToolCallDelta by id equality), silently merging their
                    // argument buffers — e.g. two concurrent `write_file`
                    // calls' JSON interleaving into one, corrupting content.
                    // `index` is unique per parallel call by construction, so
                    // fall back to it instead of a shared empty id.
                    let id = ToolCallId::new(
                        call.get("id")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map_or_else(|| format!("call_{index}"), str::to_string),
                    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with_effort(effort: Option<Effort>) -> CompletionRequest {
        let mut r = CompletionRequest::new("gpt-x");
        r.effort = effort;
        r
    }

    fn effort_field(body: &Value) -> Option<&str> {
        body.get("reasoning_effort").and_then(Value::as_str)
    }

    #[test]
    fn maps_each_level_when_supported_max_folds_to_high() {
        let levels = Effort::ALL.to_vec();
        for (effort, want) in [
            (Effort::Low, "low"),
            (Effort::Medium, "medium"),
            (Effort::High, "high"),
            (Effort::Max, "high"),
        ] {
            let body = build_body(&req_with_effort(Some(effort)), &levels);
            assert_eq!(effort_field(&body), Some(want), "level {effort}");
        }
    }

    #[test]
    fn omits_reasoning_effort_when_no_effort() {
        let body = build_body(&req_with_effort(None), &Effort::ALL);
        assert_eq!(body.get("reasoning_effort"), None);
    }

    #[test]
    fn unsupported_model_is_a_noop_even_with_effort() {
        // Empty effort_levels ⇒ the model has no reasoning control (P-9).
        let body = build_body(&req_with_effort(Some(Effort::High)), &[]);
        assert_eq!(body.get("reasoning_effort"), None);
    }

    fn map_one(data: &str) -> Vec<StreamEvent> {
        let mut mapper = OpenAiMapper::default();
        mapper
            .map(SseEvent {
                event: None,
                data: data.to_string(),
            })
            .into_iter()
            .filter_map(Result::ok)
            .collect()
    }

    #[test]
    fn reasoning_content_delta_maps_to_reasoning_delta() {
        let events =
            map_one(r#"{"choices":[{"delta":{"reasoning_content":"pondering"},"index":0}]}"#);
        assert_eq!(
            events,
            vec![StreamEvent::ReasoningDelta {
                text: "pondering".into()
            }]
        );
    }

    #[test]
    fn plain_content_delta_emits_no_reasoning() {
        let events = map_one(r#"{"choices":[{"delta":{"content":"hi"},"index":0}]}"#);
        assert_eq!(events, vec![StreamEvent::TextDelta { text: "hi".into() }]);
    }

    #[test]
    fn parallel_tool_calls_get_distinct_ids_when_backend_omits_id() {
        // Regression: some OpenAI-compatible backends never send `id` on a
        // tool_calls chunk, not even the first one for a given index. A
        // shared empty-string fallback id would make engine.rs's id-keyed
        // buffer lookup merge two concurrent calls' argument deltas into one
        // buffer, corrupting content (e.g. two `write_file` calls' JSON
        // interleaving, garbling embedded `\n` escapes).
        let mut mapper = OpenAiMapper::default();
        let feed = |mapper: &mut OpenAiMapper, data: Value| -> Vec<StreamEvent> {
            mapper
                .map(SseEvent {
                    event: None,
                    data: data.to_string(),
                })
                .into_iter()
                .filter_map(Result::ok)
                .collect()
        };

        let start0 = feed(
            &mut mapper,
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"name": "write_file", "arguments": ""}}
            ]}}]}),
        );
        let start1 = feed(
            &mut mapper,
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 1, "function": {"name": "write_file", "arguments": ""}}
            ]}}]}),
        );
        let id0 = match &start0[0] {
            StreamEvent::ToolCallStart { id, .. } => id.clone(),
            other => panic!("expected ToolCallStart, got {other:?}"),
        };
        let id1 = match &start1[0] {
            StreamEvent::ToolCallStart { id, .. } => id.clone(),
            other => panic!("expected ToolCallStart, got {other:?}"),
        };
        assert_ne!(
            id0, id1,
            "parallel calls must not collide on a shared fallback id"
        );

        let args0 = r#"{"path":"a.txt","content":"line1\nline2"}"#;
        let args1 = r#"{"path":"b.txt","content":"other\ncontent"}"#;
        let delta0 = feed(
            &mut mapper,
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": args0}}
            ]}}]}),
        );
        let delta1 = feed(
            &mut mapper,
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 1, "function": {"arguments": args1}}
            ]}}]}),
        );

        match &delta0[0] {
            StreamEvent::ToolCallDelta { id, args_delta } => {
                assert_eq!(*id, id0);
                assert_eq!(args_delta, args0);
            }
            other => panic!("expected ToolCallDelta, got {other:?}"),
        }
        match &delta1[0] {
            StreamEvent::ToolCallDelta { id, args_delta } => {
                assert_eq!(*id, id1);
                assert_eq!(args_delta, args1);
            }
            other => panic!("expected ToolCallDelta, got {other:?}"),
        }
    }

    #[test]
    fn text_only_user_message_stays_a_plain_string() {
        // Back-compat: no image ⇒ `content` is a string, not an array (P-11).
        let msg = Message::user_text("hello");
        let content = user_content(&msg);
        assert_eq!(content, Value::String("hello".into()));
    }

    #[test]
    fn image_in_user_message_produces_image_url_data_uri() {
        // P-11, Tech Spec §4.2: an image in a user message maps to an
        // `image_url` part with a `data:` URI.
        let msg = Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "what is this?".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "iVBOR".into(),
                },
            ],
        };
        let content = user_content(&msg);
        let Some(parts) = content.as_array() else {
            panic!("array when image present");
        };
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            parts[1].get("type").and_then(Value::as_str),
            Some("image_url")
        );
        let url = parts[1]
            .get("image_url")
            .and_then(|iu| iu.get("url"))
            .and_then(Value::as_str);
        assert_eq!(url, Some("data:image/png;base64,iVBOR"));
    }

    #[test]
    fn document_in_user_message_produces_file_data_uri() {
        // P-12, Tech Spec §4.2: a document in a user message maps to a `file`
        // part with a `file_data` `data:` URI — the same structured-content
        // path a document triggers as an image does (the `has_media` guard).
        let msg = Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "what does this say?".into(),
                },
                ContentBlock::Document {
                    media_type: "application/pdf".into(),
                    data: "JVBERi0".into(),
                },
            ],
        };
        let content = user_content(&msg);
        let Some(parts) = content.as_array() else {
            panic!("array when document present");
        };
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(parts[1].get("type").and_then(Value::as_str), Some("file"));
        let file = parts[1].get("file");
        assert_eq!(
            file.and_then(|f| f.get("filename")).and_then(Value::as_str),
            Some("document.pdf")
        );
        let file_data = file
            .and_then(|f| f.get("file_data"))
            .and_then(Value::as_str);
        assert_eq!(file_data, Some("data:application/pdf;base64,JVBERi0"));
    }
}
