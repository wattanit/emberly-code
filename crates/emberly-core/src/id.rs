//! Identifier newtypes shared across the event model.
//!
//! Newtypes rather than bare `String`/`u64` so the compiler prevents mixing a
//! tool-call id with a permission id, and so their JSON representation is
//! fixed in one place (HC-7 transcript longevity).

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique id for a session. Backs the transcript filename
/// (`.agents/sessions/<session-id>.jsonl`, Tech Spec §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Generate a fresh random (v4) session id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Correlates a `tool_call` with its `tool_result` across both event streams.
/// A `String` because provider wire protocols hand us opaque call ids we must
/// echo back verbatim (Anthropic `tool_use.id`, OpenAI `tool_calls[].id`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallId(pub String);

impl fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Correlates a `PermissionRequest` event with the `PermissionAnswer` command
/// that resolves it. Engine-assigned and monotonic within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermissionId(pub u64);

impl fmt::Display for PermissionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
