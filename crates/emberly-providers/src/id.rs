//! [`ToolCallId`] lives in this crate because tool-call ids originate on the
//! provider wire (Anthropic `tool_use.id`, OpenAI `tool_calls[].id`) and must
//! be echoed back verbatim. `emberly-core` re-exports it, so the engine and
//! the provider layer share one definition rather than translating at the
//! boundary (dependency flow: core → providers).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Correlates a tool call with its result across the provider stream, the
/// engine event model, and the transcript. Opaque: we never parse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallId(pub String);

impl ToolCallId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
