//! The interface string table (Design §6.2). Every user-facing string lives
//! here rather than scattered as literals across the widgets. This is not a
//! localization framework — interface text is English in v1 — it is the cheap
//! discipline that keeps the voice consistent (second person, present tense,
//! terse chrome, no exclamation marks) and leaves the door open to localization
//! later without committing to it now.
//!
//! Content — the conversation, model output, file text, diffs — is never
//! routed through here; only *chrome* is. The permission-critical strings are
//! shared by both the rich TUI and the line-mode frontend so their safety
//! guarantees read identically (Design §5, §7).

/// Product identity (Design §1.1). The wordmark is `emberly` (accent) + `code`
/// (dimmed) — assembled with theme roles at the call site.
pub mod brand {
    pub const NAME: &str = "emberly";
    pub const SUFFIX: &str = "code";
}

/// The permission prompt (Design §5). Shared verbatim by both frontends.
pub mod permission {
    /// Loud, capitalised banner for actions touching outside the project root
    /// (HC-4). Carries the meaning that the reserved colour band reinforces, so
    /// it stands alone in degraded mode (Design §7).
    pub const OUTSIDE_ROOT_BANNER: &str = "THIS ACTION AFFECTS FILES OUTSIDE YOUR PROJECT";
    pub const TITLE: &str = "permission";
    pub const HEADING: &str = "PERMISSION REQUIRED";
    pub const WHY_LABEL: &str = "why";
    pub const PATHS_LABEL: &str = "paths";
    /// The choices. Deny is the safe default and the meaning of Enter/Esc.
    pub const ALLOW_ONCE: &str = "allow once";
    pub const ALLOW_SESSION: &str = "allow this session";
    pub const DENY: &str = "DENY";
    /// Shown when a long diff/command has more content below the fold; approval
    /// should require having scrolled to the end (Design §5).
    pub const MORE_BELOW: &str = "more below — scroll to review before allowing";
}

/// The `ask_user` question prompt (T-8, Design §5.1). Deliberately its **own**
/// module, not shared with `permission`: this prompt is calm and neutral, has
/// no safety vocabulary, and never uses the reserved safety band. Shared
/// verbatim by both frontends for parity.
pub mod ask_user {
    pub const TITLE: &str = "question";
    /// Leads the free-text answer field.
    pub const ANSWER_LABEL: &str = "your answer";
    /// Status/footer hint. No "safe default" language — Esc declines, and Enter
    /// never auto-answers (Design §5.1).
    pub const HINT: &str = "type answer · ↑↓ choose · Enter send · Esc decline";
    /// Degraded-mode heading + decline hint (line frontend).
    pub const HEADING: &str = "QUESTION";
    pub const DECLINE_HINT: &str = "[Enter] send · empty = declined";
}

/// Status-bar and sidebar chrome (Design §3).
pub mod status {
    pub const CONTEXT_ABBR: &str = "ctx";
    pub const COST_ESTIMATE_SUFFIX: &str = "est.";
    /// Cumulative session token total (input + output).
    pub const TOKENS_LABEL: &str = "tokens";
    /// Compact input/output markers for the token breakdown.
    pub const TOKENS_IN: &str = "↑";
    pub const TOKENS_OUT: &str = "↓";
    pub const SANDBOX_LABEL: &str = "sandbox";
    pub const SANDBOX_UNKNOWN: &str = "—";
    pub const MODIFIED_FILES_TITLE: &str = "modified files";
    pub const MODEL_LABEL: &str = "model";
    pub const EFFORT_LABEL: &str = "effort";
    pub const ROOT_LABEL: &str = "root";
}

/// Mode names for display (Requirements §6.4). Lower-case, terse.
pub mod mode {
    pub const NORMAL: &str = "normal";
    pub const AUTO_ACCEPT_EDITS: &str = "auto-accept-edits";
    pub const AUTO: &str = "auto";
}

/// Keybinding hints for the status bar (Design §3.1, §6.2 — 2–5 words each).
/// Hints change with state; the permission set is shown while a prompt is open.
pub mod hints {
    pub const NORMAL: &str = "Enter send · Alt+Enter newline · Ctrl-P commands · Ctrl-D quit";
    pub const PERMISSION: &str = "y allow · s session · Enter deny";
}

/// Conversation-flow markers. ASCII-safe fallbacks live in the line frontend;
/// these are the rich-mode glyphs.
pub mod markers {
    pub const USER_PROMPT: &str = "›";
    pub const NOTICE: &str = "·";
    pub const RUNNING: &str = "…";
    pub const OK: &str = "ok";
    pub const FAILED: &str = "FAILED";
    /// Reasoning trail: collapsed (expandable) vs expanded (Design §4.4).
    pub const REASONING_COLLAPSED: &str = "▸";
    pub const REASONING_EXPANDED: &str = "▾";
    /// Leads the dim tool-call explanation caption (T-9, Design §4.5) so it
    /// reads as an annotation of the call above, not as tool output.
    pub const EXPLANATION: &str = "↳";
}

/// One-line orientation shown on clean exit (Design §8.3). The richer summary
/// (name, duration, cost, transcript path) arrives with Phase 5 persistence.
pub const SESSION_ENDED: &str = "session ended.";
