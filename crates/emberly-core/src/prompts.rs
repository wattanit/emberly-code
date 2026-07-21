//! The baked-in default prompts (Requirements C-1). Authored as markdown files
//! under `crates/emberly-core/prompts/` and embedded at build time via
//! `include_str!` — so they read and diff as prose, live in one place, and
//! still carry zero runtime dependency (the default always works; no file is
//! needed at run time — HC-2/portability preserved).
//!
//! They are **versioned independently of the crate** ([`VERSION`], with
//! `prompts/CHANGELOG.md`). The version is stamped into each session's
//! `session_start` record, so a transcript records which prompt set produced
//! it. Projects override any prompt under `.agents/prompts/<name>[.<family>].md`
//! (P-7); this module is only the fallback default.

/// The default prompt-set version. Bump on any prompt change and add a
/// `prompts/CHANGELOG.md` entry. Recorded in `session_start` (Tech Spec §3.2).
pub const VERSION: u32 = 5;

/// The default agent system prompt.
pub const SYSTEM: &str = include_str!("../prompts/system.md");

/// The default `/compact` summarization prompt (Tech Spec §7).
pub const COMPACT: &str = include_str!("../prompts/compact.md");

/// The tool-call explanation instruction (T-9, Tech Spec §5.4), appended to the
/// system prompt **only** when `ui.tool_explanations` is on. Kept out of
/// `system.md` so that, when the feature is off, neither this text nor the
/// schema `explanation` property is sent and no tokens are spent (Requirements
/// T-9).
pub const TOOL_EXPLANATION: &str = include_str!("../prompts/tool_explanation.md");

/// The system prompt, trimmed of trailing file whitespace.
#[must_use]
pub fn system() -> &'static str {
    SYSTEM.trim()
}

/// The tool-call explanation instruction, trimmed (T-9).
#[must_use]
pub fn tool_explanation() -> &'static str {
    TOOL_EXPLANATION.trim()
}

/// The compaction prompt, trimmed of trailing file whitespace.
#[must_use]
pub fn compact() -> &'static str {
    COMPACT.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_embedded_and_nonempty() {
        assert!(system().contains("Emberly Code"));
        assert!(compact().contains("compact"));
        assert!(tool_explanation().contains("explanation"));
        assert_eq!(VERSION, 5);
    }
}
