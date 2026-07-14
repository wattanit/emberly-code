//! The rich TUI's local view-model (Tech Spec §9). The engine owns session
//! state; the frontend keeps a *projection* of it, updated by [`UiEvent`]s and
//! read by the renderer each frame. Keeping this a plain data structure with a
//! pure [`App::apply_event`] reducer is what lets the sidebar/status logic be
//! unit-tested without a terminal (§14; group 11).
//!
//! Group 1 establishes the state and the event/key plumbing with a minimal
//! render; the real layout (group 4), markdown (group 5), diffs (group 6),
//! permission prompt (group 7), and palette (group 8) fill it in.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use emberly_core::{
    resume, AskAnswer, AskId, Command, Effort, EntrySummary, LoopResolution, MemoryOp, MemoryScope,
    Mode, PermissionDecision, PermissionId, PermissionRendering, SandboxStatus, SessionId,
    SkillMeta, SkillOrigin, TaskItem, TokenUsage, ToolCallId, TranscriptEvent, TranscriptRecord,
    UiEvent,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::commands::{self, AppCommand};
use crate::editor::LineEditor;
use crate::hit::ClickTarget;
use crate::theme::Theme;

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

/// One rendered item in the conversation flow. Group 5 enriches assistant text
/// with the markdown pass; group 6 adds diffs.
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
/// (group 7) and help (group 8) build on the same mechanism.
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
    /// A generic single-choice picker (Design §3.1): the model/provider picker
    /// now (C-6), the reasoning-effort picker in Phase 3. Enter applies the
    /// highlighted choice; `kind` decides which command it becomes.
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
enum MemoryFetchIntent {
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
/// command to issue. Reused by the effort picker in Phase 3 and the mode
/// picker.
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

/// The complete view-model the renderer reads.
pub struct App {
    pub session: SessionInfo,
    /// Where session transcripts live, so the picker can list them and a
    /// resume can read one (`/session`, `/resume`).
    pub sessions_dir: PathBuf,
    /// Configured provider-profile names, for the model picker (`/model`, C-6).
    /// Sorted; empty when no factory/profiles are available.
    pub profiles: Vec<String>,
    /// The `.agents/config.toml` starter written by `/config` when the project
    /// has none yet — same content `emberly init` materializes (C-5, single
    /// source in the binary).
    pub config_template: String,
    pub conversation: Vec<ConvItem>,
    /// True between the first `AssistantDelta` and `AssistantDone` of a turn.
    pub streaming: bool,
    /// How the reasoning trail is displayed (Design §4.4); from the `reasoning`
    /// config key. `Hidden` suppresses the trail in the view only.
    pub reasoning_view: ReasoningView,
    /// The active reasoning-effort level, for the sidebar (P-9). `None` when the
    /// model has no effort control (the line is hidden). Set by `EffortChanged`.
    pub effort: Option<Effort>,
    /// The levels the active model offers, for the `/effort` picker. Empty ⇒ no
    /// control. Set by `EffortChanged`.
    pub effort_levels: Vec<Effort>,
    /// The grapheme-aware input editor (multi-line, history, Thai-correct
    /// cursor motion). See [`crate::editor`].
    pub editor: LineEditor,
    pub context_pct: u8,
    pub context_tokens: u64,
    /// Cumulative billed tokens this session (input + output), for the sidebar
    /// total. Available regardless of pricing.
    pub session_usage: TokenUsage,
    pub cost_usd: f64,
    pub cost_known: bool,
    /// `None` until the engine reports confinement status (Phase 2).
    pub sandbox: Option<SandboxStatus>,
    pub mode: emberly_core::Mode,
    pub modified_files: Vec<ModifiedFile>,
    /// The model-maintained task list (T-11, Design §4.7). Updated from
    /// `UiEvent::TaskListUpdated`; cleared on a new session.
    pub tasks: Vec<TaskItem>,
    /// Memory entry counts for the sidebar (T-13, FR-6, Design §4.9). Updated
    /// from `UiEvent::MemoryStatus`; cleared on a new session.
    pub memory_user: usize,
    pub memory_project: usize,
    /// The skill catalog for the sidebar (T-15, FR-7, Design §4.9). Updated
    /// from `UiEvent::SkillsAvailable`; cleared on a new session.
    pub skills: Vec<SkillMeta>,
    /// The inspector's in-flight body fetch (FR-6, §4.6): the selected entry
    /// and whether the user wants to view or edit it. Set when a `MemoryView`
    /// is issued; consumed when the `MemoryBody` reply arrives.
    memory_fetch: Option<(MemoryFetchIntent, EntrySummary)>,
    /// A memory edit whose body has arrived and is ready for the `$EDITOR`
    /// handoff (FR-6, §4.6). The frontend loop drains this, runs the editor, and
    /// commits via `MemoryMutate` (the harness writes, not the TUI).
    pending_memory_edit: Option<PendingMemoryEdit>,
    /// The permission prompt currently awaiting an answer, if any. While set,
    /// the prompt owns the screen and normal input is suspended (Design §5).
    pub pending_permission: Option<(PermissionId, PermissionRendering)>,
    /// Scroll offset (rows from top) into the current permission prompt's
    /// content, so long commands/diffs can be reviewed in full (Design §5).
    pub permission_scroll: usize,
    /// The `ask_user` question currently awaiting an answer, if any (T-8). While
    /// set, the question prompt owns the screen and normal input is suspended
    /// (Design §5.1). Never the permission prompt's safety styling.
    pub pending_ask: Option<AskPrompt>,
    /// A pending loop-halt decision (S-5, Design §8.5) — the harness stepping in.
    /// While set it owns the screen; harness voice, not the question prompt.
    pub pending_loop_halt: Option<LoopHaltPrompt>,
    pub sidebar_visible: bool,
    /// Conversation scrollback offset in rows *from the bottom*: 0 follows the
    /// latest output; larger values scroll up into history. Clamped to content
    /// at render time (Design §3.1 — the main pane owns scrollback).
    pub scroll: usize,
    /// Latest unified diff per modified file, for the diff overlay (Design
    /// §4.2). Keyed by path.
    pub latest_diffs: HashMap<String, String>,
    /// The most recently modified file (target of the Ctrl+O diff overlay until
    /// sidebar selection lands in group 8).
    pub last_modified: Option<String>,
    /// The overlay stack; the last entry is on top and receives input.
    pub overlays: Vec<Overlay>,
    /// The command palette, when open (Ctrl+P). Modal while present.
    pub palette: Option<PaletteState>,
    /// The click hit-map from the last rendered frame (Design §3.4). Rebuilt by
    /// `render::frame` and stored here by the `tui` loop after each draw, so a
    /// click resolves against the geometry actually on screen.
    pub hit_map: crate::hit::HitMap,
    /// True while a turn is in flight (submit → `TurnEnded`): drives the
    /// "working" spinner (Design §6.3).
    pub busy: bool,
    /// Whether motion is enabled (Design §6.4 off-switch). Off in degraded mode
    /// (line frontend has no ticker) and via config/env.
    pub motion: bool,
    /// Animation frame counter, advanced by the ticker only while animating.
    /// Time-source-free: elapsed ≈ `anim_frame / ANIM_FPS`.
    anim_frame: usize,
    /// Frames remaining in an overlay's ease-in expansion (Design §6.4).
    overlay_ease: u8,
    /// Frames remaining in the newest modified-file's settle highlight.
    sidebar_settle: u8,
    /// The active theme (Design §2). One source the renderer reads; swapping it
    /// (mode/light-fallback later) is a value change, not a refactor.
    pub theme: Theme,
}

impl App {
    #[must_use]
    pub fn new(
        session: SessionInfo,
        sessions_dir: PathBuf,
        profiles: Vec<String>,
        config_template: String,
    ) -> Self {
        Self {
            session,
            sessions_dir,
            profiles,
            config_template,
            conversation: Vec::new(),
            streaming: false,
            reasoning_view: ReasoningView::default(),
            effort: None,
            effort_levels: Vec::new(),
            editor: LineEditor::new(),
            context_pct: 0,
            context_tokens: 0,
            session_usage: TokenUsage::default(),
            cost_usd: 0.0,
            cost_known: false,
            sandbox: None,
            mode: emberly_core::Mode::default(),
            modified_files: Vec::new(),
            tasks: Vec::new(),
            memory_user: 0,
            memory_project: 0,
            skills: Vec::new(),
            memory_fetch: None,
            pending_memory_edit: None,
            pending_permission: None,
            pending_ask: None,
            pending_loop_halt: None,
            permission_scroll: 0,
            sidebar_visible: true,
            scroll: 0,
            latest_diffs: HashMap::new(),
            last_modified: None,
            overlays: Vec::new(),
            palette: None,
            hit_map: crate::hit::HitMap::new(),
            busy: false,
            motion: true,
            anim_frame: 0,
            overlay_ease: 0,
            sidebar_settle: 0,
            theme: Theme::rich(),
        }
    }

    /// Seed the conversation timeline from a resumed transcript (Tech Spec
    /// §3.3), so the restored session shows its history rather than a blank
    /// pane. Maps durable records to display items; non-conversation records
    /// (session_start/title/permission/end) are skipped.
    pub fn seed_history(&mut self, records: &[TranscriptRecord]) {
        for record in records {
            match &record.event {
                TranscriptEvent::UserMessage { text, .. } => {
                    self.conversation.push(ConvItem::User(text.clone()));
                }
                TranscriptEvent::AssistantMessage { text, reasoning } => {
                    // Replay a recorded reasoning trail (collapsed) unless the
                    // view hides it (P-10, Design §4.4).
                    if let Some(reasoning) = reasoning {
                        if self.reasoning_view != ReasoningView::Hidden {
                            self.conversation.push(ConvItem::Reasoning {
                                text: reasoning.clone(),
                                expanded: self.reasoning_view == ReasoningView::Expanded,
                            });
                        }
                    }
                    if !text.is_empty() {
                        self.conversation.push(ConvItem::Assistant(text.clone()));
                    }
                }
                TranscriptEvent::ToolCall {
                    call_id,
                    tool,
                    args,
                } => {
                    // Reconstruct a readable label from the recorded args.
                    let summary = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|c| format!("run: {c}"))
                        .or_else(|| {
                            args.get("path")
                                .and_then(|v| v.as_str())
                                .map(|p| format!("{tool} {p}"))
                        })
                        .unwrap_or_else(|| tool.clone());
                    // The explanation (T-9) rides in the recorded args, so a
                    // replayed session shows the same caption a live one did.
                    let explanation = args
                        .get("explanation")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    self.conversation.push(ConvItem::Tool {
                        call_id: call_id.clone(),
                        tool: tool.clone(),
                        summary,
                        explanation,
                        done: None,
                        result: None,
                        preview: None,
                        untrusted: false,
                    });
                }
                TranscriptEvent::ToolResult {
                    call_id,
                    ok,
                    output,
                    ..
                } => {
                    if let Some(ConvItem::Tool { done, preview, .. }) = self.find_tool_mut(call_id)
                    {
                        *done = Some(*ok);
                        *preview = Some(output.clone());
                    }
                }
                TranscriptEvent::Compaction { summary, .. } => {
                    self.conversation
                        .push(ConvItem::Notice(format!("compacted — {summary}")));
                }
                _ => {}
            }
        }
        // Resumed content scrolls off the top; start pinned to the latest.
        self.scroll = 0;
    }

