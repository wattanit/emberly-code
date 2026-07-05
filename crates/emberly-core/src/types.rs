//! Shared domain value types referenced by more than one part of the event
//! model (`UiEvent`, `Command`, `TranscriptEvent`). Kept in one module so a
//! concept like "mode" or "sandbox status" has a single definition.

use serde::{Deserialize, Serialize};

/// Auto-accept escalation tier (Requirements §6.4, Tech Spec §6.6).
///
/// The auto tiers are only *reachable* while OS confinement is active; that
/// invariant is enforced by the engine at transition time (Phase 2), not by
/// this type. Defined in full here so the event model is stable from day one
/// (A-3), even though only [`Mode::Normal`] is exercised in Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Prompt per the rule layer (Requirements §6.2). The Phase 1 behavior.
    #[default]
    Normal,
    /// File writes/edits inside the project root auto-allow; bash still asks.
    AutoAcceptEdits,
    /// Allowlisted and session-granted bash also auto-runs.
    Auto,
}

/// OS-level confinement status, probed at startup and kept as always-visible
/// state (Requirements §6.7, Tech Spec §6.5). Emitted as a `UiEvent` and
/// written into the transcript `session_start` record.
///
/// Populated for real in Phase 2 (Landlock) / Phase 5 (Seatbelt); defined now
/// so the event and transcript schemas do not change when it arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SandboxStatus {
    /// Full confinement granted by the OS.
    Confined { backend: String },
    /// Confinement active but missing a requested capability (e.g. an older
    /// Landlock ABI); the granted-vs-requested delta is in `missing`.
    Partial { backend: String, missing: String },
    /// No kernel confinement available; the harness runs in the honest
    /// degraded mode (allowlist suspended, auto modes locked).
    Unavailable { reason: String },
}

impl SandboxStatus {
    /// Whether auto-accept modes may be offered (Requirements §6.7).
    #[must_use]
    pub fn allows_auto_modes(&self) -> bool {
        matches!(self, Self::Confined { .. } | Self::Partial { .. })
    }
}

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
