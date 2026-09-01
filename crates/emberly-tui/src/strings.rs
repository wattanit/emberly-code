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
    /// Provenance line when the action is a subagent's own, not the primary
    /// agent's (FR-9, Design §4.13/§5) — every other guarantee on this prompt
    /// holds unchanged; this is the only addition.
    pub const ON_BEHALF_OF_LABEL: &str = "on behalf of subagent";
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

/// The loop-halt surface (S-5, Design §8.5). The **harness** stepping in — its
/// own out-of-band voice, calm, no blame, no alarm styling. Distinct from the
/// question prompt (that is the model asking). Shared by both frontends.
pub mod loop_halt {
    pub const TITLE: &str = "stopped";
    /// The calm one-liner (Design §8.5). The specific reason follows on its line.
    pub const HEADING: &str = "Stopped — the last few steps repeated without progress.";
    pub const KEEP_GOING: &str = "keep going";
    pub const STOP: &str = "stop here";
    pub const SAY_SOMETHING: &str = "say something";
    pub const HINT: &str = "g keep going · s stop · t say something";
    pub const STEER_HINT: &str = "type a steer · Enter send · Esc back";
    pub const STEER_LABEL: &str = "your steer";
    /// Degraded-mode prompt line.
    pub const LINE_PROMPT: &str = "[keep] keep going · [stop] stop · or type a message to steer";
}

/// The completion-gate halt surface (S-6, Design §8.7). The **harness**
/// stepping in after a bounded number of failed completion attempts — its own
/// out-of-band voice, calm, no blame, no alarm styling. Distinct from a
/// failing check's agent-world tool-result (that is ordinary tool output the
/// model reacts to; this is the harness stepping in once the model has had
/// its bounded chances). Shared by both frontends.
pub mod completion_gate {
    pub const TITLE: &str = "stopped";
    /// The calm one-liner (Design §8.7). The failing checks and attempt count
    /// follow on their own lines.
    pub const HEADING: &str = "Stopped — the completion checks still fail.";
    pub const KEEP_GOING: &str = "keep going";
    pub const STOP: &str = "stop here";
    pub const SAY_SOMETHING: &str = "say something";
    /// Labeled as an override (Design §8.7) — never "checks passed".
    pub const FINISH_ANYWAY: &str = "finish anyway (override)";
    pub const HINT: &str = "g keep going · s stop · t say something · f finish anyway";
    pub const STEER_HINT: &str = "type a steer · Enter send · Esc back";
    pub const STEER_LABEL: &str = "your steer";
    /// Degraded-mode prompt line.
    pub const LINE_PROMPT: &str =
        "[keep] keep going · [stop] stop · [finish] finish anyway · or type a message to steer";
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
    /// The completion-gate sidebar status line's label (S-6, Design §8.7).
    pub const GATE_LABEL: &str = "gate";
    pub const SANDBOX_UNKNOWN: &str = "—";
    pub const MODIFIED_FILES_TITLE: &str = "modified files";
    pub const MODEL_LABEL: &str = "model";
    pub const EFFORT_LABEL: &str = "effort";
    pub const ROOT_LABEL: &str = "root";
    /// Session-start cue when no provider is configured (Requirements C-7,
    /// Design §8.2): points straight at the guided setup fix, in both the
    /// rich sidebar and the degraded-mode banner (same wording, matching
    /// "one reload story for every path").
    pub const NO_PROVIDER_CUE: &str = "none configured — /model to add a provider";
}

/// The guided provider/model setup wizard (Requirements C-7, Design §4.6).
/// Name comes first — the identifier the user is actually choosing — then
/// the adapter (wire format, not brand), endpoint, model id, and key.
pub mod provider_wizard {
    pub const TITLE: &str = "add a provider";
    pub const NAME_LABEL: &str = "profile name";
    pub const ADAPTER_LABEL: &str = "adapter";
    /// The wire-format-not-brand explainer (Design §4.6).
    pub const ADAPTER_EXPLAINER: &str =
        "wire format, not brand — most third-party and OpenAI-compatible APIs, including local models, use \"openai\"";
    pub const ENDPOINT_LABEL: &str = "endpoint";
    pub const MODEL_ID_LABEL: &str = "model id";
    pub const API_KEY_LABEL: &str = "API key";
    pub const SUMMARY_NAME_LABEL: &str = "name";
    pub const SUMMARY_KEY_LABEL: &str = "key";
    pub const HINT: &str = "Enter next · Esc back";
    pub const SUMMARY_HINT: &str = "Enter to save · Esc back";
}

/// Mode names for display (Requirements §6.4). Lower-case, terse.
pub mod mode {
    pub const NORMAL: &str = "normal";
    pub const AUTO_ACCEPT_EDITS: &str = "auto-accept-edits";
    pub const AUTO: &str = "auto";
}