    /// Fold one engine event into the view-model. Pure over `self` — no I/O — so
    /// a canned event sequence can be asserted against the resulting state.
    pub fn apply_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::AssistantDelta { text } => {
                if self.streaming {
                    if let Some(ConvItem::Assistant(buf)) = self.conversation.last_mut() {
                        buf.push_str(&text);
                        return;
                    }
                }
                // The answer is starting: settle the just-streamed reasoning
                // trail to its collapsed line unless the view pins it open
                // (Design §4.4).
                if self.reasoning_view == ReasoningView::Collapsed {
                    if let Some(ConvItem::Reasoning { expanded, .. }) = self.conversation.last_mut()
                    {
                        *expanded = false;
                    }
                }
                self.streaming = true;
                self.conversation.push(ConvItem::Assistant(text));
            }
            UiEvent::ReasoningDelta { text } => {
                // `hidden` is a view choice: skip the trail but the engine still
                // records the trace to the transcript (P-10, Design §4.4).
                if self.reasoning_view == ReasoningView::Hidden {
                    return;
                }
                if let Some(ConvItem::Reasoning { text: buf, .. }) = self.conversation.last_mut() {
                    buf.push_str(&text);
                } else {
                    // Stream in place while thinking; expanded until the answer
                    // begins (then settled), or always when the view pins it.
                    self.conversation.push(ConvItem::Reasoning {
                        text,
                        expanded: true,
                    });
                }
            }
            UiEvent::AssistantDone => self.streaming = false,
            UiEvent::TurnEnded => {
                self.busy = false;
                self.streaming = false;
            }
            UiEvent::ToolStarted {
                call_id,
                tool,
                summary,
                explanation,
            } => {
                self.streaming = false;
                self.conversation.push(ConvItem::Tool {
                    call_id,
                    tool,
                    summary,
                    explanation,
                    done: None,
                    result: None,
                    preview: None,
                    untrusted: false,
                });
            }
            UiEvent::ToolFinished {
                call_id,
                ok,
                summary,
                preview,
                untrusted,
            } => {
                if let Some(ConvItem::Tool {
                    done,
                    result,
                    preview: p,
                    untrusted: u,
                    ..
                }) = self.find_tool_mut(&call_id)
                {
                    *done = Some(ok);
                    // Keep the descriptive label; record the result status and
                    // a preview of the output separately.
                    *result = (!summary.is_empty()).then_some(summary);
                    *p = (!preview.is_empty()).then_some(preview);
                    *u = untrusted;
                }
            }
            UiEvent::PermissionRequest { id, rendering } => {
                self.pending_permission = Some((id, rendering));
                self.permission_scroll = 0; // start every prompt at the top
            }
            UiEvent::AskUserRequest {
                id,
                question,
                options,
            } => {
                self.streaming = false;
                self.pending_ask = Some(AskPrompt::new(id, question, options));
            }
            UiEvent::LoopHalted { reason } => {
                self.streaming = false;
                self.pending_loop_halt = Some(LoopHaltPrompt::new(reason));
            }
            UiEvent::ContextUsage { pct, tokens } => {
                self.context_pct = pct;
                self.context_tokens = tokens;
            }
            UiEvent::CostEstimate { usd, .. } => {
                self.cost_usd = usd;
                self.cost_known = true;
            }
            UiEvent::SessionUsage { usage } => self.session_usage = usage,
            UiEvent::SandboxStatus { status } => self.sandbox = Some(status),
            UiEvent::ModeChanged { mode } => self.mode = mode,
            UiEvent::ModelChanged { provider, model } => {
                // Sidebar reflects the new provider/model; the engine also emits
                // a Notice, so the switch is never silent (Design §3.1).
                self.session.provider = provider;
                self.session.model = model;
            }
            UiEvent::EffortChanged { effort, available } => {
                self.effort = effort;
                self.effort_levels = available;
            }
            UiEvent::ProfilesChanged { profiles } => {
                // A `/config` reload changed the provider set; refresh the
                // picker's list (C-5).
                self.profiles = profiles;
            }
            UiEvent::Notice { message } => self.conversation.push(ConvItem::Notice(message)),
            UiEvent::HarnessError { what, why, next } => {
                self.conversation
                    .push(ConvItem::Notice(format!("error: {what} — {why}. {next}")));
            }
            UiEvent::Retrying {
                attempt,
                max_attempts,
                delay_ms,
                reason,
            } => {
                self.conversation.push(ConvItem::Notice(format!(
                    "retrying ({attempt}/{max_attempts}) in {delay_ms}ms — {reason}"
                )));
            }
            UiEvent::SessionMeta {
                session_id,
                title,
                provider,
                model,
                project_root,
            } => {
                self.session = SessionInfo {
                    session_id,
                    title,
                    provider,
                    model,
                    project_root,
                };
            }
            UiEvent::FileModified { path, adds, dels } => {
                self.last_modified = Some(path.clone());
                let is_new = self.upsert_modified(path, adds, dels);
                // A newly-landed entry gets a brief settle highlight (Design §6.4).
                if is_new && self.motion {
                    self.sidebar_settle = SETTLE_FRAMES;
                }
            }
            UiEvent::FileDiff { path, unified } => {
                // Show it inline when the edit executes (Design §4.2) …
                self.conversation.push(ConvItem::Diff {
                    unified: unified.clone(),
                });
                // … and keep the latest per file for the on-demand overlay.
                self.latest_diffs.insert(path, unified);
            }
            UiEvent::CompactionStatus { message } => {
                self.conversation.push(ConvItem::Notice(message));
            }
            UiEvent::TaskListUpdated { items } => {
                self.tasks = items.clone();
                self.conversation.push(ConvItem::TaskList { items });
            }
            UiEvent::MemoryStatus { user, project } => {
                self.memory_user = user;
                self.memory_project = project;
            }
            UiEvent::SkillsAvailable { skills } => {
                self.skills = skills;
            }
            UiEvent::MemoryEntries { user, project } => {
                self.apply_memory_entries(user, project);
            }
            UiEvent::MemoryBody { scope, name, body } => {
                self.apply_memory_body(scope, &name, body);
            }
            UiEvent::SkillBody {
                name,
                origin,
                body,
                resources,
            } => {
                self.apply_skill_body(&name, origin, body, &resources);
            }
            // `#[non_exhaustive]`: unknown future events are ignored, not fatal.
            _ => {}
        }
    }

    fn find_tool_mut(&mut self, call_id: &ToolCallId) -> Option<&mut ConvItem> {
        self.conversation
            .iter_mut()
            .rev()
            .find(|item| matches!(item, ConvItem::Tool { call_id: c, .. } if c == call_id))
    }

    /// Insert or update a modified-file entry; returns `true` if it was new.
    fn upsert_modified(&mut self, path: String, adds: u32, dels: u32) -> bool {
        if let Some(existing) = self.modified_files.iter_mut().find(|f| f.path == path) {
            existing.adds = adds;
            existing.dels = dels;
            false
        } else {
            self.modified_files.push(ModifiedFile { path, adds, dels });
            true
        }
    }

    /// Handle a key press, returning the action for the loop to carry out.
    ///
    /// While a permission prompt is open it owns the keyboard: only the
    /// deliberate allow keys approve, and everything else (including Enter and
    /// Esc) denies — deny is the safe default (Design §5). The full prompt
    /// screen and scrolling arrive in group 7; the guarantees hold from now.
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        // The command palette is modal while open (Design §3.3).
        if self.palette.is_some() {
            return self.on_palette_key(key);
        }
        // An open overlay is modal for navigation: scroll or dismiss (Design
        // §4.2). It sits above the permission check so a diff can be reviewed,
        // but note we never open an overlay while a permission prompt is up.
        if !self.overlays.is_empty() {
            return self.on_overlay_key(key);
        }
        if let Some(id) = self.pending_permission.as_ref().map(|(i, _)| *i) {
            return self.on_permission_key(id, key);
        }
        // The question prompt also owns the keyboard while open (Design §5.1),
        // but with opposite semantics: no unsafe default, Esc declines.
        if self.pending_ask.is_some() {
            return self.on_ask_key(key);
        }
        // The loop-halt surface owns the keyboard too (Design §8.5) — the
        // harness stepping in; the user always decides what happens next.
        if self.pending_loop_halt.is_some() {
            return self.on_loop_halt_key(key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // Newline (multi-line input) via Ctrl+J — a plain LF that every
            // terminal delivers, so multi-line entry works even where Shift+
            // Enter is indistinguishable from Enter.
            KeyCode::Char('j') if ctrl => self.edit(|e| e.newline()),
            // Ctrl-D on an empty line quits; on a non-empty line it deletes
            // forward (readline convention).
            KeyCode::Char('d') if ctrl => {
                if self.editor.is_empty() {
                    return Action::Quit;
                }
                self.editor.delete();
                Action::None
            }
            KeyCode::Char('c') if ctrl => {
                if self.editor.is_empty() {
                    Action::Quit
                } else {
                    self.editor.clear();
                    Action::None
                }
            }
            KeyCode::Char('b') if ctrl => {
                self.sidebar_visible = !self.sidebar_visible;
                Action::None
            }
            // Shift+Tab cycles the auto-accept mode (crossterm delivers it as
            // BackTab). The engine gates the auto tiers on confinement.
            KeyCode::BackTab => self.run_command(AppCommand::CycleMode),
            // Open the command palette (Design §3.3).
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(PaletteState::default());
                Action::None
            }
            // Open the most-recently-modified file's diff in an overlay.
            KeyCode::Char('o') if ctrl => {
                self.open_last_diff();
                Action::None
            }
            // Toggle the most recent reasoning trail open/closed (Design §4.4).
            KeyCode::Char('r') if ctrl => {
                self.toggle_reasoning();
                Action::None
            }
            // Emacs-style line editing.
            KeyCode::Char('a') if ctrl => self.edit(|e| e.home()),
            KeyCode::Char('e') if ctrl => self.edit(|e| e.end()),
            KeyCode::Char('k') if ctrl => self.edit(|e| e.kill_to_end()),
            KeyCode::Char('w') if ctrl => self.edit(|e| e.delete_word_back()),
            // Shift+Enter and Alt+Enter insert a newline where the terminal
            // reports the modifier; plain Enter submits.
            KeyCode::Enter if shift || alt => self.edit(|e| e.newline()),
            KeyCode::Enter => match self.editor.submit() {
                Some(text) => {
                    self.scroll = 0; // jump back to the latest output
                                     // A leading '/' is a slash command, not a message.
                    if let Some(name) = text.strip_prefix('/') {
                        self.run_slash(name)
                    } else {
                        // Echo the prompt into the timeline so the main pane is
                        // a single top-to-bottom transcript of both sides.
                        self.conversation.push(ConvItem::User(text.clone()));
                        // Enter the "working" state (Design §6.3); the spinner
                        // runs from frame 0 until TurnEnded.
                        self.busy = true;
                        self.anim_frame = 0;
                        Action::Command(Command::UserInput { text })
                    }
                }
                None => Action::None,
            },
            // Scroll the conversation history.
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(SCROLL_STEP);
                Action::None
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(SCROLL_STEP);
                Action::None
            }
            KeyCode::Backspace => self.edit(|e| e.backspace()),
            KeyCode::Delete => self.edit(|e| e.delete()),
            KeyCode::Left if ctrl => self.edit(|e| e.word_left()),
            KeyCode::Right if ctrl => self.edit(|e| e.word_right()),
            KeyCode::Left => self.edit(|e| e.left()),
            KeyCode::Right => self.edit(|e| e.right()),
            KeyCode::Home => self.edit(|e| e.home()),
            KeyCode::End => self.edit(|e| e.end()),
            // Up/Down move between logical lines; at the top/bottom edge they
            // step through input history instead.
            KeyCode::Up => {
                if !self.editor.up() {
                    self.editor.history_prev();
                }
                Action::None
            }
            KeyCode::Down => {
                if !self.editor.down() {
                    self.editor.history_next();
                }
                Action::None
            }
            KeyCode::Char(c) => self.edit(|e| e.insert_char(c)),
            _ => Action::None,
        }
    }

    /// Run an editor mutation and report nothing observable to the loop.
    fn edit(&mut self, f: impl FnOnce(&mut LineEditor)) -> Action {
        f(&mut self.editor);
        Action::None
    }

    // ---- motion (Design §6.4) --------------------------------------------

    /// Whether the model is actively working — drives the spinner and the
    /// streaming accent glow. Off during a permission prompt or a question
    /// prompt, and when motion is disabled (those screens are perfectly still).
    #[must_use]
    pub fn is_working(&self) -> bool {
        self.motion && self.busy && !self.is_deciding()
    }

    /// Whether *anything* is animating right now — the working spinner/glow, or
    /// a transient effect (overlay ease-in, sidebar settle). The ticker redraws
    /// only while this is true, so idle screens stay quiet. Always false during
    /// a decision prompt or with motion off.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        if !self.motion || self.is_deciding() {
            return false;
        }
        self.busy || self.overlay_ease > 0 || self.sidebar_settle > 0
    }

    /// Whether a decision prompt (permission, question, or loop halt) is open —
    /// those screens are perfectly still (Design §5, §5.1, §6.4, §8.5).
    #[must_use]
    fn is_deciding(&self) -> bool {
        self.pending_permission.is_some()
            || self.pending_ask.is_some()
            || self.pending_loop_halt.is_some()
    }

    /// Advance one animation frame. Called by the ticker only while
    /// [`is_animating`](Self::is_animating) — so the frame count doubles as an
    /// elapsed timer without a wall clock. Transient effects count down here.
    pub fn tick(&mut self) {
        self.anim_frame = self.anim_frame.wrapping_add(1);
        self.overlay_ease = self.overlay_ease.saturating_sub(1);
        self.sidebar_settle = self.sidebar_settle.saturating_sub(1);
    }

    /// Ease-in progress for the overlay, 0.0 (just opened) → 1.0 (settled). Used
    /// to scale the overlay's size for its brief expansion.
    #[must_use]
    pub fn overlay_ease_progress(&self) -> f32 {
        if self.overlay_ease == 0 {
            return 1.0;
        }
        1.0 - f32::from(self.overlay_ease) / f32::from(EASE_FRAMES)
    }

    /// Whether the newest modified-file entry is still settling (highlighted).
    #[must_use]
    pub fn sidebar_settling(&self) -> bool {
        self.sidebar_settle > 0
    }

    /// The current animation frame (for phase-based effects like the glow).
    #[must_use]
    pub fn anim_frame(&self) -> usize {
        self.anim_frame
    }

    /// The current spinner glyph.
    #[must_use]
    pub fn spinner_glyph(&self) -> &'static str {
        SPINNER[self.anim_frame % SPINNER.len()]
    }

    /// Elapsed seconds shown next to the spinner, once past the threshold
    /// (Design §6.3). `None` before then.
    #[must_use]
    pub fn spinner_elapsed(&self) -> Option<usize> {
        let secs = self.anim_frame / ANIM_FPS;
        (secs >= ELAPSED_AFTER_SECS).then_some(secs)
    }

    /// A dull, truthful verb phrase for the spinner (Design §6.3).
    #[must_use]
    pub fn spinner_verb(&self) -> &'static str {
        let tool_running = self
            .conversation
            .iter()
            .rev()
            .any(|i| matches!(i, ConvItem::Tool { done: None, .. }));
        if tool_running {
            "working"
        } else if self.streaming {
            "responding"
        } else {
            "thinking"
        }
    }

    /// Handle a mouse-wheel scroll, routed to whatever is focused: an open
    /// overlay, the permission prompt, or the conversation history. `up` means
    /// scrolling toward older content.
    pub fn on_scroll(&mut self, up: bool) {
        let step = 3;
        // Modal priority mirrors `on_key` (palette > overlay > permission >
        // conversation, Design §3.3/§3.4): the wheel scrolls the focused
        // surface, so an open palette takes the wheel before any lower pane.
        if self.palette.is_some() {
            // The palette viewport follows `selected` (the render windows the
            // list around it), so moving the selection is exactly how the list
            // scrolls — the same action as the Up/Down keys (keyboard parity,
            // §3.4). One item per wheel notch, matching a single arrow press.
            let last = commands::matches(self.palette_query())
                .len()
                .saturating_sub(1);
            if let Some(p) = self.palette.as_mut() {
                p.selected = if up {
                    p.selected.saturating_sub(1)
                } else {
                    (p.selected + 1).min(last)
                };
            }
            return;
        }
        if let Some(o) = self.overlays.last_mut() {
            o.scroll = if up {
                o.scroll.saturating_sub(step)
            } else {
                o.scroll.saturating_add(step)
            };
        } else if self.pending_permission.is_some() {
            self.permission_scroll = if up {
                self.permission_scroll.saturating_sub(step)
            } else {
                self.permission_scroll.saturating_add(step)
            };
        } else {
            // Conversation scroll is measured from the bottom: wheel-up moves
            // back into history (larger offset).
            self.scroll = if up {
                self.scroll.saturating_add(step)
            } else {
                self.scroll.saturating_sub(step)
            };
        }
    }

    /// Handle an unmodified left click at `(col, row)` (Design §3.4). A click is
    /// a shortcut for "focus + Enter": it resolves against the last frame's
    /// hit-map and then reuses the **exact same keyboard handler** the Enter key
    /// would — the mouse adds no capability the keyboard lacks (the §3.4
    /// invariant). A click on nothing interactive is inert. Returns the `Action`
    /// the keypress would, so the frontend loop routes it identically.
    pub fn on_click(&mut self, col: u16, row: u16) -> Action {
        let Some(target) = self.hit_map.hit(col, row) else {
            return Action::None;
        };
        match target {
            ClickTarget::PaletteRow(row) => {
                // Focus the clicked row, then activate it exactly as palette
                // Enter does (on_palette_key) — no separate dispatch path.
                if let Some(p) = self.palette.as_mut() {
                    p.selected = row;
                }
                self.on_palette_key(KeyEvent::from(KeyCode::Enter))
            }
            ClickTarget::ChoiceRow(row) => {
                // Focus the clicked choice, then confirm it exactly as picker
                // Enter does (on_choice_picker_key).
                self.set_choice_selection(row);
                self.on_choice_picker_key(KeyEvent::from(KeyCode::Enter))
            }
        }
    }

    /// Insert pasted text (bracketed paste) into the input, unless a permission
    /// prompt or overlay is open — nothing may be typed into a decision, and an
    /// overlay is read-only (Design §5, §4.2).
    pub fn on_paste(&mut self, text: &str) {
        if !self.is_deciding() && self.overlays.is_empty() && self.palette.is_none() {
            self.editor.insert_str(text);
        }
    }

    /// Keys while a permission prompt is open (Design §5). Scrolling reviews the
    /// full content; only `y`/`s` allow (deliberate); Enter/Esc/`d`/`n` deny
    /// (the safe default). Any other key is ignored — no accidental decision in
    /// either direction, and nothing auto-scrolls under the user.
    fn on_permission_key(&mut self, id: PermissionId, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => {
                self.permission_scroll = self.permission_scroll.saturating_sub(1);
                Action::None
            }
            KeyCode::Down => {
                self.permission_scroll = self.permission_scroll.saturating_add(1);
                Action::None
            }
            KeyCode::PageUp => {
                self.permission_scroll = self.permission_scroll.saturating_sub(SCROLL_STEP);
                Action::None
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.permission_scroll = self.permission_scroll.saturating_add(SCROLL_STEP);
                Action::None
            }
            KeyCode::Home => {
                self.permission_scroll = 0;
                Action::None
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.decide(id, PermissionDecision::AllowOnce)
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.decide(id, PermissionDecision::AllowForSession)
            }
            KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Char('d')
            | KeyCode::Char('D')
            | KeyCode::Char('n')
            | KeyCode::Char('N') => self.decide(id, PermissionDecision::Deny),
            // Everything else: ignored. Decisions are deliberate.
            _ => Action::None,
        }
    }

    fn decide(&mut self, id: PermissionId, decision: PermissionDecision) -> Action {
        self.pending_permission = None;
        self.permission_scroll = 0;
        Action::Command(Command::PermissionAnswer { id, decision })
    }

    /// Keys while a question prompt is open (T-8, Design §5.1). Typing edits the
    /// free-text answer; ↑/↓ move the option selection; Enter submits the typed
    /// text if any, else the highlighted option, else **nothing** — Enter never
    /// auto-answers. Esc declines (a real answer). No key silently decides.
    fn on_ask_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(prompt) = self.pending_ask.as_mut() else {
            return Action::None;
        };
        match key.code {
            // Dismiss = an explicit decline returned to the model (Design §5.1).
            KeyCode::Esc => self.answer_ask(AskAnswer::Declined),
            KeyCode::Up => {
                prompt.selected = match prompt.selected {
                    None | Some(0) => None,
                    Some(i) => Some(i - 1),
                };
                Action::None
            }
            KeyCode::Down if !prompt.options.is_empty() => {
                let last = prompt.options.len() - 1;
                prompt.selected = Some(prompt.selected.map_or(0, |i| (i + 1).min(last)));
                Action::None
            }
            KeyCode::Enter => {
                let text = prompt.editor.text().trim().to_string();
                if !text.is_empty() {
                    return self.answer_ask(AskAnswer::Answered(text));
                }
                if let Some(option) = prompt.selected.and_then(|i| prompt.options.get(i)) {
                    let answer = AskAnswer::Answered(option.clone());
                    return self.answer_ask(answer);
                }
                // Nothing typed, nothing chosen: ignored (no unsafe default).
                Action::None
            }
            // A literal newline in the free-text answer (multi-line), matching
            // the main input's Ctrl+J affordance.
            KeyCode::Char('j') if ctrl => {
                prompt.editor.newline();
                Action::None
            }
            KeyCode::Backspace => {
                prompt.editor.backspace();
                Action::None
            }
            KeyCode::Char(c) if !ctrl => {
                prompt.editor.insert_char(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn answer_ask(&mut self, answer: AskAnswer) -> Action {
        let Some(prompt) = self.pending_ask.take() else {
            return Action::None;
        };
        Action::Command(Command::AskUserAnswer {
            id: prompt.id,
            answer,
        })
    }

    /// Keys while the loop-halt surface is open (S-5, Design §8.5). The menu:
    /// `g` keep going, `s` stop, `t`/Enter say something. In the steer field:
    /// type a message, Enter sends it, Esc goes back to the menu. Esc on the menu
    /// stops (the conservative choice — the loop halted to avoid wasted spend).
    fn on_loop_halt_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(prompt) = self.pending_loop_halt.as_mut() else {
            return Action::None;
        };
        if prompt.steering {
            return match key.code {
                KeyCode::Esc => {
                    prompt.steering = false;
                    prompt.editor.clear();
                    Action::None
                }
                KeyCode::Enter => {
                    let text = prompt.editor.text().trim().to_string();
                    if text.is_empty() {
                        Action::None
                    } else {
                        self.resolve_loop(LoopResolution::Steer(text))
                    }
                }
                KeyCode::Char('j') if ctrl => {
                    prompt.editor.newline();
                    Action::None
                }
                KeyCode::Backspace => {
                    prompt.editor.backspace();
                    Action::None
                }
                KeyCode::Char(c) if !ctrl => {
                    prompt.editor.insert_char(c);
                    Action::None
                }
                _ => Action::None,
            };
        }
        match key.code {
            KeyCode::Char('g') | KeyCode::Char('G') => self.resolve_loop(LoopResolution::Resume),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Esc => {
                self.resolve_loop(LoopResolution::Stop)
            }
            KeyCode::Char('t') | KeyCode::Char('T') | KeyCode::Enter => {
                prompt.steering = true;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn resolve_loop(&mut self, resolution: LoopResolution) -> Action {
        self.pending_loop_halt = None;
        Action::Command(Command::ResolveLoop { resolution })
    }

    // ---- overlays ---------------------------------------------------------

    /// Open the diff overlay for the most-recently-modified file, if any.
    pub fn open_last_diff(&mut self) {
        if let Some(path) = self.last_modified.clone() {
            if let Some(unified) = self.latest_diffs.get(&path) {
                let title = format!("diff: {path}");
                self.push_overlay(Overlay {
                    title,
                    content: OverlayContent::Diff(unified.clone()),
                    scroll: 0,
                });
            }
        }
    }

    /// Push an arbitrary text overlay (help, untruncated output — group 8).
    pub fn open_text_overlay(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push_overlay(Overlay {
            title: title.into(),
            content: OverlayContent::Text(body.into()),
            scroll: 0,
        });
    }

    // ---- memory inspector (`/memory`, FR-6, Design §4.9) ------------------

    /// `/memory` — open the memory inspector. Requests the grouped entry list;
    /// the overlay opens (or refreshes) when the `MemoryEntries` reply arrives.
    /// The engine owns the store, so the TUI never reads memory files directly.
    fn open_memory_inspector(&mut self) -> Action {
        Action::Command(Command::MemoryList)
    }

    /// Apply a `MemoryEntries` reply: refresh an already-open inspector in place
    /// (so a post-mutation re-list updates the list without a flash), or open a
    /// fresh one. The project group is simply empty on an untrusted root (FR-1).
    fn apply_memory_entries(&mut self, user: Vec<EntrySummary>, project: Vec<EntrySummary>) {
        let total = user.len() + project.len();
        let existing = self
            .overlays
            .iter()
            .rposition(|o| matches!(o.content, OverlayContent::MemoryEntries { .. }));
        match existing {
            Some(i) => {
                if let OverlayContent::MemoryEntries {
                    user: u,
                    project: p,
                    selected,
                    confirm_delete,
                } = &mut self.overlays[i].content
                {
                    *u = user;
                    *p = project;
                    *selected = (*selected).min(total.saturating_sub(1));
                    // A refresh cancels any half-finished confirm — the list it
                    // referred to just changed under it.
                    *confirm_delete = false;
                }
            }
            None => self.push_overlay(Overlay {
                title: crate::strings::memory::TITLE.into(),
                content: OverlayContent::MemoryEntries {
                    user,
                    project,
                    selected: 0,
                    confirm_delete: false,
                },
                scroll: 0,
            }),
        }
    }

    /// Apply a `MemoryBody` reply, dispatched by the intent recorded when the
    /// fetch was issued: view opens the body read-only; edit stages it for the
    /// `$EDITOR` handoff the frontend loop performs. A reply that no longer
    /// matches the recorded target (a stale/late arrival) is ignored.
    fn apply_memory_body(&mut self, scope: MemoryScope, name: &str, body: String) {
        let Some((intent, target)) = self.memory_fetch.take() else {
            return;
        };
        if target.scope != scope || target.name != name {
            return;
        }
        match intent {
            MemoryFetchIntent::View => {
                let title = format!("{} · {}", target.name, memory_scope_label(scope));
                let shown = if body.trim().is_empty() {
                    crate::strings::memory::EMPTY_BODY.to_string()
                } else {
                    body
                };
                self.open_text_overlay(title, shown);
            }
            MemoryFetchIntent::Edit => {
                self.pending_memory_edit = Some(PendingMemoryEdit {
                    scope,
                    name: target.name.clone(),
                    description: Some(target.description.clone()).filter(|d| !d.is_empty()),
                    type_: target.type_.clone(),
                    body,
                });
            }
        }
    }

    /// The entry currently highlighted in the memory inspector (flattened over
    /// the user then project groups), if any.
    fn selected_memory_entry(&self) -> Option<EntrySummary> {
        match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::MemoryEntries {
                user,
                project,
                selected,
                ..
            }) => user.iter().chain(project.iter()).nth(*selected).cloned(),
            _ => None,
        }
    }

    /// Issue a `MemoryView` for the highlighted entry, recording the intent so
    /// the `MemoryBody` reply is routed to view or edit.
    fn begin_memory_fetch(&mut self, intent: MemoryFetchIntent) -> Action {
        match self.selected_memory_entry() {
            Some(target) => {
                let cmd = Command::MemoryView {
                    scope: target.scope,
                    name: target.name.clone(),
                };
                self.memory_fetch = Some((intent, target));
                Action::Command(cmd)
            }
            None => Action::None,
        }
    }

    fn set_memory_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::MemoryEntries { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    fn set_memory_confirm(&mut self, on: bool) {
        if let Some(Overlay {
            content: OverlayContent::MemoryEntries { confirm_delete, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *confirm_delete = on;
        }
    }

    /// Drain a staged memory edit for the frontend loop's `$EDITOR` handoff
    /// (FR-6, §4.6).
    pub fn take_pending_memory_edit(&mut self) -> Option<PendingMemoryEdit> {
        self.pending_memory_edit.take()
    }

    /// Keys for the memory inspector (FR-6, Design §4.9): ↑/↓ move, Enter views
    /// the body, `e` edits it via `$EDITOR`, `d` starts a confirmed delete,
    /// Esc/q dismiss. Delete is destructive, so it takes an explicit y/N step —
    /// never a lone key (§3.4 spirit).
    fn on_memory_inspector_key(&mut self, key: KeyEvent) -> Action {
        let (user_len, project_len, selected, confirm) =
            match self.overlays.last().map(|o| &o.content) {
                Some(OverlayContent::MemoryEntries {
                    user,
                    project,
                    selected,
                    confirm_delete,
                }) => (user.len(), project.len(), *selected, *confirm_delete),
                _ => return Action::None,
            };
        let total = user_len + project_len;

        // The confirm-delete step owns the keyboard until resolved: only `y`
        // deletes; every other key cancels (deny-by-default for a destructive
        // action).
        if confirm {
            self.set_memory_confirm(false);
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                if let Some(target) = self.selected_memory_entry() {
                    return Action::Command(Command::MemoryMutate {
                        op: MemoryOp::Remove,
                        scope: target.scope,
                        name: target.name,
                        description: None,
                        type_: None,
                        body: None,
                    });
                }
            }
            return Action::None;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_memory_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_memory_selection((selected + 1).min(total.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => self.begin_memory_fetch(MemoryFetchIntent::View),
            KeyCode::Char('e') => self.begin_memory_fetch(MemoryFetchIntent::Edit),
            KeyCode::Char('d') => {
                if total > 0 {
                    self.set_memory_confirm(true);
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    // ---- skills inspector (`/skills`, FR-7, Design §4.9) ------------------

    /// `/skills` — open the skills inspector. The catalog is already cached
    /// (`SkillsAvailable`), so the list overlay opens immediately with no engine
    /// round-trip; only a selected skill's *body* is fetched on demand (§8.6).
    fn open_skills_inspector(&mut self) -> Action {
        self.push_overlay(Overlay {
            title: crate::strings::skills::TITLE.into(),
            content: OverlayContent::SkillList {
                skills: self.skills.clone(),
                selected: 0,
            },
            scroll: 0,
        });
        Action::None
    }

    /// Apply a `SkillBody` reply: open the instruction body **read-only** on top
    /// of the list (§4.9 — inspectable before it ever runs). Bundled resource
    /// paths are appended so "what the skill bundles" is visible too. Fetching
    /// the body for display runs no bundled script (FR-7).
    fn apply_skill_body(
        &mut self,
        name: &str,
        origin: SkillOrigin,
        body: String,
        resources: &[String],
    ) {
        let title = format!("{name} · {}", skill_origin_label(origin));
        let mut text = if body.trim().is_empty() {
            crate::strings::skills::EMPTY_BODY.to_string()
        } else {
            body
        };
        if !resources.is_empty() {
            text.push_str("\n\n");
            text.push_str(crate::strings::skills::RESOURCES_HEADER);
            text.push('\n');
            for r in resources {
                text.push_str(&format!("- {r}\n"));
            }
        }
        self.open_text_overlay(title, text);
    }

    fn set_skill_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::SkillList { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    /// Keys for the skills inspector (FR-7, Design §4.9): ↑/↓ move, Enter fetches
    /// and shows the selected skill's body read-only, Esc/q dismiss. There is no
    /// edit or delete — skills are externally-authored folders (read-only here).
    fn on_skills_inspector_key(&mut self, key: KeyEvent) -> Action {
        let (len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::SkillList { skills, selected }) => {
                (skills.len(), *selected, skills.get(*selected).cloned())
            }
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_skill_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_skill_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => match chosen {
                Some(skill) => Action::Command(Command::InspectSkill { name: skill.name }),
                None => Action::None,
            },
            _ => Action::None,
        }
    }

    /// Open the model/provider picker (`/model` with no args, the palette, or a
    /// keybinding — C-6). Rows are the configured profiles, the active one
    /// marked; Enter issues a `SwitchModel`.
    fn open_model_picker(&mut self) {
        if self.profiles.is_empty() {
            self.conversation.push(ConvItem::Notice(
                "no provider profiles configured — add one in .agents/config.toml".into(),
            ));
            return;
        }
        let active = self.session.provider.clone();
        let rows: Vec<ChoiceRow> = self
            .profiles
            .iter()
            .map(|name| ChoiceRow {
                label: name.clone(),
                current: *name == active,
            })
            .collect();
        let selected = rows.iter().position(|r| r.current).unwrap_or(0);
        self.push_overlay(Overlay {
            title: "switch model".into(),
            content: OverlayContent::Choices {
                kind: ChoiceKind::Model,
                rows,
                selected,
            },
            scroll: 0,
        });
    }

    /// Open the reasoning-effort picker (`/effort`, P-9): the active model's
    /// levels, current one marked; Enter issues a `SetEffort`. A model with no
    /// effort control declines with a calm notice.
    fn open_effort_picker(&mut self) {
        if self.effort_levels.is_empty() {
            self.conversation.push(ConvItem::Notice(
                "this model has no reasoning-effort control".into(),
            ));
            return;
        }
        let rows: Vec<ChoiceRow> = self
            .effort_levels
            .iter()
            .map(|level| ChoiceRow {
                label: level.as_str().to_string(),
                current: self.effort == Some(*level),
            })
            .collect();
        let selected = rows.iter().position(|r| r.current).unwrap_or(0);
        self.push_overlay(Overlay {
            title: "reasoning effort".into(),
            content: OverlayContent::Choices {
                kind: ChoiceKind::Effort,
                rows,
                selected,
            },
            scroll: 0,
        });
    }

    /// Open the permission-mode picker (`/mode` with no args or the palette).
    /// Rows are the three tiers, the current one marked; the auto tiers are
    /// marked unavailable (and not selectable) when OS confinement is not
    /// active — the engine would refuse them anyway, so the picker says so up
    /// front (Requirements §6.4, §6.7).
    fn open_mode_picker(&mut self) {
        let auto_ok = self
            .sandbox
            .as_ref()
            .is_some_and(SandboxStatus::allows_auto_modes);
        let rows: Vec<ChoiceRow> = [Mode::Normal, Mode::AutoAcceptEdits, Mode::Auto]
            .iter()
            .map(|&m| {
                let label = mode_label(m, auto_ok);
                ChoiceRow {
                    label,
                    current: m == self.mode,
                }
            })
            .collect();
        let selected = rows.iter().position(|r| r.current).unwrap_or(0);
        self.push_overlay(Overlay {
            title: "permission mode".into(),
            content: OverlayContent::Choices {
                kind: ChoiceKind::Mode,
                rows,
                selected,
            },
            scroll: 0,
        });
    }

    /// Handle `/mode [name]`: no arg opens the picker; an arg sets the mode
    /// directly, validated against confinement (the auto tiers need an active
    /// sandbox — Requirements §6.4).
    fn mode_command(&mut self, arg: &str) -> Action {
        let arg = arg.trim();
        if arg.is_empty() {
            self.open_mode_picker();
            return Action::None;
        }
        match parse_mode(arg) {
            Some(Mode::Normal) => Action::Command(Command::SetMode { mode: Mode::Normal }),
            Some(requested) => {
                let auto_ok = self
                    .sandbox
                    .as_ref()
                    .is_some_and(SandboxStatus::allows_auto_modes);
                if auto_ok {
                    Action::Command(Command::SetMode { mode: requested })
                } else {
                    self.conversation.push(ConvItem::Notice(
                        "auto-accept modes are unavailable without OS confinement".into(),
                    ));
                    Action::None
                }
            }
            None => {
                self.conversation.push(ConvItem::Notice(format!(
                    "unknown mode '{arg}' — try: normal, auto-accept-edits, auto"
                )));
                Action::None
            }
        }
    }

    /// Handle `/effort [level]`: no arg opens the picker; an arg sets the level
    /// directly, validated against the model's declared levels (P-9).
    fn effort_command(&mut self, arg: &str) -> Action {
        let arg = arg.trim();
        if arg.is_empty() {
            self.open_effort_picker();
            return Action::None;
        }
        match Effort::parse(arg) {
            Some(level) if self.effort_levels.contains(&level) => {
                Action::Command(Command::SetEffort { effort: level })
            }
            Some(level) if self.effort_levels.is_empty() => {
                self.conversation.push(ConvItem::Notice(format!(
                    "this model has no reasoning-effort control (ignoring '{level}')"
                )));
                Action::None
            }
            Some(level) => {
                let offered = self
                    .effort_levels
                    .iter()
                    .map(Effort::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                self.conversation.push(ConvItem::Notice(format!(
                    "this model does not offer '{level}' — try: {offered}"
                )));
                Action::None
            }
            None => {
                self.conversation.push(ConvItem::Notice(format!(
                    "unknown effort '{arg}' — try low, medium, high, or max"
                )));
                Action::None
            }
        }
    }

    /// Toggle the most recent reasoning trail open/closed (the expand
    /// affordance, Design §4.4).
    fn toggle_reasoning(&mut self) {
        for item in self.conversation.iter_mut().rev() {
            if let ConvItem::Reasoning { expanded, .. } = item {
                *expanded = !*expanded;
                return;
            }
        }
    }

    /// The project's `.agents/` directory, derived from the sessions dir
    /// (`<root>/.agents/sessions`).
    fn agents_dir(&self) -> PathBuf {
        self.sessions_dir
            .parent()
            .map_or_else(|| self.sessions_dir.clone(), Path::to_path_buf)
    }

    /// `/config` — edit the project `.agents/config.toml` in `$EDITOR` (C-5).
    /// Seeds it from the init template (same content `emberly init` writes) if
    /// the project has none yet (C-1/C-2); the write lands in the project tier.
    fn edit_config(&mut self) -> Action {
        match crate::edit::config_target(&self.agents_dir(), &self.config_template) {
            Ok((path, existed)) => {
                // Provenance before the edit (C-3): existing project value vs a
                // fresh override seeded from the defaults. Edits land here (C-1).
                self.conversation.push(ConvItem::Notice(if existed {
                    format!("editing your project config — {}", path.display())
                } else {
                    format!(
                        "no project config yet — created {} from the template; \
                         your edits override the defaults",
                        path.display()
                    )
                }));
                Action::EditFile(path)
            }
            Err(e) => {
                self.conversation
                    .push(ConvItem::Notice(format!("could not prepare config: {e}")));
                Action::None
            }
        }
    }

    /// `/prompt [name]` — edit a prompt file (`system` | `compact`, default
    /// `system`) in `$EDITOR` (C-5). Seeds from the baked-in default (C-1) if
    /// the project has no override yet.
    fn edit_prompt(&mut self, name: &str) -> Action {
        match crate::edit::prompt_target(&self.agents_dir(), name.trim()) {
            Ok((path, existed)) => {
                let shown = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("prompt");
                // Provenance before the edit (C-3): an existing project override
                // vs a fresh copy of the baked-in default. Edits land here (C-1).
                self.conversation.push(ConvItem::Notice(if existed {
                    format!("editing your project '{shown}' prompt — {}", path.display())
                } else {
                    format!(
                        "no project '{shown}' prompt yet — created {} from the baked-in \
                         default; your edits override it",
                        path.display()
                    )
                }));
                Action::EditFile(path)
            }
            Err(msg) => {
                self.conversation.push(ConvItem::Notice(msg));
                Action::None
            }
        }
    }

    /// Report an `$EDITOR` handoff's outcome as a timeline notice (C-5).
    pub fn note_edit(&mut self, path: &Path, status: crate::edit::EditStatus) {
        use crate::edit::EditStatus;
        let message = match status {
            // A ReloadConfig follows (C-5), which reports what actually changed.
            EditStatus::Edited => format!("edited {}", path.display()),
            EditStatus::NoEditor => {
                "no editor configured — set $EDITOR or $VISUAL, then try again".to_string()
            }
            EditStatus::Failed(why) => format!("editor failed: {why}"),
        };
        self.conversation.push(ConvItem::Notice(message));
    }

    /// Push an overlay and start its brief ease-in (Design §6.4).
    fn push_overlay(&mut self, overlay: Overlay) {
        self.overlays.push(overlay);
        if self.motion {
            self.overlay_ease = EASE_FRAMES;
        }
    }

    /// The overlay on top, if any (read by the renderer).
    #[must_use]
    pub fn active_overlay(&self) -> Option<&Overlay> {
        self.overlays.last()
    }

    // ---- commands & palette ----------------------------------------------

    /// Run a typed `/name` command; unknown names surface a calm notice.
    fn run_slash(&mut self, name: &str) -> Action {
        let name = name.trim();
        // `/model <profile> [model]` takes arguments, so it is parsed before the
        // argument-less command registry (C-6). `modelx` is not a match.
        if let Some(rest) = name
            .strip_prefix("model")
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return self.run_model_command(rest.trim());
        }
        // `/prompt [name]` takes an optional argument (system|compact).
        if let Some(rest) = name
            .strip_prefix("prompt")
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return self.edit_prompt(rest.trim());
        }
        // `/effort [level]` takes an optional argument (low|medium|high|max).
        if let Some(rest) = name
            .strip_prefix("effort")
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return self.effort_command(rest.trim());
        }
        // `/mode [name]` opens the picker with no arg, or sets the mode
        // directly (Shift-Tab still cycles — the quick-toggle keybinding).
        if let Some(rest) = name
            .strip_prefix("mode")
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return self.mode_command(rest.trim());
        }
        match commands::by_name(name) {
            Some(cmd) => self.run_command(cmd),
            None => {
                self.conversation.push(ConvItem::Notice(format!(
                    "unknown command: /{name} — Ctrl-P lists commands"
                )));
                Action::None
            }
        }
    }

    /// `/model <profile> [model]` — switch the active provider profile (and
    /// optionally the model) for subsequent turns (C-6). Gated at idle, like
    /// `/new`: a switch applies to the next turn.
    fn run_model_command(&mut self, args: &str) -> Action {
        let mut parts = args.split_whitespace();
        // No arguments → open the picker (browsing is fine even mid-turn).
        let Some(profile) = parts.next() else {
            self.open_model_picker();
            return Action::None;
        };
        if self.busy {
            self.conversation.push(ConvItem::Notice(
                "finish or cancel the current turn before switching models".into(),
            ));
            return Action::None;
        }
        let model = parts.next().map(str::to_string);
        Action::Command(Command::SwitchModel {
            profile: profile.to_string(),
            model,
        })
    }

    /// Execute a command from the palette, a slash command, or a keybinding.
    /// One place maps each [`AppCommand`] to its effect.
    pub fn run_command(&mut self, cmd: AppCommand) -> Action {
        match cmd {
            AppCommand::Help => {
                self.open_text_overlay("commands", help_text());
                Action::None
            }
            AppCommand::View => {
                match self.last_assistant_text() {
                    Some(text) => self.open_text_overlay("message", text),
                    None => self.open_text_overlay("message", "(no assistant message yet)"),
                }
                Action::None
            }
            AppCommand::Diff => {
                self.open_last_diff();
                Action::None
            }
            AppCommand::Files => {
                self.open_text_overlay("modified files", self.files_text());
                Action::None
            }
            AppCommand::Session => {
                let rows = self.session_rows();
                self.push_overlay(Overlay {
                    title: "sessions".into(),
                    content: OverlayContent::Sessions { rows, selected: 0 },
                    scroll: 0,
                });
                Action::None
            }
            AppCommand::NewSession => {
                // A switch resets the conversation, so refuse mid-turn — the
                // frontend gates it here rather than dropping it in the engine.
                if self.busy {
                    self.conversation.push(ConvItem::Notice(
                        "finish or cancel the current turn before starting a new session".into(),
                    ));
                    Action::None
                } else {
                    Action::NewSession
                }
            }
            AppCommand::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                Action::None
            }
            AppCommand::CycleMode => {
                // Cycle to the next tier; the engine is the single gate that
                // resolves it against confinement (Tech Spec §6.6) and emits a
                // ModeChanged on success or an explanatory Notice on refusal, so
                // the frontend just proposes the next tier.
                let next = match self.mode {
                    emberly_core::Mode::Normal => emberly_core::Mode::AutoAcceptEdits,
                    emberly_core::Mode::AutoAcceptEdits => emberly_core::Mode::Auto,
                    emberly_core::Mode::Auto => emberly_core::Mode::Normal,
                };
                Action::Command(Command::SetMode { mode: next })
            }
            AppCommand::Model => {
                self.open_model_picker();
                Action::None
            }
            AppCommand::Effort => {
                self.open_effort_picker();
                Action::None
            }
            AppCommand::Memory => self.open_memory_inspector(),
            AppCommand::Skills => self.open_skills_inspector(),
            AppCommand::Config => self.edit_config(),
            AppCommand::Prompt => self.edit_prompt("system"),
            AppCommand::Reload => Action::Command(Command::ReloadConfig),
            AppCommand::Compact => Action::Command(Command::Compact),
            AppCommand::Cancel => Action::Command(Command::Cancel),
            AppCommand::Quit => Action::Quit,
        }
    }

    fn last_assistant_text(&self) -> Option<String> {
        self.conversation.iter().rev().find_map(|item| match item {
            ConvItem::Assistant(text) => Some(text.clone()),
            _ => None,
        })
    }

    fn files_text(&self) -> String {
        if self.modified_files.is_empty() {
            return "No files changed yet.".to_string();
        }
        self.modified_files
            .iter()
            .map(|f| format!("{}  +{} -{}", f.path, f.adds, f.dels))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Build the session-picker rows from the transcripts on disk, newest
    /// first, marking the one we are currently in.
    fn session_rows(&self) -> Vec<SessionRow> {
        resume::list_sessions(&self.sessions_dir)
            .into_iter()
            .map(|s| {
                let flag = if s.interrupted { " · interrupted" } else { "" };
                SessionRow {
                    current: s.id == self.session.session_id,
                    title: s.title.unwrap_or_else(|| "(untitled)".into()),
                    subtitle: format!("{}/{} · {} events{flag}", s.provider, s.model, s.events),
                    id: s.id,
                }
            })
            .collect()
    }

    /// Reset the timeline and identity for a brand-new session started in place
    /// (`/new`). Driven by the frontend once the engine has been told; the
    /// engine's follow-up context-usage event refines the counters.
    pub fn begin_new_session(&mut self, id: SessionId) {
        self.reset_for_switch(id, String::new());
        self.conversation
            .push(ConvItem::Notice("started a new session".into()));
    }

    /// Reset and reseed the timeline for a resumed session (`/resume` from the
    /// picker), restoring its history so it is not a blank pane.
    pub fn begin_resumed_session(
        &mut self,
        id: SessionId,
        title: String,
        records: &[TranscriptRecord],
    ) {
        self.reset_for_switch(id, title);
        self.seed_history(records);
        self.conversation
            .push(ConvItem::Notice("resumed session".into()));
    }

    /// Push a harness-voice notice into the timeline (used by the frontend for
    /// out-of-band feedback such as a failed session switch).
    pub fn notice(&mut self, message: impl Into<String>) {
        self.conversation.push(ConvItem::Notice(message.into()));
    }

    /// Shared reset for both switch paths: clear the conversation and per-session
    /// view state, adopt the new identity.
    fn reset_for_switch(&mut self, id: SessionId, title: String) {
        self.session.session_id = id;
        self.session.title = title;
        self.conversation.clear();
        self.modified_files.clear();
        self.tasks.clear();
        self.memory_user = 0;
        self.memory_project = 0;
        self.skills.clear();
        self.memory_fetch = None;
        self.pending_memory_edit = None;
        self.latest_diffs.clear();
        self.last_modified = None;
        self.scroll = 0;
        self.streaming = false;
        self.context_pct = 0;
        self.context_tokens = 0;
        self.session_usage = TokenUsage::default();
        self.cost_usd = 0.0;
        self.cost_known = false;
        self.overlays.clear();
    }

    /// Keys while the palette is open: type to filter, ↑/↓ to move, Enter to
    /// run the selection, Esc/Ctrl+P to dismiss.
    fn on_palette_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let filtered = commands::matches(self.palette_query());
        match key.code {
            KeyCode::Esc => {
                self.palette = None;
                Action::None
            }
            KeyCode::Char('p') if ctrl => {
                self.palette = None;
                Action::None
            }
            KeyCode::Enter => {
                let chosen = self
                    .palette
                    .as_ref()
                    .and_then(|p| filtered.get(p.selected).copied());
                self.palette = None;
                match chosen {
                    // Dispatch by name so argument-taking commands (`/mode`,
                    // `/model`, `/effort`) open their picker from the palette,
                    // just as their no-arg slash forms do.
                    Some(i) => self.run_slash(commands::COMMANDS[i].name),
                    None => Action::None,
                }
            }
            KeyCode::Up => {
                if let Some(p) = self.palette.as_mut() {
                    p.selected = p.selected.saturating_sub(1);
                }
                Action::None
            }
            KeyCode::Down => {
                if let Some(p) = self.palette.as_mut() {
                    let last = filtered.len().saturating_sub(1);
                    p.selected = (p.selected + 1).min(last);
                }
                Action::None
            }
            KeyCode::Backspace => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
                Action::None
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn palette_query(&self) -> &str {
        self.palette.as_ref().map_or("", |p| p.query.as_str())
    }

    /// Keys while an overlay is open. The session picker is interactive (↑/↓
    /// move, Enter resumes); every other overlay is a scrollable, read-only
    /// pane (Esc/q dismiss; the rest scroll).
    fn on_overlay_key(&mut self, key: KeyEvent) -> Action {
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Sessions { .. })
        ) {
            return self.on_session_picker_key(key);
        }
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Choices { .. })
        ) {
            return self.on_choice_picker_key(key);
        }
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::MemoryEntries { .. })
        ) {
            return self.on_memory_inspector_key(key);
        }
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::SkillList { .. })
        ) {
            return self.on_skills_inspector_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
            }
            KeyCode::Char('c') if ctrl => {
                self.overlays.pop();
            }
            KeyCode::Up => self.scroll_overlay(-1),
            KeyCode::Down => self.scroll_overlay(1),
            KeyCode::PageUp => self.scroll_overlay(-(SCROLL_STEP as isize)),
            KeyCode::PageDown => self.scroll_overlay(SCROLL_STEP as isize),
            KeyCode::Home => {
                if let Some(o) = self.overlays.last_mut() {
                    o.scroll = 0;
                }
            }
            _ => {}
        }
        Action::None
    }

    /// Keys for the session picker: ↑/↓ move the selection, Enter resumes the
    /// highlighted session (a no-op on the current one), Esc/q dismiss.
    fn on_session_picker_key(&mut self, key: KeyEvent) -> Action {
        // Read the selection and the chosen row without holding a borrow across
        // the mutation the arms perform.
        let (len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::Sessions { rows, selected }) => {
                (rows.len(), *selected, rows.get(*selected).cloned())
            }
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_picker_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_picker_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => match chosen {
                Some(row) if row.current => {
                    self.overlays.pop();
                    self.conversation
                        .push(ConvItem::Notice("already in this session".into()));
                    Action::None
                }
                Some(row) if self.busy => {
                    self.overlays.pop();
                    self.conversation.push(ConvItem::Notice(
                        "finish or cancel the current turn before switching sessions".into(),
                    ));
                    let _ = row;
                    Action::None
                }
                Some(row) => {
                    self.overlays.pop();
                    Action::ResumeSession(row.id)
                }
                None => Action::None,
            },
            _ => Action::None,
        }
    }

    fn set_picker_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::Sessions { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    /// Keys for the generic choice picker (`/model` now): ↑/↓ move, Enter
    /// applies the highlighted choice via the command its `kind` maps to,
    /// Esc/q dismiss.
    fn on_choice_picker_key(&mut self, key: KeyEvent) -> Action {
        let (kind, len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::Choices {
                kind,
                rows,
                selected,
            }) => (*kind, rows.len(), *selected, rows.get(*selected).cloned()),
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_choice_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_choice_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => {
                let Some(row) = chosen else {
                    return Action::None;
                };
                self.overlays.pop();
                match kind {
                    ChoiceKind::Model => {
                        if row.current {
                            self.conversation
                                .push(ConvItem::Notice(format!("already using {}", row.label)));
                            Action::None
                        } else if self.busy {
                            self.conversation.push(ConvItem::Notice(
                                "finish or cancel the current turn before switching models".into(),
                            ));
                            Action::None
                        } else {
                            Action::Command(Command::SwitchModel {
                                profile: row.label,
                                model: None,
                            })
                        }
                    }
                    ChoiceKind::Effort => {
                        if row.current {
                            Action::None // already at this level
                        } else {
                            match Effort::parse(&row.label) {
                                Some(effort) => Action::Command(Command::SetEffort { effort }),
                                None => Action::None,
                            }
                        }
                    }
                    ChoiceKind::Mode => {
                        if row.current {
                            Action::None // already in this mode
                        } else {
                            // The label is the mode name, possibly with a
                            // `(needs OS confinement)` suffix when the auto
                            // tier is unavailable — parse the leading name.
                            let name = row.label.split_whitespace().next().unwrap_or("");
                            match parse_mode(name) {
                                Some(mode) if mode == Mode::Normal => {
                                    Action::Command(Command::SetMode { mode })
                                }
                                Some(mode) => {
                                    let auto_ok = self
                                        .sandbox
                                        .as_ref()
                                        .is_some_and(SandboxStatus::allows_auto_modes);
                                    if auto_ok {
                                        Action::Command(Command::SetMode { mode })
                                    } else {
                                        self.conversation.push(ConvItem::Notice(
                                            "auto-accept modes are unavailable without OS confinement"
                                                .into(),
                                        ));
                                        Action::None
                                    }
                                }
                                None => Action::None,
                            }
                        }
                    }
                }
            }
            _ => Action::None,
        }
    }

    fn set_choice_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::Choices { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    fn scroll_overlay(&mut self, delta: isize) {
        if let Some(o) = self.overlays.last_mut() {
            o.scroll = o.scroll.saturating_add_signed(delta);
        }
    }
}

