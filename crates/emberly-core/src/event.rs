//! [`UiEvent`] — the ephemeral, high-frequency events the engine emits to a
//! frontend (Tech Spec §3.1). Distinct from [`crate::transcript`] events,
//! which are the durable, append-only record: UI streaming and transcript
//! durability have incompatible granularity (deltas vs complete messages), so
//! they are separate types on purpose.
//!
//! Serializable from day one (A-3) — this is what makes a headless frontend
//! and event-replay testing (A-2) cheap rather than bolted on.

use serde::{Deserialize, Serialize};

use crate::id::{AskId, PermissionId, SessionId, ToolCallId};
use crate::types::{CheckResult, Effort, Mode, PermissionRendering, SandboxStatus, TokenUsage};

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
    /// A chunk of the model's reasoning/thinking, distinct from the answer
    /// (P-10, Design §4.4). The frontend streams it into the collapsed thinking
    /// trail; it is never rendered as the answer. High-frequency.
    ReasoningDelta { text: String },
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
        /// The model's optional caption for a non-obvious call (T-9, §5.4,
        /// Design §4.5). `None` when the model gave none — the UI shows no
        /// placeholder.
        explanation: Option<String>,
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
        /// Whether the result is untrusted web content (T-14, Design §4.10).
        /// When `true`, the frontend renders it with the untrusted-content
        /// styling — fetched web data, never harness or assistant voice.
        #[serde(default)]
        untrusted: bool,
    },

    /// The engine needs a permission decision before proceeding. The frontend
    /// renders `rendering` in full and replies with a
    /// [`Command::PermissionAnswer`](crate::command::Command::PermissionAnswer)
    /// carrying the same `id`.
    PermissionRequest {
        id: PermissionId,
        rendering: PermissionRendering,
    },

    /// The model is asking the user a question and the loop is blocked until
    /// they answer (T-8, Tech Spec §5.2, Design §5.1). The frontend renders the
    /// question (and `options` as a selectable list when non-empty) with a
    /// free-text answer always available, then replies with an
    /// [`AskUserAnswer`](crate::command::Command::AskUserAnswer) carrying the
    /// same `id`. Calm styling, never the safety band; no unsafe default.
    AskUserRequest {
        id: AskId,
        question: String,
        options: Vec<String>,
    },

    /// The loop-breaking guardrail halted a non-progressing loop (S-5, Tech Spec
    /// §7, Design §8.5). A **harness-world** moment — rendered in the harness's
    /// own out-of-band voice, not as model output, and distinct from the question
    /// prompt. The frontend offers resume / stop / steer and replies with
    /// [`Command::ResolveLoop`](crate::command::Command::ResolveLoop).
    LoopHalted { reason: String },

    /// The completion gate halted after `max_attempts` failed completion
    /// attempts (S-6, Tech Spec §7, Design §8.7). A **harness-world** moment,
    /// the S-6 mirror of [`LoopHalted`]: rendered in the harness's own
    /// out-of-band voice, distinct from a failing check's agent-world
    /// tool-result. The frontend offers keep-going / steer / stop / finish
    /// (an explicit override, never presented as though the checks passed)
    /// and replies with
    /// [`Command::ResolveCompletionGate`](crate::command::Command::ResolveCompletionGate).
    CompletionGateHalted {
        failing: Vec<CheckResult>,
        attempts: usize,
    },

    /// The registered completion checks' most recent results (S-6, Design
    /// §8.7, §3.1), for the sidebar's gate-status line. Emitted after every
    /// completion-gate evaluation (win or lose); never emitted when no checks
    /// are registered, so a session with none shows nothing — like
    /// [`MemoryStatus`](UiEvent::MemoryStatus) and
    /// [`SkillsAvailable`](UiEvent::SkillsAvailable), the sidebar is silently
    /// absent rather than a "None" stub until the gate has run at least once.
    CompletionStatus { checks: Vec<CheckResult> },

    /// Context-window usage against the budget (Requirements §8.4). Always
    /// visible in the UI; invisible exhaustion is a defect.
    ContextUsage { pct: u8, tokens: u64 },

    /// Running session cost estimate (Requirements P-6, Design §3.1). Always
    /// labeled "est." in the UI.
    CostEstimate { usage: TokenUsage, usd: f64 },

    /// Cumulative billed tokens this session (input + output). Emitted on every
    /// accounting update regardless of whether a pricing table exists, so the
    /// sidebar can always show a session total (Design §3.1).
    SessionUsage { usage: TokenUsage },

    /// Sandbox status changed or was (re)probed (Requirements §6.7).
    SandboxStatus { status: SandboxStatus },

    /// The auto-accept mode changed (Requirements §6.4).
    ModeChanged { mode: Mode },

    /// The active provider profile / model changed in-session (C-6, Design
    /// §3.1). The sidebar updates; the switch is also announced via a
    /// [`Notice`](UiEvent::Notice) — never silent.
    ModelChanged { provider: String, model: String },

    /// The configured provider profiles changed after a `/config` reload (C-5),
    /// so the frontend refreshes its model picker.
    ProfilesChanged { profiles: Vec<String> },

    /// The active reasoning-effort level and the levels this model offers
    /// (C-6/P-9, Design §3.1). Emitted at startup, on a set, and on a model
    /// switch, so the sidebar shows the current level and the picker offers the
    /// right options. `effort` is `None` and `available` empty when the model
    /// has no effort control (the sidebar hides the line, the picker declines).
    /// A user-driven change is also announced via a [`Notice`] — never silent.
    EffortChanged {
        effort: Option<Effort>,
        available: Vec<Effort>,
    },

    /// A plain-language, harness-voice notice for the timeline (Design §6.1):
    /// a persisted permission grant's written line (§6.6), a refused auto-mode
    /// switch when degraded (§6.7), or the one-time degraded-sandbox
    /// explanation. Informational — not an error, not a decision.
    Notice { message: String },

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

    /// Progress/outcome of a `/compact` operation (Requirements §8.3).
    CompactionStatus { message: String },

    /// The model updated its task list (T-11, Tech Spec §3.1, Design §4.7). The
    /// full list is sent on every update (replace, not merge). The sidebar's
    /// Tasks section and the inline checklist both render from this. Additive —
    /// older frontends warn-skip it.
    TaskListUpdated { items: Vec<emberly_tools::TaskItem> },

    /// Memory entry counts for the sidebar inspector (T-13, FR-6, Design §4.9).
    /// `user` is the global count; `project` is the project-scoped count (0
    /// when the root is untrusted — Design §4.9 "silently absent, not
    /// half-loaded"). Additive — older frontends warn-skip it.
    MemoryStatus { user: usize, project: usize },

    /// The skill catalog for the sidebar Skills section (T-15, FR-7, Design
    /// §4.9). Emitted at session start and when the set changes. Each entry
    /// carries name, description, and origin (user vs project — origin is how
    /// the user reads trust). Additive — older frontends warn-skip it.
    SkillsAvailable {
        skills: Vec<emberly_tools::SkillMeta>,
    },

    /// The memory inspector's grouped entry list (FR-6, Design §4.9), sent in
    /// reply to [`Command::MemoryList`](crate::command::Command::MemoryList).
    /// Entries are **summaries only** — no bodies, so listing preserves
    /// progressive disclosure (Tech Spec §7/§8.6); a body loads on demand via a
    /// `recall`/edit fetch. `project` is empty on an untrusted root (the
    /// project section is then silently absent — FR-1). Additive — older
    /// frontends warn-skip it.
    MemoryEntries {
        user: Vec<crate::memory::EntrySummary>,
        project: Vec<crate::memory::EntrySummary>,
    },

    /// A single memory entry's body for the inspector's view/edit step (FR-6,
    /// §4.6), sent in reply to [`Command::MemoryView`](crate::command::Command::MemoryView).
    /// The body is fetched on demand and never pinned (progressive disclosure).
    /// `scope`/`name` echo the request so the frontend correlates it with the
    /// summary it selected. An empty `body` means the entry has none (or was
    /// removed between listing and viewing). Additive — older frontends
    /// warn-skip it.
    MemoryBody {
        scope: emberly_tools::MemoryScope,
        name: String,
        body: String,
    },

    /// A skill's instruction body for the inspector (FR-7, Design §4.9), sent in
    /// reply to [`Command::InspectSkill`](crate::command::Command::InspectSkill).
    /// This is the §4.9 promise — "what could this skill tell the model to do"
    /// is inspectable *before it ever runs*. Loading the body for display runs
    /// no bundled script (FR-7). `origin` is how the user reads trust;
    /// `resources` lists bundled file paths. Additive — older frontends
    /// warn-skip it.
    SkillBody {
        name: String,
        origin: emberly_tools::SkillOrigin,
        body: String,
        resources: Vec<String>,
    },

    /// A subagent was created (T-18, Tech Spec §8.4), for the sidebar Agents
    /// section (Design §3.1/§4.13): present only while at least one subagent
    /// is alive, following the established no-empty-stub rule (Tasks/Memory/
    /// Skills). Additive — older frontends warn-skip it.
    SubagentSpawned {
        id: String,
        name: String,
        profile: String,
        model: String,
    },

    /// A subagent was ended (T-21, Tech Spec §8.4) — explicitly, or as part
    /// of the owning session ending. Additive — older frontends warn-skip
    /// it.
    SubagentEnded { id: String, reason: String },

    /// A subagent's own activity, read from its nested transcript (Tech Spec
    /// §8.4), sent in reply to
    /// [`Command::InspectAgent`](crate::command::Command::InspectAgent). A
    /// read-only snapshot as of the last flush (Design §4.13) — `text` is
    /// already formatted for display, so the frontend renders it exactly like
    /// any other text overlay (`SkillBody`'s pattern). An unknown/ended id
    /// still gets a reply, with `text` saying so. Additive — older frontends
    /// warn-skip it.
    AgentActivity {
        id: String,
        name: String,
        text: String,
    },

    /// A `Command::AttachImage` (FR-10, Design §4.14) validated and encoded
    /// successfully; it now rides staged in `Engine::pending_attachments`
    /// until the next `Command::UserInput` drains it. The frontend uses this
    /// to show the attachment chip in the compose area before the message is
    /// sent. `pending_count` is the engine's own staged count *after* this
    /// one, so the frontend can enforce the `image.max_attachments` UI
    /// affordance (e.g. graying out further attach) without recomputing it.
    ImageAttached {
        path: String,
        name: String,
        media_type: String,
        width: usize,
        height: usize,
        format_label: String,
        pending_count: usize,
    },

    /// A `Command::AttachImage` failed validation (HC-6 — data, not a crash):
    /// oversize, an unrecognized format, unreadable path, or over
    /// `image.max_attachments`. Rendered as a plain input-time error (Design
    /// §4.14), never a silent drop.
    AttachFailed { path: String, reason: String },

    /// An in-session `/export` command finished writing a session export
    /// (FR-12, Tech Spec §8.6) to `path`. Not a transcript event — the export
    /// is an action taken on the session's own record, not part of what the
    /// session did (Tech Spec §3.1). A failure surfaces as a plain
    /// [`Notice`](UiEvent::Notice) instead (HC-3 — never a crash).
    SessionExported { path: String },
}
