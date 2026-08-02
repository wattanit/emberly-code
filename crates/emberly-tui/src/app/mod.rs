//! The rich TUI's local view-model (Tech Spec §9). The engine owns session
//! state; the frontend keeps a *projection* of it, updated by [`UiEvent`]s and
//! read by the renderer each frame. Keeping this a plain data structure with a
//! pure [`App::apply_event`] reducer is what lets the sidebar/status logic be
//! unit-tested without a terminal (§14).
//!
//! This module holds the state and the event/key plumbing. Rendering it is
//! [`crate::render`]'s job: layout, the markdown pass, diffs, the permission
//! prompt, and the palette all read this projection and never mutate it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use emberly_core::{
    resume, AskAnswer, AskId, CheckResult, Command, Effort, EntrySummary, GateResolution,
    LoopResolution, MemoryOp, MemoryScope, Mode, NewProviderProfile, PermissionDecision,
    PermissionId, PermissionRendering, ProviderProfileWriter, SandboxStatus, SessionId, SkillMeta,
    SkillOrigin, TaskItem, TokenUsage, ToolCallId, TranscriptEvent, TranscriptRecord, UiEvent,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::commands::{self, AppCommand, Slash};
use crate::editor::LineEditor;
use crate::hit::{ClickTarget, PermissionChoice};
use crate::theme::Theme;

/// A no-op [`ProviderProfileWriter`], for tests that construct an [`App`] but
/// never exercise the guided setup wizard's write path.
#[cfg(test)]
pub(crate) fn test_provider_writer() -> Arc<dyn ProviderProfileWriter> {
    struct NoopWriter;
    impl ProviderProfileWriter for NoopWriter {
        fn write_profile(&self, _profile: NewProviderProfile) -> Result<(), String> {
            Ok(())
        }
    }
    Arc::new(NoopWriter)
}

/// Rows the conversation scrolls per PageUp/PageDown.
const SCROLL_STEP: usize = 5;

/// Animation ticker rate (Design §6.4 — a handful of cells at ~12fps).
pub const ANIM_FPS: usize = 12;
/// The ember-pulse spinner: a single steady dot whose accent brightness
/// breathes in lockstep with the wordmark glow (Design §6.4) — one ember
/// pulse, not a separate glyph-cycling animation.
const SPINNER: [&str; 1] = ["●"];
/// Elapsed time appears only after this many seconds (Design §6.3).
const ELAPSED_AFTER_SECS: usize = 5;
/// Frames an overlay eases in over (one or two frames of expansion, not a slide
/// show — Design §6.4).
pub const EASE_FRAMES: u8 = 2;
/// Frames a freshly-landed modified-file entry stays highlighted as it settles.
pub const SETTLE_FRAMES: u8 = 5;

/// One rendered item in the conversation flow. Assistant text is enriched by the
/// markdown pass at render time; file changes carry a diff.
#[derive(Debug, Clone, PartialEq)]
pub enum ConvItem {
    /// A prompt the user submitted.
    User(String),
    /// Accumulated assistant text for one turn (deltas append to it).
    Assistant(String),
    /// The model's reasoning trail for one turn (P-10, Design §4.4), distinct
    /// from the answer. Collapsed to a dim one-line summary by default;
    /// `expanded` shows the full text. Deltas append while thinking.
    Reasoning { text: String, expanded: bool },
    /// A tool invocation and its outcome.
    Tool {
        call_id: ToolCallId,
        tool: String,
        /// What the call is doing (from `describe`) — e.g. `run: cargo test`.
        /// Set at start and kept; the result status is separate.
        summary: String,
        /// The model's caption for a non-obvious call (T-9, Design §4.5).
        /// `None` when the model gave none — rendered as nothing, no placeholder.
        explanation: Option<String>,
        /// `None` while running; `Some(ok)` once finished.
        done: Option<bool>,
        /// The finished one-line status (e.g. `exit 0`, `read foo.rs (12 lines)`).
        result: Option<String>,
        /// A short excerpt of the tool's output (Design §6.1).
        preview: Option<String>,
        /// Whether the result is untrusted web content (T-14, Design §4.10).
        /// When `true`, the render styles it as fetched web data with visible
        /// source URLs — never harness or assistant voice.
        untrusted: bool,
    },
    /// A harness-world line (error, retry) — rendered out-of-band from the
    /// conversation voice (Design §6.1).
    Notice(String),
    /// A unified diff shown inline when an edit executes (Design §4.2). Capped
    /// on render; the full diff is available in the overlay (Ctrl+O).
    Diff { unified: String },
    /// The model's task list (T-11, Design §4.7). Shown as an inline checklist
    /// block; a completed list settles to an all-done block rather than
    /// vanishing.
    TaskList { items: Vec<TaskItem> },
}

