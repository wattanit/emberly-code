//! [`Command`] — the messages a frontend sends *into* the engine (Tech Spec
//! §2). The mirror of [`UiEvent`](crate::event::UiEvent): together they are
//! the entire engine/frontend boundary (A-1). The engine cannot tell which
//! kind of frontend produced a command, which is what makes headless and IDE
//! frontends possible without engine changes.

use serde::{Deserialize, Serialize};

use crate::id::{PermissionId, SessionId};
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

    /// Switch the active provider profile — and optionally the model — for
    /// subsequent turns (C-6). `model` `None` keeps the current model id.
    /// Issued at idle (the frontend gates it while a turn runs); applies to the
    /// next turn and never rewrites prior turns.
    SwitchModel {
        profile: String,
        model: Option<String>,
    },

    /// Request manual compaction (Requirements §8.3). Handled from Phase 5;
    /// queued until a clean message boundary if invoked mid-run.
    Compact,

    /// Cancel the in-flight turn (Esc / Ctrl+C). Handled at the next await
    /// point; a running child process is killed by process group (S-4).
    Cancel,

    /// Start a fresh session in place, saving the current one first (`/new`).
    /// The frontend mints the new id so it can update its own view without a
    /// round trip; the engine ends the current transcript and rolls a new one.
    NewSession { session_id: SessionId },

    /// Resume a saved session by id, replacing the live conversation with the
    /// one rebuilt from that transcript (`/resume` from the picker). The engine
    /// ends the current session and continues appending to the target file.
    ResumeSession { session_id: SessionId },
}
