//! Shared domain value types referenced by more than one part of the event
//! model (`UiEvent`, `Command`, `TranscriptEvent`). Kept in one module so a
//! concept like "mode" or "sandbox status" has a single definition.

use serde::{Deserialize, Serialize};

/// Auto-accept escalation tier and OS-level confinement status. Both are
/// *produced by* the sandbox (mode transitions are confinement-gated; the
/// status is the probe's output), so they live in `emberly-sandbox` and are
/// re-exported here — the event/command/transcript schemas reference them
/// through core unchanged (mirrors the `TokenUsage` re-export from
/// `emberly-providers`).
pub use emberly_sandbox::{Mode, SandboxStatus};

/// Everything the frontend needs to render a permission prompt fully, without
/// reaching back into the engine (Design §5: "saying yes always requires
/// having seen what you are saying yes to").
///
/// The engine builds this; both the line-mode frontend and the rich TUI render
/// it. It carries content, never a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRendering {
    /// Tool being invoked (e.g. `bash`, `write_file`).
    pub tool: String,
    /// One-line summary for compact display (e.g. `run: cargo test`).
    pub summary: String,
    /// The complete content the user must see: the full command, or the full
    /// diff. Never truncated to fit — the frontend scrolls it (Design §5).
    pub detail: String,
    /// Filesystem paths this action affects, for the prompt to list.
    pub affected_paths: Vec<String>,
    /// True when any affected path is outside the project root (Requirements
    /// HC-4). Triggers the reserved, visually-loud safety styling (Design §5)
    /// and can never be auto-approved.
    pub outside_root: bool,
    /// Why this prompt appeared — the rule that matched, or "outside project
    /// root". Shown as one dimmed line to teach the model in situ (Design §5).
    pub reason: String,
    /// The subagent this request came from, if any (FR-9, Tech Spec §8.4) —
    /// `None` for the primary agent's own requests. Lets the frontend add one
    /// dimmed "on behalf of subagent X" line (Design §4.13/§5) without a
    /// second prompt type or diluting the reserved outside-root styling.
    /// Additive: absent on older transcript records reads as `None`.
    #[serde(default)]
    pub on_behalf_of: Option<String>,
}

/// The user's answer to a permission request. Deny is the safe default and
/// the meaning of Enter/Esc (Design §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Refuse this action. Returned to the model as structured data, never a
    /// silent drop (Requirements §6.6, HC-6).
    Deny,
    /// Allow this single invocation.
    AllowOnce,
    /// Allow for the rest of this session (in-memory grant).
    AllowForSession,
    /// Persist an allow rule to project `permissions.toml`.
    AlwaysAllowInProject,
}

impl PermissionDecision {
    /// Whether this decision permits the action to run.
    #[must_use]
    pub fn is_allow(self) -> bool {
        !matches!(self, Self::Deny)
    }
}

/// The user's reply to an `ask_user` question (T-8). Unlike a permission
/// decision, this carries data and has no unsafe default: there is no keypress
/// that answers for the user; a dismissal is an explicit
/// [`Declined`](AskAnswer::Declined) (Design §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskAnswer {
    /// The user's answer: free text, or the label of a chosen option.
    Answered(String),
    /// The user dismissed the question without answering.
    Declined,
}

/// The user's decision after the loop-breaking guardrail halts a non-progressing
/// loop (S-5, Design §8.5). The guardrail never resumes or abandons on its own —
/// the user always chooses one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopResolution {
    /// Keep going — continue the loop from where it halted.
    Resume,
    /// Stop here — end the turn cleanly.
    Stop,
    /// Hand a steer back to the model: push this text as a user message and
    /// continue.
    Steer(String),
}

/// The outcome of one completion-gate check evaluation (S-6, Tech Spec §3.2,
/// §7): its name, whether it passed, and a human-readable reason (built from
/// the exit status plus a reduced output tail on failure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub reason: String,
}

/// The user's decision after the completion gate halts on `max_attempts`
/// failed attempts (S-6, Design §8.7). Mirrors [`LoopResolution`] with one
/// addition: `Finish` is the user's override — end the task as done over a
/// still-failing gate. The gate never resumes or abandons on its own; the user
/// always chooses one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateResolution {
    /// Keep going — try the completion attempt again.
    Resume,
    /// Stop here — end the turn cleanly, gate still unsatisfied.
    Stop,
    /// Hand a steer back to the model: push this text as a user message and
    /// try again.
    Steer(String),
    /// End the task as done over the still-failing gate — the user's override,
    /// never presented as though the checks passed (Design §8.7).
    Finish,
}

/// Provider-reported or estimated token counts for a completion
/// (Requirements P-6, Tech Spec §4.4). Drives the context indicator and the
/// cost estimate. Defined in `emberly-providers` and re-exported so the two
/// crates agree on the accounting unit.
pub use emberly_providers::TokenUsage;

/// The reasoning-effort level (Requirements P-9, Tech Spec §4.6). Defined in
/// `emberly-providers` (it rides on `CompletionRequest`/`ModelInfo`) and
/// re-exported so commands, events, and the transcript share one type.
pub use emberly_providers::Effort;

/// An image attached to a prompt by the user (FR-10, Tech Spec §4.1/§9),
/// already validated and base64-encoded at attach time (`Command::AttachImage`,
/// via the shared `emberly_tools::image::encode_image_bytes` path). Rides on
/// `Command::UserInput`; the engine turns it into a `ContentBlock::Image` on
/// the outgoing message, told apart from a model-read image structurally (by
/// which message carries it), never by a flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachedImage {
    /// Display name for the chip and transcript record (Design §4.14) — the
    /// original file's name, not a full path.
    pub name: String,
    pub media_type: String,
    pub data: String,
    pub width: usize,
    pub height: usize,
    pub format_label: String,
}

/// The durable, byte-free record of an attached image (Tech Spec §4.1): the
/// same choice T-12's own `ToolResult` already makes for a model-read image —
/// metadata only, never the base64 bytes, so a resumed session's attachment
/// chip is a durable fact even though the pixels are not replayed to the
/// model on resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachedImageMeta {
    pub name: String,
    pub media_type: String,
    pub width: usize,
    pub height: usize,
    pub format_label: String,
}

impl From<&AttachedImage> for AttachedImageMeta {
    fn from(img: &AttachedImage) -> Self {
        Self {
            name: img.name.clone(),
            media_type: img.media_type.clone(),
            width: img.width,
            height: img.height,
            format_label: img.format_label.clone(),
        }
    }
}

/// The result of one MCP server's connection/discovery attempt at session
/// start or `/reload` (FR-11, Tech Spec §5.6/§8.5). Connecting itself is
/// composition-root logic (the `emberly` binary, mirroring how `web_search`
/// is conditionally built, Tech Spec §5.5) — the engine only reports what
/// already happened, as `UiEvent::McpServerConnected`/`McpServerFailed` plus
/// a `TranscriptEvent::McpConnection` audit record (extends HC-7, no
/// exemption for connection lifecycle).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpConnectionOutcome {
    /// The server connected and advertised these (already-namespaced) tools.
    Connected { server: String, tools: Vec<String> },
    /// The server could not be connected to, or its handshake/discovery
    /// failed. Never fatal to the session (Design §8.10 — the rest of the
    /// harness stays usable).
    Failed { server: String, reason: String },
}