/// A dismissable, scrollable pane overlay (Design §4.2). Modal for navigation:
/// while an overlay is open, keys scroll or dismiss it. The permission prompt
/// and help build on the same mechanism.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub title: String,
    pub content: OverlayContent,
    /// Scroll offset in rows from the top.
    pub scroll: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayContent {
    /// A unified diff, rendered with diff colours.
    Diff(String),
    /// Plain text (help, untruncated output).
    Text(String),
    /// The interactive session picker (`/session`): a selectable list of saved
    /// sessions, newest first. Enter resumes the highlighted one.
    Sessions {
        rows: Vec<SessionRow>,
        selected: usize,
    },
    /// A generic single-choice picker (Design §3.1): the model/provider
    /// picker (C-6) and the reasoning-effort picker (P-9) both reuse it.
    /// Enter applies the highlighted choice; `kind` decides which command it
    /// becomes.
    Choices {
        kind: ChoiceKind,
        rows: Vec<ChoiceRow>,
        selected: usize,
    },
    /// The memory inspector (`/memory`, FR-6, Design §4.9): entries grouped by
    /// scope, each viewable/editable/deletable via the in-app edit path (§4.6).
    /// Unlike the pickers, Enter/`e`/`d` have distinct actions, so it is its own
    /// variant rather than a `Choices` reuse. `selected` indexes the flattened
    /// entry list (user entries, then project entries); `confirm_delete` gates
    /// the destructive delete behind an explicit y/N step (never a lone key).
    MemoryEntries {
        user: Vec<EntrySummary>,
        project: Vec<EntrySummary>,
        selected: usize,
        confirm_delete: bool,
    },
    /// The skills inspector (`/skills`, FR-7, Design §4.9): the available skills
    /// as a selectable list; Enter fetches the selected skill's instruction body
    /// to view **read-only**. There is no edit/delete — a skill is an
    /// externally-authored on-disk folder; the inspector shows what it could
    /// tell the model to do before it ever runs. The catalog is already cached
    /// in `App::skills`, so this needs no engine round-trip to open.
    SkillList {
        skills: Vec<SkillMeta>,
        selected: usize,
    },
}

/// Whether an inspector body-fetch is for read-only viewing or for editing
/// (FR-6, §4.6). Recorded when a [`Command::MemoryView`] is issued; consumed
/// when the [`UiEvent::MemoryBody`] reply arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryFetchIntent {
    View,
    Edit,
}

/// A memory edit staged for the frontend loop's `$EDITOR` handoff (FR-6, §4.6).
/// The loop opens `$EDITOR` on the body, then commits via
/// [`Command::MemoryMutate`] — the harness performs the write, never the TUI.
/// `description`/`type_` are carried through unchanged so a body edit never
/// silently erases the entry's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMemoryEdit {
    pub scope: MemoryScope,
    pub name: String,
    pub description: Option<String>,
    pub type_: Option<String>,
    pub body: String,
}

/// What a [`OverlayContent::Choices`] picker selects, so Enter knows which
/// command to issue. Shared by the model, effort, and mode pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceKind {
    /// Switch the active provider profile (C-6). The row label is the profile
    /// name; the current model is kept.
    Model,
    /// Set the reasoning-effort level (P-9). The row label is the level name.
    Effort,
    /// Set the permission mode (§6.4). The row label is the mode name; auto
    /// tiers are marked unavailable when OS confinement is not active.
    Mode,
}

/// How the reasoning trail is shown by default (Design §4.4). A view choice
/// only — the trace is always recorded to the transcript regardless (P-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasoningView {
    /// One dim summary line, expandable. The default.
    #[default]
    Collapsed,
    /// The full reasoning text, always shown.
    Expanded,
    /// Not shown in the view (still recorded to the transcript).
    Hidden,
}

