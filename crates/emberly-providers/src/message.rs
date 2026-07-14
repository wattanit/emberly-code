//! Normalized request types (P-1). These are the harness's own message and
//! tool-schema vocabulary; every provider maps its wire format to and from
//! these, and no provider-specific type ever leaks past this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::id::ToolCallId;
use crate::model::Effort;

/// The author of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A piece of message content. Rich enough to represent a tool-use round trip
/// in a provider-agnostic way; the provider clients translate to/from their
/// wire shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text.
    Text { text: String },
    /// A tool the assistant asked to call, with fully-formed arguments.
    ToolUse {
        id: ToolCallId,
        name: String,
        input: Value,
    },
    /// The result of a tool call fed back to the model. `is_error` marks a
    /// structured failure payload (HC-6) — still normal data, not a crash.
    ToolResult {
        call_id: ToolCallId,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// A captured reasoning/thinking block from a prior assistant turn (P-10).
    /// Carried in the conversation so a provider that requires reasoning to be
    /// echoed back on later tool-use turns can replay it. `signature` is an
    /// opaque provider token, never interpreted by the engine (P-1); `redacted`
    /// marks an encrypted block whose `text` was withheld by the provider. An
    /// adapter that has no use for reasoning blocks simply skips them.
    Reasoning {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        redacted: bool,
    },
    /// An image to send to the model (P-11, Tech Spec §4.1). `data` is the
    /// base64 encoding of the file bytes; `media_type` is the MIME string
    /// (`image/png`, `image/jpeg`, `image/gif`, `image/webp`). Each adapter maps
    /// this to its provider's native image content shape (Tech Spec §4.2).
    Image { media_type: String, data: String },
}

/// `skip_serializing_if` helper: omit `redacted` from the wire when false.
#[allow(clippy::trivially_copy_pass_by_ref)] // signature required by serde
fn is_false(b: &bool) -> bool {
    !*b
}

/// One message in the normalized conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    #[must_use]
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// A message carrying a single tool result back to the model.
    #[must_use]
    pub fn tool_result(call_id: ToolCallId, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id,
                content: content.into(),
                is_error,
            }],
        }
    }
}

/// A tool advertised to the model: name, description, and a JSON Schema for
/// its arguments. The engine builds these from `emberly-tools` `ToolSpec`s
/// (the two crates do not depend on each other; the engine bridges them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A normalized completion request. Providers translate this into their wire
/// request; nothing provider-specific appears here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// The reasoning effort for this turn (P-9, Tech Spec §4.6). Each adapter
    /// maps it to its provider's native control or drops it (a no-op for a
    /// model without such a control — never an error). `None` sends nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
}

impl CompletionRequest {
    /// A bare request for `model` with no messages, tools, or overrides.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            max_output_tokens: None,
            temperature: None,
            effort: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_without_effort_field_deserializes_to_none() {
        // A request serialized before Phase 3 carries no `effort` key.
        let json = r#"{"model":"m","messages":[]}"#;
        let req: CompletionRequest = match serde_json::from_str(json) {
            Ok(r) => r,
            Err(e) => panic!("deserialize legacy: {e}"),
        };
        assert_eq!(req.effort, None);
    }

    #[test]
    fn new_request_omits_effort_when_none() {
        let req = CompletionRequest::new("m");
        let json = match serde_json::to_string(&req) {
            Ok(s) => s,
            Err(e) => panic!("serialize: {e}"),
        };
        assert!(!json.contains("effort"), "None effort is skipped: {json}");
    }
}
