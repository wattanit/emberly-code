//! [`TranscriptRecord`] / [`TranscriptEvent`] — the durable, append-only
//! record (Requirements HC-7, §8.2; Tech Spec §3.2). One JSON object per line
//! in `.agents/sessions/<session-id>.jsonl`. The transcript is ground truth;
//! the in-context conversation is a derived view over it and never rewrites a
//! line.
//!
//! Group 1 defines the schema (types serializable from day one, A-3). The
//! writer, sidecar files, and resume/replay land in Phase 5.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::id::{PermissionId, SessionId, ToolCallId};
use crate::types::{Mode, PermissionDecision, PermissionRendering, SandboxStatus};

/// Current transcript schema version. Present on every record from day one so
/// a reader can detect and warn on newer schemas rather than crash (Tech Spec
/// §3.3).
pub const SCHEMA_VERSION: u32 = 1;

/// One line of the transcript: the schema version, a timestamp, and the event
/// itself flattened alongside them, producing
/// `{"v":1,"ts":"…","type":"…", …}` (Tech Spec §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptRecord {
    /// Schema version (Tech Spec §3.2 `v`).
    pub v: u32,
    /// Event time, RFC 3339 UTC.
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    /// The event, flattened so its `type` tag and fields sit at the top level.
    #[serde(flatten)]
    pub event: TranscriptEvent,
}

impl TranscriptRecord {
    /// Wrap an event with the current schema version and a timestamp.
    #[must_use]
    pub fn new(ts: OffsetDateTime, event: TranscriptEvent) -> Self {
        Self {
            v: SCHEMA_VERSION,
            ts,
            event,
        }
    }
}

/// A durable transcript event. Internally tagged on `type` (snake_case).
///
/// Unlike [`UiEvent`](crate::event::UiEvent), assistant text is stored
/// *complete* (not as deltas): the transcript records what was said, not how
/// it streamed. `#[non_exhaustive]` for forward compatibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TranscriptEvent {
    /// Opens a session: identity and the environment it started in.
    SessionStart {
        session_id: SessionId,
        provider: String,
        model: String,
        project_root: String,
        sandbox: SandboxStatus,
        /// One entry per configuration piece whose source is not the baked-in
        /// default (Requirements C-3). Empty when everything is default.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        config_provenance: Vec<ConfigProvenance>,
    },

    /// A user message. The first user message of a session is the original
    /// task and is pinned (never compacted) — flagged here so the view
    /// rebuilder can honor that (Requirements §8.3, Tech Spec §7).
    UserMessage {
        text: String,
        #[serde(default, skip_serializing_if = "is_false")]
        original_task: bool,
    },

    /// A complete assistant message (post-stream).
    AssistantMessage { text: String },

    /// A tool invocation the model requested.
    ToolCall {
        call_id: ToolCallId,
        tool: String,
        args: serde_json::Value,
    },

    /// The result handed back to the model. Records whether the result was
    /// truncated at ingestion and, if so, where the full output lives
    /// (Requirements §8.1, Tech Spec §5.3). `ok` marks success vs a structured
    /// failure payload (HC-6).
    ToolResult {
        call_id: ToolCallId,
        ok: bool,
        output: String,
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        full_output_ref: Option<String>,
    },

    /// A permission prompt was raised (what was asked).
    PermissionRequest {
        id: PermissionId,
        rendering: PermissionRendering,
    },

    /// A permission prompt was resolved: what was asked, what the user
    /// answered, and what actually ran (Requirements §6.6).
    PermissionDecision {
        id: PermissionId,
        decision: PermissionDecision,
        /// The command/action that actually executed after the decision, if
        /// any (denials execute nothing).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        executed: Option<String>,
    },

    /// The auto-accept mode changed (Requirements §6.4).
    ModeChange { mode: Mode },

    /// A `/compact` occurred: the summary text and the range of view turns it
    /// replaced (Requirements §8.3). The JSONL log itself is untouched.
    Compaction {
        summary: String,
        replaced_from: u32,
        replaced_to: u32,
    },

    /// The session title was set or renamed (Requirements §8.2).
    SessionTitle { title: String },

    /// Clean session end.
    SessionEnd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Written by the supervisor on an abnormal exit when possible
    /// (Requirements HC-3, S-2).
    AbnormalExit { reason: String },
}

/// Records which configuration tier an active piece came from, for provenance
/// reporting (Requirements C-3, Tech Spec §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProvenance {
    /// What the piece is, e.g. `system_prompt`, `pricing.model-x`.
    pub piece: String,
    /// Where it came from, e.g. `project:.agents/config.toml`, `default`.
    pub source: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // signature required by serde's skip_serializing_if
fn is_false(b: &bool) -> bool {
    !*b
}