impl ReasoningView {
    /// Parse the `reasoning` config key; unknown values fall back to the
    /// default (`collapsed`) so a typo is never fatal.
    #[must_use]
    pub fn parse(s: &str) -> ReasoningView {
        match s.trim().to_ascii_lowercase().as_str() {
            "expanded" => ReasoningView::Expanded,
            "hidden" => ReasoningView::Hidden,
            _ => ReasoningView::Collapsed,
        }
    }
}

/// One row in a [`OverlayContent::Choices`] picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRow {
    /// The value (e.g. a profile name) and the displayed label.
    pub label: String,
    /// True for the currently-active choice (marked, not re-applied).
    pub current: bool,
}

/// The command palette's state (Design §3.3): the fuzzy query and which match
/// is selected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteState {
    pub query: String,
    /// Index into the *filtered* match list.
    pub selected: usize,
}

/// A file the agent created or changed this session (sidebar list, Design §3.1).
#[derive(Debug, Clone, PartialEq)]
pub struct ModifiedFile {
    pub path: String,
    pub adds: u32,
    pub dels: u32,
}

/// Session identity for the sidebar/header (Design §3.1).
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    /// The current session id, so the picker can mark and skip it, and the
    /// header can name it. Updated in place on an in-session switch.
    pub session_id: SessionId,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub project_root: String,
}

/// One row in the session picker overlay (`/session`). A display projection of
/// [`emberly_core::resume::SessionSummary`] — cheap to clone and compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: SessionId,
    /// The session title, or a stand-in for an untitled one.
    pub title: String,
    /// One-line metadata (provider/model · age · events · interrupted).
    pub subtitle: String,
    /// True for the session the app is currently in (cannot resume itself).
    pub current: bool,
}

/// What a key press asked the frontend loop to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Nothing observable (e.g. a plain edit of the input buffer).
    None,
    /// Send this command to the engine.
    Command(Command),
    /// Begin a clean shutdown.
    Quit,
    /// Start a fresh session in place (`/new`). The frontend mints a new id,
    /// tells the engine, and resets the view.
    NewSession,
    /// Resume the saved session with this id (picker selection). The frontend
    /// reads its transcript, tells the engine, and reseeds the view.
    ResumeSession(SessionId),
    /// Open this file in `$EDITOR` (C-5). The frontend suspends the TUI, runs
    /// the editor, restores, and reports the outcome.
    EditFile(PathBuf),
}

/// A pending `ask_user` question and the state of the user's reply-in-progress
/// (T-8, Design §5.1). Options are selectable and a free-text answer is always
/// available; `selected` starts `None` so Enter never auto-answers.
pub struct AskPrompt {
    pub id: AskId,
    pub question: String,
    pub options: Vec<String>,
    /// The highlighted option, or `None` until the user moves to one — there is
    /// no default selection (Design §5.1: no unsafe default).
    pub selected: Option<usize>,
    /// The free-text answer buffer, always available alongside any options.
    pub editor: LineEditor,
}

impl AskPrompt {
    fn new(id: AskId, question: String, options: Vec<String>) -> Self {
        Self {
            id,
            question,
            options,
            selected: None,
            editor: LineEditor::new(),
        }
    }
}

/// A pending loop-halt decision (S-5, Design §8.5). The **harness** stepping in
/// when the model stopped progressing — rendered in the harness voice, distinct
/// from the question prompt. The user picks keep-going / stop / say-something;
/// choosing to steer opens the free-text field.
pub struct LoopHaltPrompt {
    pub reason: String,
    /// False = the three-choice menu; true = typing a steer message.
    pub steering: bool,
    pub editor: LineEditor,
}

impl LoopHaltPrompt {
    fn new(reason: String) -> Self {
        Self {
            reason,
            steering: false,
            editor: LineEditor::new(),
        }
    }
}

/// A pending completion-gate halt decision (S-6, Design §8.7). The harness
/// stepping in after a bounded number of failed completion attempts —
/// rendered in the harness voice, distinct from a failing check's agent-world
/// tool-result. The user picks keep-going / stop / say-something / **finish
/// anyway** (the explicit override); choosing to steer opens the free-text
/// field.
pub struct CompletionGatePrompt {
    pub failing: Vec<CheckResult>,
    pub attempts: usize,
    /// False = the four-choice menu; true = typing a steer message.
    pub steering: bool,
    pub editor: LineEditor,
}