/// The `/help` body: every command with its keybinding and description, from
/// the single registry (Design §3.3).
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use emberly_core::ToolCallId;

    fn app() -> App {
        App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            vec!["anthropic".into(), "openai".into(), "zai".into()],
            "# test config\n".to_string(),
        )
    }

    #[test]
    fn slash_model_switches_provider_and_model() {
        let mut a = app();
        assert_eq!(
            a.run_slash("model zai glm-4.6"),
            Action::Command(Command::SwitchModel {
                profile: "zai".into(),
                model: Some("glm-4.6".into()),
            })
        );
        // Profile only → keep-current-model (None).
        assert_eq!(
            a.run_slash("model openai"),
            Action::Command(Command::SwitchModel {
                profile: "openai".into(),
                model: None,
            })
        );
    }

    #[test]
    fn slash_model_without_args_opens_the_picker() {
        let mut a = app();
        assert_eq!(a.run_slash("model"), Action::None);
        assert!(
            matches!(
                a.overlays.last().map(|o| &o.content),
                Some(OverlayContent::Choices {
                    kind: ChoiceKind::Model,
                    ..
                })
            ),
            "no-arg /model opens the model picker"
        );
    }

    #[test]
    fn model_picker_marks_current_and_enter_switches() {
        let mut a = app();
        a.session.provider = "anthropic".into();
        a.open_model_picker();
        // The active profile is preselected and marked current.
        let Some(OverlayContent::Choices { rows, selected, .. }) =
            a.overlays.last().map(|o| &o.content)
        else {
            panic!("expected a choices overlay");
        };
        assert!(rows[*selected].current && rows[*selected].label == "anthropic");
        // Move to a different profile and press Enter → SwitchModel (keep model).
        let down = KeyEvent::from(KeyCode::Down);
        let _ = a.on_choice_picker_key(down);
        let enter = KeyEvent::from(KeyCode::Enter);
        assert!(matches!(
            a.on_choice_picker_key(enter),
            Action::Command(Command::SwitchModel { model: None, .. })
        ));
    }

    #[test]
    fn model_picker_reports_when_no_profiles() {
        let mut a = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        a.open_model_picker();
        assert!(a.overlays.is_empty(), "no overlay without profiles");
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("no provider profiles"))));
    }

    #[test]
    fn slash_modelx_is_not_the_model_command() {
        // A command whose name merely starts with "model" is not `/model`.
        let mut a = app();
        assert_eq!(a.run_slash("modelx"), Action::None);
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown command"))));
    }

    #[test]
    fn slash_mode_without_args_opens_the_picker() {
        let mut a = app();
        assert_eq!(a.run_slash("mode"), Action::None);
        assert!(
            matches!(
                a.overlays.last().map(|o| &o.content),
                Some(OverlayContent::Choices {
                    kind: ChoiceKind::Mode,
                    ..
                })
            ),
            "no-arg /mode opens the mode picker"
        );
    }

    #[test]
    fn mode_picker_marks_current_and_marks_auto_unavailable_without_sandbox() {
        let mut a = app(); // sandbox: None → auto tiers unavailable
        a.open_mode_picker();
        let Some(OverlayContent::Choices { rows, selected, .. }) =
            a.overlays.last().map(|o| &o.content)
        else {
            panic!("expected a choices overlay");
        };
        // Normal is current (the default) and preselected.
        assert!(rows[*selected].current);
        assert_eq!(rows[*selected].label, "normal");
        // Auto tiers are annotated as needing confinement.
        assert!(rows
            .iter()
            .any(|r| r.label.starts_with("auto-accept-edits") && r.label.contains("OS confinement")));
        assert!(rows
            .iter()
            .any(|r| r.label == "auto  (needs OS confinement)" || r.label.starts_with("auto  (")));
    }

    #[test]
    fn mode_picker_switches_to_normal_on_enter() {
        let mut a = app();
        a.mode = Mode::Auto;
        a.sandbox = Some(SandboxStatus::Confined { backend: "landlock".into() });
        a.open_mode_picker();
        // Current is Auto; move up to AutoAcceptEdits, then up to Normal.
        let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Up));
        let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Up));
        assert!(matches!(
            a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter)),
            Action::Command(Command::SetMode { mode: Mode::Normal })
        ));
    }

    #[test]
    fn mode_picker_refuses_auto_without_confinement() {
        let mut a = app(); // sandbox: None
        a.open_mode_picker();
        // The auto tier row carries the unavailability suffix; selecting it and
        // pressing Enter declines with a notice rather than emitting SetMode.
        // Move down to the auto-accept-edits row (index 1).
        let _ = a.on_choice_picker_key(KeyEvent::from(KeyCode::Down));
        let action = a.on_choice_picker_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, Action::None);
        assert!(a.conversation.iter().any(|i| matches!(
            i,
            ConvItem::Notice(n) if n.contains("unavailable without OS confinement")
        )));
    }

    #[test]
    fn slash_mode_arg_sets_directly_and_validates() {
        let mut a = app();
        a.sandbox = Some(SandboxStatus::Confined { backend: "landlock".into() });
        // A direct arg with confinement active → SetMode.
        assert_eq!(
            a.run_slash("mode auto"),
            Action::Command(Command::SetMode { mode: Mode::Auto })
        );
        // Normal is always allowed.
        assert_eq!(
            a.run_slash("mode normal"),
            Action::Command(Command::SetMode { mode: Mode::Normal })
        );
        // Unknown mode → a notice, not a command.
        assert_eq!(a.run_slash("mode bogus"), Action::None);
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown mode"))));
    }

    #[test]
    fn slash_mode_arg_refuses_auto_without_confinement() {
        let mut a = app(); // sandbox: None
        assert_eq!(a.run_slash("mode auto-accept-edits"), Action::None);
        assert!(a.conversation.iter().any(|i| matches!(
            i,
            ConvItem::Notice(n) if n.contains("unavailable without OS confinement")
        )));
        // Normal is still allowed without confinement.
        assert_eq!(
            a.run_slash("mode normal"),
            Action::Command(Command::SetMode { mode: Mode::Normal })
        );
    }

    #[test]
    fn slash_config_seeds_then_edits_without_clobbering() {
        let root = std::env::temp_dir().join(format!("emberly-cfgedit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions = root.join(".agents").join("sessions");
        std::fs::create_dir_all(&sessions).expect("mkdir");
        let mut a = App::new(
            SessionInfo::default(),
            sessions,
            Vec::new(),
            "# seeded config\n".to_string(),
        );

        let cfg = root.join(".agents").join("config.toml");
        let action = a.run_slash("config");
        assert!(cfg.exists(), "config.toml is seeded when absent");
        assert!(matches!(action, Action::EditFile(ref p) if *p == cfg));

        // A second /config must not overwrite the (now user-edited) file.
        std::fs::write(&cfg, "user edits").expect("write");
        let _ = a.run_slash("config");
        assert_eq!(std::fs::read_to_string(&cfg).expect("read"), "user edits");
    }

    #[test]
    fn slash_prompt_seeds_from_default_and_rejects_unknown() {
        let root = std::env::temp_dir().join(format!("emberly-promptedit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions = root.join(".agents").join("sessions");
        std::fs::create_dir_all(&sessions).expect("mkdir");
        let mut a = App::new(SessionInfo::default(), sessions, Vec::new(), String::new());

        let p = root.join(".agents").join("prompts").join("system.md");
        let action = a.run_slash("prompt system");
        assert!(p.exists(), "prompt seeded from the baked-in default");
        assert!(matches!(action, Action::EditFile(ref pp) if *pp == p));
        assert!(!std::fs::read_to_string(&p).expect("read").is_empty());

        // An unknown prompt name is a notice, not an edit.
        assert_eq!(a.run_slash("prompt bogus"), Action::None);
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("unknown prompt"))));
    }

    #[test]
    fn edit_provenance_distinguishes_new_override_from_existing() {
        let root = std::env::temp_dir().join(format!("emberly-prov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions = root.join(".agents").join("sessions");
        std::fs::create_dir_all(&sessions).expect("mkdir");
        let mut a = App::new(
            SessionInfo::default(),
            sessions,
            Vec::new(),
            "# t\n".to_string(),
        );

        // First edit: no project config yet → seeded, told it overrides defaults.
        a.run_slash("config");
        assert!(matches!(
            a.conversation.last(),
            Some(ConvItem::Notice(n)) if n.contains("no project config yet")
        ));
        // Second edit: the file exists → editing an existing project value.
        a.run_slash("config");
        assert!(matches!(
            a.conversation.last(),
            Some(ConvItem::Notice(n)) if n.contains("editing your project config")
        ));
    }

    #[test]
    fn note_edit_reports_no_editor() {
        let mut a = app();
        a.note_edit(
            Path::new("/x/config.toml"),
            crate::edit::EditStatus::NoEditor,
        );
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(n) if n.contains("no editor configured"))));
    }

    #[test]
    fn model_changed_updates_the_sidebar() {
        let mut a = app();
        a.apply_event(UiEvent::ModelChanged {
            provider: "zai".into(),
            model: "glm-4.6".into(),
        });
        assert_eq!(a.session.provider, "zai");
        assert_eq!(a.session.model, "glm-4.6");
    }

    #[test]
    fn assistant_deltas_accumulate_into_one_item() {
        let mut a = app();
        a.apply_event(UiEvent::AssistantDelta { text: "Hel".into() });
        a.apply_event(UiEvent::AssistantDelta { text: "lo".into() });
        a.apply_event(UiEvent::AssistantDone);
        assert_eq!(a.conversation, vec![ConvItem::Assistant("Hello".into())]);
        assert!(!a.streaming);
    }

    #[test]
    fn tool_finished_marks_the_matching_start() {
        let mut a = app();
        let id = ToolCallId::new("c1");
        a.apply_event(UiEvent::ToolStarted {
            call_id: id.clone(),
            tool: "bash".into(),
            summary: "run: ls".into(),
            explanation: None,
        });
        a.apply_event(UiEvent::ToolFinished {
            call_id: id.clone(),
            ok: true,
            summary: "exit 0".into(),
            preview: "hello\nworld".into(),
            untrusted: false,
        });
        match &a.conversation[0] {
            ConvItem::Tool {
                done,
                summary,
                result,
                preview,
                ..
            } => {
                assert_eq!(*done, Some(true));
                // The descriptive label is kept; result + preview are separate.
                assert_eq!(summary, "run: ls");
                assert_eq!(result.as_deref(), Some("exit 0"));
                assert_eq!(preview.as_deref(), Some("hello\nworld"));
            }
            other => panic!("expected a tool item, got {other:?}"),
        }
    }

    #[test]
    fn tool_started_carries_the_explanation_onto_the_item() {
        let mut a = app();
        a.apply_event(UiEvent::ToolStarted {
            call_id: ToolCallId::new("c1"),
            tool: "bash".into(),
            summary: "run: sed …".into(),
            explanation: Some("raise the log level".into()),
        });
        match &a.conversation[0] {
            ConvItem::Tool { explanation, .. } => {
                assert_eq!(explanation.as_deref(), Some("raise the log level"));
            }
            other => panic!("expected a tool item, got {other:?}"),
        }
    }

    #[test]
    fn file_diff_shows_inline_and_opens_overlay() {
        let mut a = app();
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 1,
            dels: 0,
        });
        a.apply_event(UiEvent::FileDiff {
            path: "a.rs".into(),
            unified: "--- a/a.rs\n+++ b/a.rs\n+x".into(),
        });
        // Inline diff item recorded.
        assert!(matches!(a.conversation.last(), Some(ConvItem::Diff { .. })));
        assert_eq!(a.last_modified.as_deref(), Some("a.rs"));
        // Ctrl+O opens the overlay for the most-recent file.
        a.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(a.overlays.len(), 1);
        assert!(matches!(
            a.active_overlay().map(|o| &o.content),
            Some(OverlayContent::Diff(_))
        ));
    }

    #[test]
    fn busy_spinner_spans_the_turn_and_respects_gates() {
        let mut a = app();
        // Submitting a message enters the busy/working state.
        a.on_key(KeyEvent::from(KeyCode::Char('h')));
        a.on_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.busy);
        assert!(a.is_animating(), "spinner runs while working");
        // No elapsed time before the threshold; the glyph cycles on tick.
        assert_eq!(a.spinner_elapsed(), None);
        for _ in 0..ANIM_FPS * ELAPSED_AFTER_SECS {
            a.tick();
        }
        assert_eq!(a.spinner_elapsed(), Some(ELAPSED_AFTER_SECS));
        // A permission prompt freezes motion (that screen is perfectly still).
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: x".into(),
                detail: "x".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "asks".into(),
            },
        });
        assert!(!a.is_animating(), "no motion during a permission prompt");
        // Answering resumes; TurnEnded stops the spinner.
        a.on_key(KeyEvent::from(KeyCode::Enter)); // deny
        assert!(a.is_animating());
        a.apply_event(UiEvent::TurnEnded);
        assert!(!a.busy);
        assert!(!a.is_animating());
    }

    #[test]
    fn motion_off_disables_animation() {
        let mut a = app();
        a.motion = false;
        a.busy = true;
        assert!(!a.is_animating(), "motion=false is the off switch");
    }

    #[test]
    fn overlay_ease_animates_then_settles() {
        let mut a = app();
        a.open_text_overlay("t", "body");
        // Idle, but the ease-in makes it animate briefly, and start scaled down.
        assert!(a.is_animating());
        assert!(!a.is_working(), "ease-in is not the working spinner");
        assert!(a.overlay_ease_progress() < 1.0);
        for _ in 0..EASE_FRAMES {
            a.tick();
        }
        assert!((a.overlay_ease_progress() - 1.0).abs() < f32::EPSILON);
        assert!(!a.is_animating(), "settles to a still screen when idle");
    }

    #[test]
    fn sidebar_settle_highlights_only_new_entries() {
        let mut a = app();
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 1,
            dels: 0,
        });
        assert!(a.sidebar_settling(), "a new entry settles in");
        for _ in 0..SETTLE_FRAMES {
            a.tick();
        }
        assert!(!a.sidebar_settling());
        // An update to an existing entry does not re-trigger the settle.
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 2,
            dels: 1,
        });
        assert!(
            !a.sidebar_settling(),
            "updates don't settle, only new entries"
        );
    }

    #[test]
    fn motion_off_skips_transient_effects() {
        let mut a = app();
        a.motion = false;
        a.open_text_overlay("t", "body");
        assert!(!a.is_animating());
        assert!(
            (a.overlay_ease_progress() - 1.0).abs() < f32::EPSILON,
            "no ease-in scaling with motion off"
        );
    }

    #[test]
    fn ctrl_j_inserts_a_newline() {
        let mut a = app();
        a.on_key(KeyEvent::from(KeyCode::Char('a')));
        a.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        a.on_key(KeyEvent::from(KeyCode::Char('b')));
        assert_eq!(a.editor.text(), "a\nb");
        assert_eq!(a.editor.line_count(), 2);
    }

    #[test]
    fn shift_tab_cycles_the_auto_accept_mode() {
        let mut a = app();
        // The view starts in Normal; Shift+Tab (BackTab) proposes the next tier.
        // The frontend only proposes — the engine gates it against confinement —
        // so we assert the emitted command, then advance the view as the engine
        // would (ModeChanged) to check the full cycle.
        let step = |a: &mut App, from: emberly_core::Mode, to: emberly_core::Mode| {
            a.mode = from;
            let action = a.on_key(KeyEvent::from(KeyCode::BackTab));
            assert_eq!(action, Action::Command(Command::SetMode { mode: to }));
        };
        step(
            &mut a,
            emberly_core::Mode::Normal,
            emberly_core::Mode::AutoAcceptEdits,
        );
        step(
            &mut a,
            emberly_core::Mode::AutoAcceptEdits,
            emberly_core::Mode::Auto,
        );
        step(&mut a, emberly_core::Mode::Auto, emberly_core::Mode::Normal);
    }

    #[test]
    fn shift_tab_cycles_via_the_cycle_mode_command() {
        // The registry wires `Shift-Tab` to `CycleMode`. `/mode` (slash and
        // palette) is intercepted earlier in `run_slash` to open the picker, so
        // this registry entry now serves only the quick-toggle keybinding.
        assert_eq!(
            crate::commands::by_name("mode"),
            Some(crate::commands::AppCommand::CycleMode)
        );
    }

    #[test]
    fn shift_and_alt_enter_insert_newlines() {
        // Both modifiers newline (where the terminal reports them); plain Enter
        // still submits. Shift is enabled by the kitty protocol at runtime.
        for modifier in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
            let mut a = app();
            a.on_key(KeyEvent::from(KeyCode::Char('a')));
            a.on_key(KeyEvent::new(KeyCode::Enter, modifier));
            a.on_key(KeyEvent::from(KeyCode::Char('b')));
            assert_eq!(a.editor.text(), "a\nb", "{modifier:?}+Enter should newline");
        }
    }

    #[test]
    fn session_usage_is_stored() {
        let mut a = app();
        a.apply_event(UiEvent::SessionUsage {
            usage: TokenUsage {
                input: 1200,
                output: 340,
            },
        });
        assert_eq!(a.session_usage.input, 1200);
        assert_eq!(a.session_usage.output, 340);
    }

    #[test]
    fn wheel_scrolls_conversation_and_routes_to_overlay() {
        let mut a = app();
        a.on_scroll(true); // wheel up → into history
        assert!(a.scroll > 0);
        a.on_scroll(false);
        assert_eq!(a.scroll, 0);
        // With an overlay open, the wheel scrolls the overlay, not the history.
        a.open_text_overlay("t", "x");
        a.on_scroll(false);
        assert_eq!(a.scroll, 0, "conversation untouched while overlay is up");
        assert!(a.active_overlay().is_some_and(|o| o.scroll > 0));
    }

    #[test]
    fn wheel_routes_to_the_open_palette() {
        let mut a = app();
        // Open the palette (Ctrl+P) — it is modal and takes the wheel first.
        a.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(a.palette.is_some());
        // Wheel down moves the selection down, exactly like the Down key…
        a.on_scroll(false);
        assert_eq!(a.palette.as_ref().map(|p| p.selected), Some(1));
        // …and wheel up moves it back, clamped at the top.
        a.on_scroll(true);
        assert_eq!(a.palette.as_ref().map(|p| p.selected), Some(0));
        a.on_scroll(true);
        assert_eq!(a.palette.as_ref().map(|p| p.selected), Some(0));
        // The wheel never leaks to the conversation while the palette is up.
        assert_eq!(a.scroll, 0);
    }

    #[test]
    fn click_on_a_palette_row_is_focus_plus_enter() {
        // A click resolves via the hit-map to a row, then does exactly what
        // "arrow to that row + Enter" does — no separate authority (§3.4).
        let region = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 1,
        };

        let mut by_click = app();
        by_click.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        by_click.hit_map.push(region, ClickTarget::PaletteRow(1));
        let click_action = by_click.on_click(0, 0);

        let mut by_key = app();
        by_key.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        by_key.on_key(KeyEvent::from(KeyCode::Down)); // focus row 1
        let key_action = by_key.on_key(KeyEvent::from(KeyCode::Enter));

        assert_eq!(click_action, key_action, "a click is focus + Enter");
        assert!(
            by_click.palette.is_none(),
            "activating a row closes the palette, like Enter"
        );
        assert!(by_key.palette.is_none());
    }

    #[test]
    fn click_on_a_choice_row_is_focus_plus_enter() {
        // Same parity for the model/effort/mode picker rows.
        let region = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 1,
        };

        let mut by_click = app();
        by_click.open_mode_picker(); // rows: Normal(current) / AutoAcceptEdits / Auto
        by_click.hit_map.push(region, ClickTarget::ChoiceRow(2));
        let click_action = by_click.on_click(0, 0);

        let mut by_key = app();
        by_key.open_mode_picker();
        by_key.on_key(KeyEvent::from(KeyCode::Down));
        by_key.on_key(KeyEvent::from(KeyCode::Down)); // focus row 2
        let key_action = by_key.on_key(KeyEvent::from(KeyCode::Enter));

        assert_eq!(click_action, key_action, "clicking a choice == arrow + Enter");
        assert!(
            by_click.overlays.is_empty(),
            "confirming a choice closes the picker, like Enter"
        );
        assert!(by_key.overlays.is_empty());
    }

    #[test]
    fn a_click_on_nothing_interactive_is_inert() {
        // An empty hit-map (nothing rendered clickable) → no action, no panic.
        let mut a = app();
        assert_eq!(a.on_click(5, 5), Action::None);
    }

    #[test]
    fn overlay_scrolls_and_dismisses() {
        let mut a = app();
        a.open_text_overlay("t", "line1\nline2\nline3");
        a.on_key(KeyEvent::from(KeyCode::PageDown));
        assert!(a.active_overlay().is_some_and(|o| o.scroll > 0));
        // Typing does not leak into the editor while an overlay is modal.
        a.on_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(a.editor.is_empty());
        a.on_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.overlays.is_empty());
    }

    #[test]
    fn modified_files_upsert_by_path() {
        let mut a = app();
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 1,
            dels: 0,
        });
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 3,
            dels: 2,
        });
        assert_eq!(a.modified_files.len(), 1);
        assert_eq!(a.modified_files[0].adds, 3);
        assert_eq!(a.modified_files[0].dels, 2);
    }

    #[test]
    fn permission_defaults_to_deny_on_enter() {
        let mut a = app();
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: rm -rf x".into(),
                detail: "rm -rf x".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "bash asks".into(),
            },
        });
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::PermissionAnswer {
                id: PermissionId(1),
                decision: PermissionDecision::Deny,
            })
        );
        assert!(a.pending_permission.is_none());
    }

    // ---- ask_user question prompt (T-8, Design §5.1) ---------------------

    fn ask(a: &mut App, options: &[&str]) {
        a.apply_event(UiEvent::AskUserRequest {
            id: AskId(7),
            question: "which environment?".into(),
            options: options.iter().map(|s| (*s).to_string()).collect(),
        });
    }

    #[test]
    fn ask_enter_never_auto_answers() {
        let mut a = app();
        ask(&mut a, &["dev", "prod"]);
        // Nothing typed, no option chosen: Enter must not answer (Design §5.1).
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, Action::None);
        assert!(a.pending_ask.is_some(), "the question is still waiting");
    }

    #[test]
    fn ask_esc_declines() {
        let mut a = app();
        ask(&mut a, &["dev", "prod"]);
        let action = a.on_key(KeyEvent::from(KeyCode::Esc));
        assert_eq!(
            action,
            Action::Command(Command::AskUserAnswer {
                id: AskId(7),
                answer: AskAnswer::Declined,
            })
        );
        assert!(a.pending_ask.is_none());
    }

    #[test]
    fn ask_arrow_then_enter_picks_the_selected_option() {
        let mut a = app();
        ask(&mut a, &["dev", "prod"]);
        a.on_key(KeyEvent::from(KeyCode::Down)); // select "dev" (index 0)
        a.on_key(KeyEvent::from(KeyCode::Down)); // select "prod" (index 1)
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::AskUserAnswer {
                id: AskId(7),
                answer: AskAnswer::Answered("prod".into()),
            })
        );
    }

    #[test]
    fn ask_free_text_submits_and_beats_a_selection() {
        let mut a = app();
        ask(&mut a, &["dev", "prod"]);
        a.on_key(KeyEvent::from(KeyCode::Down)); // highlight an option…
        for c in "staging".chars() {
            a.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        // …but typed text wins on Enter.
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::AskUserAnswer {
                id: AskId(7),
                answer: AskAnswer::Answered("staging".into()),
            })
        );
    }

    #[test]
    fn ask_free_text_works_without_options() {
        let mut a = app();
        ask(&mut a, &[]);
        for c in "yes".chars() {
            a.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::AskUserAnswer {
                id: AskId(7),
                answer: AskAnswer::Answered("yes".into()),
            })
        );
    }

    #[test]
    fn ask_prompt_stills_motion() {
        let mut a = app();
        a.busy = true;
        a.motion = true;
        assert!(a.is_working(), "working before the question");
        ask(&mut a, &["dev"]);
        assert!(
            !a.is_working(),
            "no spinner while a question is up (Design §6.4)"
        );
        assert!(!a.is_animating());
    }

    // ---- loop-halt surface (S-5, Design §8.5) ----------------------------

    fn halt(a: &mut App) {
        a.apply_event(UiEvent::LoopHalted {
            reason: "the last few steps repeated without progress".into(),
        });
    }

    #[test]
    fn loop_halt_keep_going_resumes() {
        let mut a = app();
        halt(&mut a);
        let action = a.on_key(KeyEvent::from(KeyCode::Char('g')));
        assert_eq!(
            action,
            Action::Command(Command::ResolveLoop {
                resolution: LoopResolution::Resume
            })
        );
        assert!(a.pending_loop_halt.is_none());
    }

    #[test]
    fn loop_halt_stop_and_esc_both_stop() {
        for key in [KeyCode::Char('s'), KeyCode::Esc] {
            let mut a = app();
            halt(&mut a);
            let action = a.on_key(KeyEvent::from(key));
            assert_eq!(
                action,
                Action::Command(Command::ResolveLoop {
                    resolution: LoopResolution::Stop
                })
            );
        }
    }

    #[test]
    fn loop_halt_say_something_then_steer() {
        let mut a = app();
        halt(&mut a);
        a.on_key(KeyEvent::from(KeyCode::Char('t')));
        assert!(a.pending_loop_halt.as_ref().is_some_and(|h| h.steering));
        for c in "read a.txt".chars() {
            a.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::ResolveLoop {
                resolution: LoopResolution::Steer("read a.txt".into())
            })
        );
    }

    #[test]
    fn loop_halt_steer_esc_returns_to_menu() {
        let mut a = app();
        halt(&mut a);
        a.on_key(KeyEvent::from(KeyCode::Char('t')));
        a.on_key(KeyEvent::from(KeyCode::Char('x')));
        a.on_key(KeyEvent::from(KeyCode::Esc)); // back to menu, not a decision
        assert!(a.pending_loop_halt.as_ref().is_some_and(|h| !h.steering));
        let action = a.on_key(KeyEvent::from(KeyCode::Char('g')));
        assert_eq!(
            action,
            Action::Command(Command::ResolveLoop {
                resolution: LoopResolution::Resume
            })
        );
    }

    #[test]
    fn loop_halt_stills_motion() {
        let mut a = app();
        a.busy = true;
        a.motion = true;
        halt(&mut a);
        assert!(!a.is_working(), "the halt screen is perfectly still");
        assert!(!a.is_animating());
    }

    #[test]
    fn permission_scroll_keys_review_without_deciding() {
        let mut a = app();
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(9),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: x".into(),
                detail: "long\ncommand".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "bash asks".into(),
            },
        });
        // Scrolling and Space page-down must NOT decide.
        a.on_key(KeyEvent::from(KeyCode::Down));
        assert!(a.permission_scroll > 0);
        assert!(a.pending_permission.is_some(), "scroll must not decide");
        a.on_key(KeyEvent::from(KeyCode::Char(' ')));
        assert!(a.pending_permission.is_some());
        // A stray letter is ignored — no accidental decision either way.
        a.on_key(KeyEvent::from(KeyCode::Char('k')));
        assert!(a.pending_permission.is_some());
        // Home returns to the top.
        a.on_key(KeyEvent::from(KeyCode::Home));
        assert_eq!(a.permission_scroll, 0);
    }

    #[test]
    fn wheel_scrolls_a_permission_prompt_without_deciding() {
        let mut a = app();
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(11),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: x".into(),
                detail: "long\ncommand\nbelow\nthe\nfold".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "bash asks".into(),
            },
        });
        // The wheel reviews the prompt body (permission_scroll), never the
        // conversation, and never decides (Design §5, §3.4).
        a.on_scroll(false);
        assert!(a.permission_scroll > 0);
        assert_eq!(a.scroll, 0, "conversation untouched while a prompt is up");
        assert!(a.pending_permission.is_some(), "the wheel never decides");
        a.on_scroll(true);
        assert_eq!(a.permission_scroll, 0);
        assert!(a.pending_permission.is_some());
    }

    #[test]
    fn permission_allows_only_on_deliberate_key() {
        let mut a = app();
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(2),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: ls".into(),
                detail: "ls".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "bash asks".into(),
            },
        });
        let action = a.on_key(KeyEvent::from(KeyCode::Char('y')));
        assert_eq!(
            action,
            Action::Command(Command::PermissionAnswer {
                id: PermissionId(2),
                decision: PermissionDecision::AllowOnce,
            })
        );
    }

    #[test]
    fn enter_submits_user_input() {
        let mut a = app();
        a.on_key(KeyEvent::from(KeyCode::Char('h')));
        a.on_key(KeyEvent::from(KeyCode::Char('i')));
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::UserInput { text: "hi".into() })
        );
        assert!(a.editor.is_empty());
        // The prompt is echoed into the timeline so the pane is a full
        // top-to-bottom transcript of both sides.
        assert_eq!(a.conversation.last(), Some(&ConvItem::User("hi".into())));
    }

    #[test]
    fn slash_command_is_not_echoed_as_a_message() {
        let mut a = app();
        for c in "/help".chars() {
            a.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        a.on_key(KeyEvent::from(KeyCode::Enter));
        // A slash command runs (opens the help overlay); it is not a message.
        assert!(!a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::User(_))));
    }

    fn picker(a: &mut App, rows: Vec<SessionRow>) {
        a.push_overlay(Overlay {
            title: "sessions".into(),
            content: OverlayContent::Sessions { rows, selected: 0 },
            scroll: 0,
        });
    }

    fn row(id: SessionId, current: bool) -> SessionRow {
        SessionRow {
            id,
            title: "t".into(),
            subtitle: "s".into(),
            current,
        }
    }

    #[test]
    fn new_command_returns_action_when_idle_and_is_refused_while_busy() {
        let mut a = app();
        assert_eq!(a.run_command(AppCommand::NewSession), Action::NewSession);
        a.busy = true;
        assert_eq!(a.run_command(AppCommand::NewSession), Action::None);
        assert!(matches!(a.conversation.last(), Some(ConvItem::Notice(_))));
    }

    #[test]
    fn clear_is_an_alias_for_new_session() {
        assert_eq!(commands::by_name("clear"), Some(AppCommand::NewSession));
    }

    #[test]
    fn session_command_opens_the_picker() {
        let mut a = app();
        let _ = a.run_command(AppCommand::Session);
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Sessions { .. })
        ));
    }

    #[test]
    fn picker_enter_on_the_current_session_does_not_resume() {
        let mut a = app();
        let id = a.session.session_id;
        picker(&mut a, vec![row(id, true)]);
        let action = a.on_session_picker_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, Action::None, "cannot resume the current session");
        assert!(a.overlays.is_empty(), "picker dismissed");
    }

    #[test]
    fn picker_enter_on_another_session_resumes_it() {
        let mut a = app();
        let other = SessionId::new();
        picker(&mut a, vec![row(other, false)]);
        let action = a.on_session_picker_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, Action::ResumeSession(other));
        assert!(a.overlays.is_empty());
    }

    #[test]
    fn picker_enter_while_busy_defers_instead_of_switching() {
        let mut a = app();
        a.busy = true;
        picker(&mut a, vec![row(SessionId::new(), false)]);
        assert_eq!(
            a.on_session_picker_key(KeyEvent::from(KeyCode::Enter)),
            Action::None
        );
        assert!(matches!(a.conversation.last(), Some(ConvItem::Notice(_))));
    }

    #[test]
    fn begin_new_session_resets_the_timeline_and_identity() {
        let mut a = app();
        a.conversation.push(ConvItem::User("old".into()));
        a.modified_files.push(ModifiedFile {
            path: "x".into(),
            adds: 1,
            dels: 0,
        });
        let id = SessionId::new();
        a.begin_new_session(id);
        assert_eq!(a.session.session_id, id);
        assert!(a.session.title.is_empty());
        assert!(a.modified_files.is_empty());
        assert!(!a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::User(t) if t == "old")));
    }

    #[test]
    fn begin_resumed_session_adopts_title_and_clears_prior_timeline() {
        let mut a = app();
        a.conversation.push(ConvItem::User("old".into()));
        let id = SessionId::new();
        a.begin_resumed_session(id, "resumed".into(), &[]);
        assert_eq!(a.session.session_id, id);
        assert_eq!(a.session.title, "resumed");
        assert!(!a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::User(t) if t == "old")));
    }

    #[test]
    fn reasoning_delta_builds_a_trail_that_settles_on_the_answer() {
        let mut a = app();
        a.apply_event(UiEvent::ReasoningDelta {
            text: "think ".into(),
        });
        a.apply_event(UiEvent::ReasoningDelta {
            text: "more".into(),
        });
        // While thinking, the trail streams expanded.
        assert!(matches!(
            a.conversation.last(),
            Some(ConvItem::Reasoning { text, expanded: true }) if text == "think more"
        ));
        // The answer begins → the trail settles to collapsed (default view).
        a.apply_event(UiEvent::AssistantDelta {
            text: "answer".into(),
        });
        assert!(matches!(
            a.conversation.first(),
            Some(ConvItem::Reasoning {
                expanded: false,
                ..
            })
        ));
        assert!(matches!(a.conversation.last(), Some(ConvItem::Assistant(t)) if t == "answer"));
    }

    #[test]
    fn hidden_view_drops_the_trail_but_keeps_the_answer() {
        let mut a = app();
        a.reasoning_view = ReasoningView::Hidden;
        a.apply_event(UiEvent::ReasoningDelta {
            text: "secret".into(),
        });
        a.apply_event(UiEvent::AssistantDelta {
            text: "answer".into(),
        });
        assert!(!a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Reasoning { .. })));
        assert!(matches!(a.conversation.last(), Some(ConvItem::Assistant(t)) if t == "answer"));
    }

    #[test]
    fn expanded_view_keeps_the_trail_open_after_the_answer() {
        let mut a = app();
        a.reasoning_view = ReasoningView::Expanded;
        a.apply_event(UiEvent::ReasoningDelta { text: "why".into() });
        a.apply_event(UiEvent::AssistantDelta { text: "a".into() });
        assert!(matches!(
            a.conversation.first(),
            Some(ConvItem::Reasoning { expanded: true, .. })
        ));
    }

    #[test]
    fn ctrl_r_toggles_the_reasoning_trail() {
        let mut a = app();
        a.apply_event(UiEvent::ReasoningDelta { text: "hmm".into() });
        a.apply_event(UiEvent::AssistantDelta { text: "a".into() }); // settle → collapsed
        a.toggle_reasoning();
        assert!(matches!(
            a.conversation.first(),
            Some(ConvItem::Reasoning { expanded: true, .. })
        ));
    }

    #[test]
    fn effort_picker_offers_levels_and_declines_when_none() {
        let mut a = app();
        // No effort control ⇒ a calm notice, no overlay.
        assert_eq!(a.run_slash("effort"), Action::None);
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(m) if m.contains("no reasoning-effort"))));
        assert!(a.overlays.is_empty());

        // With levels available, `/effort` opens the picker.
        a.effort_levels = vec![Effort::Low, Effort::High];
        a.effort = Some(Effort::Low);
        a.run_slash("effort");
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Choices {
                kind: ChoiceKind::Effort,
                ..
            })
        ));
    }

    #[test]
    fn effort_arg_sets_a_supported_level_and_rejects_others() {
        let mut a = app();
        a.effort_levels = vec![Effort::Low, Effort::High];
        assert_eq!(
            a.run_slash("effort high"),
            Action::Command(Command::SetEffort {
                effort: Effort::High
            })
        );
        // A level the model doesn't offer is a notice, not a command.
        assert_eq!(a.run_slash("effort medium"), Action::None);
        assert!(a
            .conversation
            .iter()
            .any(|i| matches!(i, ConvItem::Notice(m) if m.contains("does not offer"))));
    }

    #[test]
    fn effort_changed_updates_sidebar_state() {
        let mut a = app();
        a.apply_event(UiEvent::EffortChanged {
            effort: Some(Effort::High),
            available: vec![Effort::Low, Effort::High],
        });
        assert_eq!(a.effort, Some(Effort::High));
        assert_eq!(a.effort_levels, vec![Effort::Low, Effort::High]);
    }

    #[test]
    fn compact_command_is_reachable_via_slash_and_palette() {
        // The registry entry exists so /compact flows through `by_name` and
        // the palette fuzzy list (Design §3.3, Tech Spec §9).
        assert_eq!(
            crate::commands::by_name("compact"),
            Some(crate::commands::AppCommand::Compact)
        );
        // `/compact` via the slash parser dispatches Command::Compact.
        let mut a = app();
        assert_eq!(a.run_slash("compact"), Action::Command(Command::Compact));
        // The palette entry dispatches the same command.
        assert_eq!(
            a.run_command(crate::commands::AppCommand::Compact),
            Action::Command(Command::Compact)
        );
        // `compact` appears in the command listing (used by /help).
        assert!(crate::commands::COMMANDS
            .iter()
            .any(|c| c.name == "compact"));
    }

    // ---- memory inspector (`/memory`, FR-6, Design §4.9) ------------------

    fn mem_summary(name: &str, desc: &str, scope: MemoryScope) -> EntrySummary {
        EntrySummary {
            name: name.into(),
            description: desc.into(),
            type_: None,
            scope,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn memory_command_requests_the_list() {
        let mut a = app();
        // Reachable three ways (§3.3): slash, palette dispatch, and registry.
        assert_eq!(a.run_slash("memory"), Action::Command(Command::MemoryList));
        assert_eq!(
            a.run_command(AppCommand::Memory),
            Action::Command(Command::MemoryList)
        );
        assert!(commands::COMMANDS.iter().any(|c| c.name == "memory"));
    }

    #[test]
    fn memory_entries_open_grouped_overlay_with_origin() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
            project: vec![mem_summary("Proj", "p", MemoryScope::Project)],
        });
        match a.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::MemoryEntries {
                user,
                project,
                selected,
                confirm_delete,
            }) => {
                assert_eq!(user.len(), 1);
                assert_eq!(project.len(), 1);
                assert_eq!(user[0].scope, MemoryScope::User);
                assert_eq!(project[0].scope, MemoryScope::Project);
                assert_eq!(*selected, 0);
                assert!(!confirm_delete);
            }
            other => panic!("expected MemoryEntries overlay, got {other:?}"),
        }
    }

    #[test]
    fn memory_enter_issues_view_for_the_selected_entry() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![
                mem_summary("Alpha", "first", MemoryScope::User),
                mem_summary("Beta", "second", MemoryScope::User),
            ],
            project: vec![],
        });
        a.on_key(key(KeyCode::Down)); // move to Beta
        assert_eq!(
            a.on_key(key(KeyCode::Enter)),
            Action::Command(Command::MemoryView {
                scope: MemoryScope::User,
                name: "Beta".into(),
            })
        );
    }

    #[test]
    fn memory_delete_requires_explicit_confirmation() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
            project: vec![],
        });
        // A single `d` arms the confirm — it does NOT delete.
        assert_eq!(a.on_key(key(KeyCode::Char('d'))), Action::None);
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::MemoryEntries {
                confirm_delete: true,
                ..
            })
        ));
        // `y` confirms → the delete is issued (harness performs the write).
        assert_eq!(
            a.on_key(key(KeyCode::Char('y'))),
            Action::Command(Command::MemoryMutate {
                op: MemoryOp::Remove,
                scope: MemoryScope::User,
                name: "Alpha".into(),
                description: None,
                type_: None,
                body: None,
            })
        );
    }

    #[test]
    fn memory_delete_is_cancelled_by_any_other_key() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
            project: vec![],
        });
        a.on_key(key(KeyCode::Char('d')));
        // Anything but `y` cancels: no command, confirm cleared, entry intact.
        assert_eq!(a.on_key(key(KeyCode::Char('n'))), Action::None);
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::MemoryEntries {
                confirm_delete: false,
                ..
            })
        ));
    }

    #[test]
    fn memory_edit_stages_a_pending_edit_when_body_arrives() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
            project: vec![],
        });
        // `e` issues a body fetch with the edit intent.
        assert_eq!(
            a.on_key(key(KeyCode::Char('e'))),
            Action::Command(Command::MemoryView {
                scope: MemoryScope::User,
                name: "Alpha".into(),
            })
        );
        // The body reply stages a pending edit for the loop's $EDITOR handoff,
        // carrying the metadata through unchanged (so a body edit never erases
        // the description).
        a.apply_event(UiEvent::MemoryBody {
            scope: MemoryScope::User,
            name: "Alpha".into(),
            body: "the body".into(),
        });
        let edit = a.take_pending_memory_edit().expect("edit staged");
        assert_eq!(edit.name, "Alpha");
        assert_eq!(edit.scope, MemoryScope::User);
        assert_eq!(edit.description, Some("first".into()));
        assert_eq!(edit.body, "the body");
        // An edit does not open a read-only overlay (that's the view path).
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::MemoryEntries { .. })
        ));
    }

    #[test]
    fn memory_view_opens_a_read_only_body_overlay() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "first", MemoryScope::User)],
            project: vec![],
        });
        a.on_key(key(KeyCode::Enter)); // view intent
        a.apply_event(UiEvent::MemoryBody {
            scope: MemoryScope::User,
            name: "Alpha".into(),
            body: "hello body".into(),
        });
        match a.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::Text(body)) => assert!(body.contains("hello body")),
            other => panic!("expected a Text overlay, got {other:?}"),
        }
        // A view never stages an edit.
        assert!(a.take_pending_memory_edit().is_none());
    }

    #[test]
    fn memory_entries_refresh_in_place_and_clamp_selection() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![
                mem_summary("Alpha", "a", MemoryScope::User),
                mem_summary("Beta", "b", MemoryScope::User),
            ],
            project: vec![],
        });
        a.on_key(key(KeyCode::Down)); // select Beta (index 1)
        // A re-list with fewer entries reuses the overlay and clamps selection.
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "a", MemoryScope::User)],
            project: vec![],
        });
        assert_eq!(a.overlays.len(), 1);
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::MemoryEntries { selected: 0, .. })
        ));
    }

    #[test]
    fn memory_inspector_esc_dismisses() {
        let mut a = app();
        a.apply_event(UiEvent::MemoryEntries {
            user: vec![mem_summary("Alpha", "a", MemoryScope::User)],
            project: vec![],
        });
        assert_eq!(a.on_key(key(KeyCode::Esc)), Action::None);
        assert!(a.overlays.is_empty());
    }

    // ---- skills inspector (`/skills`, FR-7, Design §4.9) ------------------

    fn skill_meta(name: &str, desc: &str, origin: SkillOrigin) -> SkillMeta {
        SkillMeta {
            name: name.into(),
            description: desc.into(),
            origin,
        }
    }

    #[test]
    fn skills_command_opens_inspector_from_cached_catalog() {
        let mut a = app();
        a.skills = vec![
            skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User),
            skill_meta("linter", "Run linters", SkillOrigin::Project),
        ];
        // No engine round-trip — the catalog is already cached, so the overlay
        // opens immediately.
        assert_eq!(a.run_slash("skills"), Action::None);
        match a.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::SkillList { skills, selected }) => {
                assert_eq!(skills.len(), 2);
                assert_eq!(skills[0].origin, SkillOrigin::User);
                assert_eq!(skills[1].origin, SkillOrigin::Project);
                assert_eq!(*selected, 0);
            }
            other => panic!("expected SkillList overlay, got {other:?}"),
        }
        // Reachable three ways (§3.3): palette dispatch and the registry too.
        assert_eq!(a.run_command(AppCommand::Skills), Action::None);
        assert!(commands::COMMANDS.iter().any(|c| c.name == "skills"));
    }

    #[test]
    fn skills_enter_issues_inspect_for_the_selected_skill() {
        let mut a = app();
        a.skills = vec![
            skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User),
            skill_meta("linter", "Run linters", SkillOrigin::Project),
        ];
        a.run_command(AppCommand::Skills);
        a.on_key(key(KeyCode::Down)); // select linter
        assert_eq!(
            a.on_key(key(KeyCode::Enter)),
            Action::Command(Command::InspectSkill {
                name: "linter".into(),
            })
        );
    }

    #[test]
    fn skill_body_opens_a_read_only_overlay_with_resources() {
        let mut a = app();
        a.skills = vec![skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User)];
        a.run_command(AppCommand::Skills);
        a.on_key(key(KeyCode::Enter));
        a.apply_event(UiEvent::SkillBody {
            name: "pdf-fill".into(),
            origin: SkillOrigin::User,
            body: "Step 1: open the template.".into(),
            resources: vec!["/abs/template.txt".into()],
        });
        match a.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::Text(body)) => {
                assert!(body.contains("Step 1: open the template."));
                assert!(body.contains("template.txt"), "bundled resource listed");
            }
            other => panic!("expected a Text overlay, got {other:?}"),
        }
    }

    #[test]
    fn skills_inspector_has_no_edit_or_delete() {
        let mut a = app();
        a.skills = vec![skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User)];
        a.run_command(AppCommand::Skills);
        // `e`/`d` are inert for skills (read-only) — no command, overlay stays.
        assert_eq!(a.on_key(key(KeyCode::Char('e'))), Action::None);
        assert_eq!(a.on_key(key(KeyCode::Char('d'))), Action::None);
        assert!(matches!(
            a.overlays.last().map(|o| &o.content),
            Some(OverlayContent::SkillList { .. })
        ));
    }

    #[test]
    fn skills_empty_catalog_opens_an_empty_overlay() {
        let mut a = app();
        a.skills.clear();
        a.run_command(AppCommand::Skills);
        match a.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::SkillList { skills, .. }) => assert!(skills.is_empty()),
            other => panic!("expected an empty SkillList overlay, got {other:?}"),
        }
    }

    #[test]
    fn skills_inspector_esc_dismisses() {
        let mut a = app();
        a.skills = vec![skill_meta("pdf-fill", "Fill PDF forms", SkillOrigin::User)];
        a.run_command(AppCommand::Skills);
        assert_eq!(a.on_key(key(KeyCode::Esc)), Action::None);
        assert!(a.overlays.is_empty());
    }
}
