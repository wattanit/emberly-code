//! Normalized request types (P-1). These are the harness's own message and
//! tool-schema vocabulary; every provider maps its wire format to and from
//! these, and no provider-specific type ever leaks past this layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::id::ToolCallId;

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
        }
    }
}