impl CompletionGatePrompt {
    fn new(failing: Vec<CheckResult>, attempts: usize) -> Self {
        Self {
            failing,
            attempts,
            steering: false,
            editor: LineEditor::new(),
        }
    }
}

/// Adapters the guided setup wizard offers (Requirements C-7, Tech Spec §16
/// open item): the two wire formats `build_profile` already validates.
pub const WIZARD_ADAPTERS: [&str; 2] = ["anthropic", "openai"];

/// The model/provider picker's trailing entry point into the guided setup
/// wizard (Design §4.6). Compared by value in [`App::on_choice_picker_key`]
/// since `ChoiceRow` carries no variant tag.
const ADD_PROVIDER_ROW: &str = "+ add new provider…";

/// A step of the guided provider/model setup wizard (Requirements C-7, Design
/// §4.6). Name comes first — it's the identifier the user is actually
/// choosing (e.g. "deepseek"), distinct from the adapter (wire format) that
/// follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Name,
    Adapter,
    Endpoint,
    ModelId,
    ApiKey,
    Summary,
}

/// The guided setup wizard's in-progress state (Requirements C-7). Owns the
/// screen while set — harness voice, matching the other `pending_*` prompts.
/// One `editor` is reused across the free-text steps, reset to that step's
/// already-collected value on entry (so stepping back and forward doesn't
/// lose anything typed).
pub struct ProviderWizard {
    pub step: WizardStep,
    pub name: String,
    /// Index into [`WIZARD_ADAPTERS`].
    pub adapter_selected: usize,
    pub endpoint: String,
    pub model_id: String,
    pub api_key: String,
    pub editor: LineEditor,
    /// A name-collision or write failure, shown inline on the `Summary` step
    /// without losing any collected answer.
    pub error: Option<String>,
}

impl ProviderWizard {
    fn new() -> Self {
        Self {
            step: WizardStep::Name,
            name: String::new(),
            adapter_selected: 0,
            endpoint: String::new(),
            model_id: String::new(),
            api_key: String::new(),
            editor: LineEditor::new(),
            error: None,
        }
    }

    pub fn adapter(&self) -> &'static str {
        WIZARD_ADAPTERS[self.adapter_selected]
    }
}

/// The conversation timeline and how far up it the user has scrolled.
/// Grouped because `streaming` describes whether the last item is still
/// growing, and `scroll` is an offset into these items — reading one without
/// the others gives a stale picture of the pane.
pub(crate) struct Timeline {
    pub(crate) items: Vec<ConvItem>,
    /// True between the first `AssistantDelta` and `AssistantDone` of a turn.
    pub(crate) streaming: bool,
    /// Conversation scrollback offset in rows *from the bottom*: 0 follows the
    /// latest output; larger values scroll up into history. Clamped to content
    /// at render time (Design §3.1 — the main pane owns scrollback).
    pub(crate) scroll: usize,
    /// How the reasoning trail is displayed (Design §4.4); from the `reasoning`
    /// config key. `Hidden` suppresses the trail in the view only.
    pub(crate) reasoning: ReasoningView,
}

/// What this session has consumed. All five move together on a provider
/// `Usage`, and all five reset together on a session switch.
pub(crate) struct UsageState {
    /// Cumulative billed tokens this session (input + output), for the sidebar
    /// total. Available regardless of pricing.
    pub(crate) tokens: TokenUsage,
    pub(crate) cost_usd: f64,
    pub(crate) cost_known: bool,
    pub(crate) context_pct: u8,
    pub(crate) context_tokens: u64,
}