/// Memory inspector (`/memory`, FR-6, Design §4.9). Origin (scope) is how the
/// user reads trust, so it labels every group.
pub mod memory {
    pub const TITLE: &str = "memory";
    pub const SCOPE_USER: &str = "user";
    pub const SCOPE_PROJECT: &str = "project";
    /// Group headers (dimmed chrome, not accent — §2).
    pub const HEADER_USER: &str = "user memory";
    pub const HEADER_PROJECT: &str = "project memory";
    /// Shown when neither scope has any entries.
    pub const EMPTY: &str = "(no stored memory yet)";
    /// Shown when a viewed entry has an empty body.
    pub const EMPTY_BODY: &str = "(this entry has an empty body)";
    /// The action hint at the foot of the inspector.
    pub const HINT: &str = " Enter view · e edit · d delete · Esc close";
    /// Confirm-delete prompt (the entry name is inserted between the two).
    pub const CONFIRM_PREFIX: &str = " delete ";
    pub const CONFIRM_SUFFIX: &str = "?  y confirm · any other key cancels";
}

/// Skills inspector (`/skills`, FR-7, Design §4.9). Read-only: skills are
/// externally-authored folders; the inspector shows what a skill could tell the
/// model to do before it ever runs. Origin is how the user reads trust.
pub mod skills {
    pub const TITLE: &str = "skills";
    pub const ORIGIN_USER: &str = "user";
    pub const ORIGIN_PROJECT: &str = "project";
    /// Shown when no skills are available.
    pub const EMPTY: &str = "(no skills available)";
    /// Shown when a skill's instruction body is empty.
    pub const EMPTY_BODY: &str = "(this skill has an empty instruction body)";
    /// Header before the bundled-resource list in the body view.
    pub const RESOURCES_HEADER: &str = "bundled files:";
    /// The action hint at the foot of the inspector.
    pub const HINT: &str = " Enter view · ↑↓ move · Esc close";
}

/// The Agents inspector (FR-9, Design §4.13) — mirrors `skills`'s shape: a
/// selectable list, Enter opens a read-only body on top (a subagent's own
/// activity, not an instruction to inspect before it runs, but the same
/// "inspectors, not black boxes" pattern).
pub mod agents {
    pub const TITLE: &str = "agents";
    /// Shown when no subagents are currently alive.
    pub const EMPTY: &str = "(no subagents are currently alive)";
    /// The action hint at the foot of the inspector.
    pub const HINT: &str = " Enter view activity · ↑↓ move · Esc close";
    /// The foot hint on the activity overlay itself — names the live refresh
    /// (Design §4.13) so the text changing under the user's eyes reads as
    /// expected, not as a glitch.
    pub const ACTIVITY_HINT: &str = " Esc close · ↑↓ PgUp/PgDn scroll · updates live";
}

/// The MCP inspector (`/mcp`, FR-11, Design §4.15).
pub mod mcp {
    pub const TITLE: &str = "mcp";
    /// Shown when no servers are currently connected.
    pub const EMPTY: &str = "(no MCP servers are currently connected)";
    /// The action hint at the foot of the inspector.
    pub const HINT: &str = " Enter view tools · ↑↓ move · Esc close";
}

/// Session export (FR-12, Design §8.11): the calm, one-time disclosure line
/// shared verbatim by both frontends, since export does not redact content
/// (Requirements FR-12 honesty clause) — the disclosure is the mitigation.
pub mod export {
    pub const SENSITIVE_CONTENT_NOTE: &str = "This file may contain file contents, command output, and anything else this session touched — review before sharing.";
}

/// Keybinding hints for the status bar (Design §3.1, §6.2 — 2–5 words each).
/// Hints change with state; the permission set is shown while a prompt is open.
pub mod hints {
    pub const NORMAL: &str = "Enter send · Alt+Enter newline · Ctrl-P commands · Ctrl-D quit";
    pub const PERMISSION: &str = "y allow · s session · Enter deny";
}

/// The one-time orientation shown in an otherwise-empty conversation pane at
/// the start of a fresh session (never a resumed one) — the blank pane's own
/// answer to "how do I do anything here" (owner's call: shown every fresh
/// session, not just an uninitialized project — see [`crate::app::App::seed_history`]
/// and `begin_new_session`). Rendered as a `ConvItem::Notice`, so it carries
/// the same terse, dim harness voice as every other notice — a pointer, not a
/// tutorial.
pub mod welcome {
    pub const TEXT: &str = "Ctrl-P opens the command palette — a few commands worth knowing:\n  /init      scaffold .agents/ for this project\n  /model     pick a provider and model\n  /compact   reclaim context by summarizing older turns\n  /help      the full command list";
}

/// Conversation-flow markers. ASCII-safe fallbacks live in the line frontend;
/// these are the rich-mode glyphs.
pub mod markers {
    pub const USER_PROMPT: &str = "›";
    /// Leads a user-attached image's chip on the sent message (FR-10, Design
    /// §4.14) — deliberately distinct from `NOTICE`/tool-activity styling,
    /// since this is content on the user's own message, not tool activity.
    pub const ATTACHMENT: &str = "📎";
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
    /// Task-list status glyphs (T-11, Design §4.7). Rich-mode symbols; the
    /// plain frontend uses ASCII `[ ]`/`[~]`/`[x]` fallbacks (Design §7).
    pub const TASK_PENDING: &str = "○";
    pub const TASK_IN_PROGRESS: &str = "◐";
    pub const TASK_DONE: &str = "✓";
    /// Leads the label marking untrusted web-content results (T-14, Design
    /// §4.10) so the block reads visibly as fetched web data, not tool output.
    pub const WEB: &str = "↩";
}
