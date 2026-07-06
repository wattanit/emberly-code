//! [`UiEvent`] — the ephemeral, high-frequency events the engine emits to a
//! frontend (Tech Spec §3.1). Distinct from [`crate::transcript`] events,
//! which are the durable, append-only record: UI streaming and transcript
//! durability have incompatible granularity (deltas vs complete messages), so
//! they are separate types on purpose.
//!
//! Serializable from day one (A-3) — this is what makes a headless frontend
//! and event-replay testing (A-2) cheap rather than bolted on.

use serde::{Deserialize, Serialize};

use crate::id::{PermissionId, SessionId, ToolCallId};
use crate::types::{Mode, PermissionRendering, SandboxStatus, TokenUsage};

/// An event emitted by the engine for a frontend to render.
///
/// `#[non_exhaustive]` (Tech Spec §3.1): frontends must handle unknown future
/// variants gracefully, and new variants are not a breaking change. Internally
/// tagged on `kind` for stable, self-describing JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UiEvent {
    /// A chunk of streaming assistant text. High-frequency.
    AssistantDelta { text: String },
    /// The assistant's turn finished streaming (no more deltas for this turn).
    AssistantDone,

    /// The whole turn is complete and the engine is idle again (the model
    /// stopped without more tool calls, or the turn errored/was canceled). Lets
    /// a frontend stop its "working" affordance (Design §6.3). One per turn.
    TurnEnded,

    /// A tool began executing.
    ToolStarted {
        call_id: ToolCallId,
        tool: String,
        /// One-line human summary (e.g. `read src/main.rs`).
        summary: String,
    },
    /// A tool finished. `ok` distinguishes a success payload from a structured
    /// failure payload — both are normal data to the model (HC-6), never a
    /// harness error. `preview` is a short, already-truncated excerpt of the
    /// result for the conversation, so the user sees what the tool produced
    /// (Design §6.1) without the frontend holding the full output.
    ToolFinished {
        call_id: ToolCallId,
        ok: bool,
        summary: String,
        preview: String,
    },

    /// The engine needs a permission decision before proceeding. The frontend
    /// renders `rendering` in full and replies with a
    /// [`Command::PermissionAnswer`](crate::command::Command::PermissionAnswer)
    /// carrying the same `id`.
    PermissionRequest {
        id: PermissionId,
        rendering: PermissionRendering,
    },

    /// Context-window usage against the budget (Requirements §8.4). Always
    /// visible in the UI; invisible exhaustion is a defect.
    ContextUsage { pct: u8, tokens: u64 },

    /// Running session cost estimate (Requirements P-6, Design §3.1). Always
    /// labeled "est." in the UI. Emitted from Phase 3 onward.
    CostEstimate { usage: TokenUsage, usd: f64 },

    /// Sandbox status changed or was (re)probed (Requirements §6.7). Emitted
    /// from Phase 2 onward.
    SandboxStatus { status: SandboxStatus },

    /// The auto-accept mode changed (Requirements §6.4). Emitted from Phase 2.
    ModeChanged { mode: Mode },

    /// A harness-world failure (network, provider, bug) — distinct from an
    /// agent-world tool failure. Answers what happened, why, and what to do
    /// next (Design §6.1). The type makes the "next step" non-optional.
    HarnessError {
        what: String,
        why: String,
        next: String,
    },

    /// A retryable failure is being retried (Tech Spec §4.3, S-3). Surfaced as
    /// a dimmed harness-voice line so retries are never silent (Design §6.1).
    Retrying {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        reason: String,
    },

    /// Session metadata for the sidebar/header (Design §3.1): id, title,
    /// provider, model, project root. Title is auto-generated and renamable.
    SessionMeta {
        session_id: SessionId,
        title: String,
        provider: String,
        model: String,
        project_root: String,
    },

    /// A file was created or changed this session, with line deltas, for the
    /// sidebar's modified-files list (Design §3.1).
    FileModified { path: String, adds: u32, dels: u32 },

    /// The unified diff of a file change, for inline display and the diff
    /// overlay (Design §4.2). Emitted alongside [`UiEvent::FileModified`] when
    /// the tool supplied a diff. Separate from `FileModified` so a frontend
    /// that only wants line counts can ignore it.
    FileDiff { path: String, unified: String },

    /// Progress/outcome of a `/compact` operation (Requirements §8.3). Emitted
    /// from Phase 5 onward.
    CompactionStatus { message: String },
}