/// The prompts that take over the screen while awaiting an answer. At most one
/// is ever set; keeping them together is what makes that invariant legible
/// (Design §5, §5.1, §8.5, §8.7).
pub(crate) struct Prompts {
    /// The permission prompt currently awaiting an answer, if any. While set,
    /// the prompt owns the screen and normal input is suspended (Design §5).
    pub(crate) permission: Option<(PermissionId, PermissionRendering)>,
    /// Scroll offset (rows from top) into the current permission prompt's
    /// content, so long commands/diffs can be reviewed in full (Design §5).
    pub(crate) permission_scroll: usize,
    /// The `ask_user` question currently awaiting an answer, if any (T-8). While
    /// set, the question prompt owns the screen and normal input is suspended
    /// (Design §5.1). Never the permission prompt's safety styling.
    pub(crate) ask: Option<AskPrompt>,
    /// A pending loop-halt decision (S-5, Design §8.5) — the harness stepping in.
    /// While set it owns the screen; harness voice, not the question prompt.
    pub(crate) loop_halt: Option<LoopHaltPrompt>,
    /// A pending completion-gate halt decision (S-6, Design §8.7) — the
    /// harness stepping in after a bounded number of failed completion
    /// attempts. While set it owns the screen; harness voice.
    pub(crate) completion_gate: Option<CompletionGatePrompt>,
}

/// The guided provider/model setup wizard (C-7) and the two things that
/// outlive one run of it.
pub(crate) struct WizardState {
    /// The wizard in progress (Design §4.6). While set it owns the screen.
    pub(crate) pending: Option<ProviderWizard>,
    /// Writes a wizard-completed profile to disk, injected by the binary
    /// composition root so this crate never owns `config.toml`/`keys.toml`
    /// schema knowledge (A-1).
    pub(crate) writer: Arc<dyn ProviderProfileWriter>,
    /// Model ids the wizard registered this session, keyed by profile name: the
    /// picker has no other per-profile default model to fall back to, so a
    /// freshly-added profile would otherwise be tried with whatever model was
    /// previously active.
    pub(crate) created_models: HashMap<String, String>,
}

/// The memory panel (FR-6, T-13, Design §4.9): sidebar counts plus the
/// inspector's in-flight fetch and pending edit.
pub(crate) struct MemoryPanel {
    /// Entry counts for the sidebar. Updated from `UiEvent::MemoryStatus`;
    /// cleared on a new session.
    pub(crate) user: usize,
    pub(crate) project: usize,
    /// The inspector's in-flight body fetch (§4.6): the selected entry and
    /// whether the user wants to view or edit it. Set when a `MemoryView` is
    /// issued; consumed when the `MemoryBody` reply arrives.
    pub(crate) fetch: Option<(MemoryFetchIntent, EntrySummary)>,
    /// A memory edit whose body has arrived and is ready for the `$EDITOR`
    /// handoff (§4.6). The frontend loop drains this, runs the editor, and
    /// commits via `MemoryMutate` (the harness writes, not the TUI).
    pub(crate) pending_edit: Option<PendingMemoryEdit>,
}

/// Modified files and their diffs (Design §4.2).
pub(crate) struct FilesState {
    pub(crate) modified: Vec<ModifiedFile>,
    /// Latest unified diff per modified file, for the diff overlay. Keyed by path.
    pub(crate) diffs: HashMap<String, String>,
    /// The most recently modified file — the target of the Ctrl+O diff overlay,
    /// since the sidebar has no per-file selection.
    pub(crate) last: Option<String>,
}

/// Animation and busy state (Design §6.3, §6.4). `tick` advances every counter
/// here and `is_animating` reads every one of them, so they are one unit.
pub(crate) struct MotionState {
    /// True while a turn is in flight (submit → `TurnEnded`): drives the
    /// "working" spinner (Design §6.3).
    pub(crate) busy: bool,
    /// Whether motion is enabled (Design §6.4 off-switch). Off in degraded mode
    /// (line frontend has no ticker) and via config/env.
    pub(crate) active: bool,
    /// Animation frame counter, advanced by the ticker only while animating.
    /// Time-source-free: elapsed ≈ `frame / ANIM_FPS`.
    pub(crate) frame: usize,
    /// Frames remaining in an overlay's ease-in expansion (Design §6.4).
    pub(crate) overlay_ease: u8,
    /// Frames remaining in the newest modified-file's settle highlight.
    pub(crate) sidebar_settle: u8,
}

