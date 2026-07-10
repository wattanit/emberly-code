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
/// The engine builds this; both the line-mode frontend (Phase 1) and the full
/// TUI (Phase 4) render it. It carries content, never a decision.
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
}

/// The user's answer to a permission request. Deny is the safe default and
/// the meaning of Enter/Esc (Design §5).
///
/// Phase 1 wires [`Deny`](PermissionDecision::Deny) and
/// [`AllowOnce`](PermissionDecision::AllowOnce) only; the persisting grants
/// arrive with the rule engine in Phase 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Refuse this action. Returned to the model as structured data, never a
    /// silent drop (Requirements §6.6, HC-6).
    Deny,
    /// Allow this single invocation.
    AllowOnce,
    /// Allow for the rest of this session (in-memory grant; Phase 2).
    AllowForSession,
    /// Persist an allow rule to project `permissions.toml` (Phase 2).
    AlwaysAllowInProject,
}

impl PermissionDecision {
    /// Whether this decision permits the action to run.
    #[must_use]
    pub fn is_allow(self) -> bool {
        !matches!(self, Self::Deny)
    }
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
