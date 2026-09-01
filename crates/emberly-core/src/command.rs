//! [`Command`] — the messages a frontend sends *into* the engine (Tech Spec
//! §2). The mirror of [`UiEvent`](crate::event::UiEvent): together they are
//! the entire engine/frontend boundary (A-1). The engine cannot tell which
//! kind of frontend produced a command, which is what makes headless and IDE
//! frontends possible without engine changes.

use serde::{Deserialize, Serialize};

use emberly_tools::{MemoryOp, MemoryScope};

use crate::id::{AskId, PermissionId, SessionId};
use crate::types::{AskAnswer, Effort, GateResolution, LoopResolution, Mode, PermissionDecision};

/// A command issued to the engine. `#[non_exhaustive]` so new commands are not
/// a breaking change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Command {
    /// The user submitted a prompt. Starts (or continues) the agent loop.
    /// Any images staged by a prior `Command::AttachImage` (FR-10, Design
    /// §4.14) ride along automatically — the engine holds that staging list
    /// itself (`Engine::pending_attachments`) and drains it into this
    /// message's content, so this variant's shape is unchanged and every
    /// existing caller is unaffected.
    UserInput { text: String },

    /// Attach an image to the prompt currently being composed (FR-10, Design
    /// §4.14), before it is sent. `path` may be any filesystem path the user
    /// names directly — this is a **user**-initiated action, not an
    /// agent-initiated one, so no project-root confinement or permission
    /// prompt applies (the same posture as `emberly export`'s output path,
    /// FR-12). Validated and encoded immediately (format/size caps shared
    /// with T-12; `image.max_attachments` enforced against the engine's own
    /// pending count) so the frontend can show the attachment chip before
    /// the message goes out; the engine replies with `UiEvent::ImageAttached`
    /// or `UiEvent::AttachFailed` and holds the result until the next
    /// `Command::UserInput` consumes it.
    AttachImage { path: String },

    /// The user's answer to a pending permission request, correlated by `id`
    /// with the [`UiEvent::PermissionRequest`](crate::event::UiEvent::PermissionRequest)
    /// that raised it.
    PermissionAnswer {
        id: PermissionId,
        decision: PermissionDecision,
    },

    /// The user's answer to a pending `ask_user` question (T-8), correlated by
    /// `id` with the [`UiEvent::AskUserRequest`](crate::event::UiEvent::AskUserRequest)
    /// that raised it. Carries the typed answer or an explicit
    /// [`Declined`](crate::types::AskAnswer::Declined); Enter never auto-answers
    /// (Design §5.1).
    AskUserAnswer { id: AskId, answer: AskAnswer },

    /// The user's decision after the loop guardrail halted a non-progressing
    /// loop (S-5, Design §8.5): resume, stop, or steer. The engine is parked on
    /// this after emitting [`UiEvent::LoopHalted`](crate::event::UiEvent::LoopHalted).
    ResolveLoop { resolution: LoopResolution },

    /// The user's decision after the completion gate halted on `max_attempts`
    /// failed completion attempts (S-6, Design §8.7): keep-going, stop, steer,
    /// or finish (the override). The engine is parked on this after emitting
    /// [`UiEvent::CompletionGateHalted`](crate::event::UiEvent::CompletionGateHalted).
    ResolveCompletionGate { resolution: GateResolution },

    /// Change the auto-accept mode (Requirements §6.4). The engine validates the
    /// transition against the sandbox status.
    SetMode { mode: Mode },

    /// Switch the active provider profile — and optionally the model — for
    /// subsequent turns (C-6). `model` `None` keeps the current model id.
    /// Issued at idle (the frontend gates it while a turn runs); applies to the
    /// next turn and never rewrites prior turns.
    SwitchModel {
        profile: String,
        model: Option<String>,
    },

    /// Set the reasoning-effort level for subsequent turns (C-6, P-9). Issued at
    /// idle (the frontend gates it while a turn runs); applies to the next turn
    /// and never rewrites prior turns. A model without an effort control accepts
    /// the setting silently — it just never reaches the wire (P-9).
    SetEffort { effort: Effort },

    /// Re-read config + prompts from disk and apply them to the running session
    /// (C-5) — sent by the frontend after an in-app edit. Live pieces (prompts,
    /// provider profiles) take effect on the next turn; restart-only changes are
    /// named, not applied.
    ReloadConfig,

    /// Request manual compaction (Requirements §8.3). Queued until a clean
    /// message boundary if invoked mid-run.
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

    /// Request the memory inspector's grouped entry list (FR-6, Design §4.9).
    /// A user/TUI action, not a model turn — issued at idle; the engine replies
    /// with [`UiEvent::MemoryEntries`](crate::event::UiEvent::MemoryEntries).
    /// The model's `memory` tool never lists (its `MemoryOp` enum is unchanged);
    /// listing lives here so the tool surface stays minimal.
    MemoryList,

    /// Commit an inspector edit or delete to a memory entry (FR-6, C-5, §4.6).
    /// Routed through the **same** validated write path as the `memory` tool
    /// (`execute_memory_op`), so the harness — never the TUI — performs the
    /// write (FR-6): the name is re-slugged, the scope is fixed, and
    /// `MemoryStatus` re-emits after. Only `Update`/`Remove` are meaningful from
    /// the inspector (there is no create-from-empty in the overlay); a `Recall`
    /// here would be a no-op mutation and `Write` is the tool's job.
    MemoryMutate {
        op: MemoryOp,
        scope: MemoryScope,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
        type_: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },

    /// Fetch a single memory entry's body for the inspector's view/edit step
    /// (FR-6, §4.6). Issued at idle; the engine reads the body via the store's
    /// `Recall` and replies with [`UiEvent::MemoryBody`](crate::event::UiEvent::MemoryBody).
    /// Kept off the model-facing `MemoryOp` enum (the model has its own `recall`);
    /// this is a user/TUI read like `MemoryList`. The body is fetched on demand
    /// so it never enters standing context (progressive disclosure).
    MemoryView { scope: MemoryScope, name: String },

    /// Fetch a skill's instruction body for the inspector (FR-7, Design §4.9).
    /// Issued at idle; the engine resolves it through the catalog (project
    /// precedence + untrusted-root gating still apply) and replies with
    /// [`UiEvent::SkillBody`](crate::event::UiEvent::SkillBody). Fetching the
    /// body for display runs no bundled script (FR-7).
    InspectSkill { name: String },

    /// Fetch a subagent's own activity for the sidebar Agents inspector (FR-9,
    /// Tech Spec §8.4, Design §4.13). Issued at idle; the engine reads that
    /// subagent's own nested transcript (the same file its own `Engine`
    /// writes, Tech Spec §3.2) and replies with
    /// [`UiEvent::AgentActivity`](crate::event::UiEvent::AgentActivity) — a
    /// read-only snapshot as of the last flush, not a continuously live
    /// stream. An unknown or already-ended id still replies, with a body
    /// saying so, rather than doing nothing.
    InspectAgent { id: String },

    /// Export this session — plus any subagents it spawned (FR-9) — to a
    /// self-contained HTML file at `path` (FR-12, Design §8.11, Tech Spec
    /// §8.6). Issued at idle; reads the session's own transcript and derived
    /// view read-only, never mutating either. `path` may fall outside the
    /// project root — a user-initiated action, so HC-4 does not apply (the
    /// same posture as `emberly clean`'s scratch-path handling, FR-8). Replies
    /// with [`UiEvent::SessionExported`](crate::event::UiEvent::SessionExported)
    /// on success or a [`UiEvent::Notice`](crate::event::UiEvent::Notice) on
    /// an ordinary I/O failure — never a crash (HC-3).
    ExportSession { path: String },
}