/// The complete view-model the renderer reads.
///
/// State is grouped into sub-structs by surface rather than held flat. 29 of
/// the fields belonged to seven clusters whose members are always read and
/// written together — one tick, one session switch, one provider `Usage` — and
/// flat, it was possible to update one and forget its siblings.
pub struct App {
    pub(crate) timeline: Timeline,
    pub(crate) usage: UsageState,
    pub(crate) prompts: Prompts,
    pub(crate) wizard: WizardState,
    pub(crate) memory: MemoryPanel,
    pub(crate) files: FilesState,
    pub(crate) anim: MotionState,
    pub(crate) session: SessionInfo,
    /// Where session transcripts live, so the picker can list them and a
    /// resume can read one (`/session`, `/resume`).
    pub(crate) sessions_dir: PathBuf,
    /// Configured provider-profile names, for the model picker (`/model`, C-6).
    /// Sorted; empty when no factory/profiles are available.
    profiles: Vec<String>,
    /// The `.agents/config.toml` starter written by `/config` when the project
    /// has none yet — same content `emberly init` materializes (C-5, single
    /// source in the binary).
    config_template: String,
    /// The active reasoning-effort level, for the sidebar (P-9). `None` when the
    /// model has no effort control (the line is hidden). Set by `EffortChanged`.
    pub(crate) effort: Option<Effort>,
    /// The levels the active model offers, for the `/effort` picker. Empty ⇒ no
    /// control. Set by `EffortChanged`.
    effort_levels: Vec<Effort>,
    /// The grapheme-aware input editor (multi-line, history, Thai-correct
    /// cursor motion). See [`crate::editor`].
    pub(crate) editor: LineEditor,
    /// `None` until the engine reports confinement status (Phase 2).
    pub(crate) sandbox: Option<SandboxStatus>,
    pub(crate) mode: emberly_core::Mode,
    /// The model-maintained task list (T-11, Design §4.7). Updated from
    /// `UiEvent::TaskListUpdated`; cleared on a new session.
    pub(crate) tasks: Vec<TaskItem>,
    /// The skill catalog for the sidebar (T-15, FR-7, Design §4.9). Updated
    /// from `UiEvent::SkillsAvailable`; cleared on a new session.
    pub(crate) skills: Vec<SkillMeta>,
    /// The registered completion checks' most recent results, for the
    /// sidebar's gate-status line (S-6, Design §8.7, §3.1). Updated from
    /// `UiEvent::CompletionStatus`; empty (and so hidden — never a "None"
    /// stub) until the gate has run at least once, and cleared on a new
    /// session.
    pub(crate) completion_status: Vec<CheckResult>,
    pub(crate) sidebar_visible: bool,
    /// The overlay stack; the last entry is on top and receives input.
    pub(crate) overlays: Vec<Overlay>,
    /// The command palette, when open (Ctrl+P). Modal while present.
    pub(crate) palette: Option<PaletteState>,
    /// The click hit-map from the last rendered frame (Design §3.4). Rebuilt by
    /// `render::frame` and stored here by the `tui` loop after each draw, so a
    /// click resolves against the geometry actually on screen.
    pub(crate) hit_map: crate::hit::HitMap,
    /// The active theme (Design §2). One source the renderer reads; swapping it
    /// (mode/light-fallback later) is a value change, not a refactor.
    pub(crate) theme: Theme,
}

// The `App` impl is spread across these modules by surface, each holding its
// own `impl App` block. Each module sees this one's private items, but not a
// sibling's, so a method called from another of these modules is marked
// `pub(super)`.
mod actions;
mod anim;
mod events;
mod keys;
mod memory;
mod overlays;
mod pickers;
mod prompts;
mod session;
mod skills;
mod wizard;

#[cfg(test)]
mod tests;

