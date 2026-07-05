//! [`Command`] — the messages a frontend sends *into* the engine (Tech Spec
//! §2). The mirror of [`UiEvent`](crate::event::UiEvent): together they are
//! the entire engine/frontend boundary (A-1). The engine cannot tell which
//! kind of frontend produced a command, which is what makes headless and IDE
//! frontends possible without engine changes.

use serde::{Deserialize, Serialize};

use crate::id::PermissionId;
use crate::types::{Mode, PermissionDecision};

/// A command issued to the engine. `#[non_exhaustive]` so new commands are not
/// a breaking change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Command {
    /// The user submitted a prompt. Starts (or continues) the agent loop.
    UserInput { text: String },

    /// The user's answer to a pending permission request, correlated by `id`
    /// with the [`UiEvent::PermissionRequest`](crate::event::UiEvent::PermissionRequest)
    /// that raised it.
    PermissionAnswer {
        id: PermissionId,
        decision: PermissionDecision,
    },

    /// Change the auto-accept mode (Requirements §6.4). Handled from Phase 2;
    /// the engine validates the transition against the sandbox status.
    SetMode { mode: Mode },

    /// Request manual compaction (Requirements §8.3). Handled from Phase 5;
    /// queued until a clean message boundary if invoked mid-run.
    Compact,

    /// Cancel the in-flight turn (Esc / Ctrl+C). Handled at the next await
    /// point; a running child process is killed by process group (S-4).
    Cancel,
}