impl App {
    #[must_use]
    pub fn new(
        session: SessionInfo,
        sessions_dir: PathBuf,
        profiles: Vec<String>,
        config_template: String,
        provider_writer: Arc<dyn ProviderProfileWriter>,
    ) -> Self {
        Self {
            timeline: Timeline {
                items: Vec::new(),
                streaming: false,
                scroll: 0,
                reasoning: ReasoningView::default(),
            },
            usage: UsageState {
                tokens: TokenUsage::default(),
                cost_usd: 0.0,
                cost_known: false,
                context_pct: 0,
                context_tokens: 0,
            },
            prompts: Prompts {
                permission: None,
                permission_scroll: 0,
                ask: None,
                loop_halt: None,
                completion_gate: None,
            },
            wizard: WizardState {
                pending: None,
                writer: provider_writer,
                created_models: HashMap::new(),
            },
            memory: MemoryPanel {
                user: 0,
                project: 0,
                fetch: None,
                pending_edit: None,
            },
            files: FilesState {
                modified: Vec::new(),
                diffs: HashMap::new(),
                last: None,
            },
            anim: MotionState {
                busy: false,
                active: true,
                frame: 0,
                overlay_ease: 0,
                sidebar_settle: 0,
            },
            session,
            sessions_dir,
            profiles,
            config_template,
            effort: None,
            effort_levels: Vec::new(),
            editor: LineEditor::new(),
            sandbox: None,
            mode: emberly_core::Mode::default(),
            tasks: Vec::new(),
            skills: Vec::new(),
            completion_status: Vec::new(),
            sidebar_visible: true,
            overlays: Vec::new(),
            palette: None,
            hit_map: crate::hit::HitMap::new(),
            theme: Theme::rich(),
        }
    }
}

/// The display label for a mode row in the picker. Auto tiers are annotated
/// `(needs OS confinement)` when the sandbox is not active, so the picker is
/// honest about why they can't be chosen (Requirements §6.4, §6.7).
fn mode_label(mode: Mode, auto_ok: bool) -> String {
    let name = match mode {
        Mode::Normal => crate::strings::mode::NORMAL,
        Mode::AutoAcceptEdits => crate::strings::mode::AUTO_ACCEPT_EDITS,
        Mode::Auto => crate::strings::mode::AUTO,
    };
    if mode.is_auto() && !auto_ok {
        format!("{name}  (needs OS confinement)")
    } else {
        name.to_string()
    }
}

/// Parse a `/mode <name>` argument; unknown values return `None`. Accepts the
/// `strings::mode` names and a couple of common shorthands.
fn parse_mode(s: &str) -> Option<Mode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(Mode::Normal),
        "auto-accept-edits" | "auto-accept" | "edits" => Some(Mode::AutoAcceptEdits),
        "auto" => Some(Mode::Auto),
        _ => None,
    }
}

/// The user-facing label for a memory scope (origin is how the user reads
/// trust, Design §4.9). Centralized so the inspector and any future surface
/// agree.
fn memory_scope_label(scope: MemoryScope) -> &'static str {
    match scope {
        MemoryScope::User => crate::strings::memory::SCOPE_USER,
        MemoryScope::Project => crate::strings::memory::SCOPE_PROJECT,
    }
}

/// The user-facing label for a skill's origin (user vs project — origin is how
/// the user reads trust, FR-7/§4.9).
fn skill_origin_label(origin: SkillOrigin) -> &'static str {
    match origin {
        SkillOrigin::User => crate::strings::skills::ORIGIN_USER,
        SkillOrigin::Project => crate::strings::skills::ORIGIN_PROJECT,
    }
}

/// The `/help` body: every command with its keybinding and description, from
/// the single registry (Design §3.3).
fn help_text() -> String {
    let mut out = String::from("Commands — run via Ctrl-P, /name, or a keybinding.\n\n");
    for spec in commands::COMMANDS {
        let key = spec.key.map(|k| format!("  [{k}]")).unwrap_or_default();
        out.push_str(&format!("/{:<9}{}\n    {}\n", spec.name, key, spec.desc));
    }
    // `/model` also accepts arguments for a direct switch (C-6).
    out.push_str("  (also: /model <profile> [model] to switch directly)\n");
    // `/mode` and `/effort` also take a direct argument.
    out.push_str("  (also: /mode <normal|auto-accept-edits|auto>, /effort <level>)\n");
    // Mouse (Design §3.4): additive to the keyboard — everything here the
    // keyboard already does. Documents the Shift-passthrough and the off switch.
    out.push_str("\nMouse (on by default; set [ui] mouse = false to turn off):\n");
    out.push_str("    wheel / trackpad scrolls the focused pane or open overlay\n");
    out.push_str("    click selects a row (palette, picker, sidebar entry, reasoning trail) —\n");
    out.push_str("      the same as focusing it and pressing Enter; it never approves a prompt\n");
    out.push_str("    hold Shift (in most terminals) to drag-select and copy text as usual;\n");
    out.push_str("      or set mouse = false to let the terminal own the pointer entirely\n");
    out
}
