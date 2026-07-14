//! The single command registry (Design §3.3, Tech Spec §9). One source of
//! truth — name, keybinding, one-line description, action — that serves all
//! three ways of reaching functionality: the Ctrl+P palette, `/command`
//! parsing, and `/help`. New-user help teaches the palette first.
//!
//! Keeping this a plain table means adding a command is one entry here plus a
//! match arm in [`crate::app::App::run_command`]; the palette, slash parser,
//! and help list update automatically.

/// A command's identity — the action the app performs when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    /// List commands and keybindings.
    Help,
    /// View the last assistant message in full.
    View,
    /// Open the latest file's diff overlay.
    Diff,
    /// List files changed this session.
    Files,
    /// List saved sessions and switch to one.
    Session,
    /// Start a fresh session in place, saving the current one.
    NewSession,
    /// Toggle the sidebar.
    ToggleSidebar,
    /// Cycle the auto-accept mode (normal → auto-accept-edits → auto → …).
    /// The engine gates the auto tiers on active OS confinement and explains
    /// when it refuses (Requirements §6.4).
    CycleMode,
    /// Open the model/provider picker (C-6). `/model <profile>` switches
    /// directly; with no argument (or from the palette) it opens the picker.
    Model,
    /// Open the reasoning-effort picker (P-9). `/effort <level>` sets it
    /// directly; with no argument (or from the palette) it opens the picker.
    Effort,
    /// Edit the project `.agents/config.toml` in `$EDITOR` (C-5).
    Config,
    /// Edit a prompt file in `$EDITOR` (C-5). `/prompt [system|compact]`.
    Prompt,
    /// Re-read config + prompts from disk and apply them (C-5) — useful after
    /// editing a file outside emberly.
    Reload,
    /// Inspect, edit, and delete stored memory (FR-6, Design §4.9). Opens the
    /// memory inspector overlay grouped by scope.
    Memory,
    /// Manually compact the conversation — summarize older turns into a
    /// summary at a clean boundary (Requirements §8.3, Tech Spec §7).
    Compact,
    /// Cancel the in-flight turn.
    Cancel,
    /// Exit emberly.
    Quit,
}

/// A registry entry.
pub struct CommandSpec {
    /// The `/name` used to invoke it, and the palette label.
    pub name: &'static str,
    /// The keybinding hint, if it has one.
    pub key: Option<&'static str>,
    pub desc: &'static str,
    pub cmd: AppCommand,
}

/// The v1 command set. Order is the palette's default (most useful first).
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        key: Some("Ctrl-P"),
        desc: "List commands and keybindings",
        cmd: AppCommand::Help,
    },
    CommandSpec {
        name: "view",
        key: None,
        desc: "View the last assistant message in full",
        cmd: AppCommand::View,
    },
    CommandSpec {
        name: "diff",
        key: Some("Ctrl-O"),
        desc: "Open the latest file diff",
        cmd: AppCommand::Diff,
    },
    CommandSpec {
        name: "files",
        key: None,
        desc: "List files changed this session",
        cmd: AppCommand::Files,
    },
    CommandSpec {
        name: "session",
        key: None,
        desc: "List saved sessions and switch to one",
        cmd: AppCommand::Session,
    },
    CommandSpec {
        name: "new",
        key: None,
        desc: "Start a fresh session (saves the current one)",
        cmd: AppCommand::NewSession,
    },
    CommandSpec {
        name: "mode",
        key: Some("Shift-Tab"),
        desc: "Cycle permission mode (normal / auto-accept edits / auto)",
        cmd: AppCommand::CycleMode,
    },
    CommandSpec {
        name: "model",
        key: None,
        desc: "Switch the active provider/model",
        cmd: AppCommand::Model,
    },
    CommandSpec {
        name: "effort",
        key: None,
        desc: "Set the reasoning-effort level (/effort low|medium|high|max)",
        cmd: AppCommand::Effort,
    },
    CommandSpec {
        name: "config",
        key: None,
        desc: "Edit .agents/config.toml in $EDITOR",
        cmd: AppCommand::Config,
    },
    CommandSpec {
        name: "prompt",
        key: None,
        desc: "Edit a prompt file in $EDITOR (/prompt system|compact)",
        cmd: AppCommand::Prompt,
    },
    CommandSpec {
        name: "reload",
        key: None,
        desc: "Re-read config & prompts from disk and apply them",
        cmd: AppCommand::Reload,
    },
    CommandSpec {
        name: "memory",
        key: None,
        desc: "Inspect, edit, and delete stored memory",
        cmd: AppCommand::Memory,
    },
    CommandSpec {
        name: "compact",
        key: None,
        desc: "Summarize older turns to reclaim context space",
        cmd: AppCommand::Compact,
    },
    CommandSpec {
        name: "sidebar",
        key: Some("Ctrl-B"),
        desc: "Toggle the sidebar",
        cmd: AppCommand::ToggleSidebar,
    },
    CommandSpec {
        name: "cancel",
        key: None,
        desc: "Cancel the current turn",
        cmd: AppCommand::Cancel,
    },
    CommandSpec {
        name: "quit",
        key: Some("Ctrl-D"),
        desc: "Exit emberly",
        cmd: AppCommand::Quit,
    },
];

/// Look up a command by exact `/name` (the slash already stripped). `/clear` is
/// an alias for `/new` — both start a fresh session (the user's "New or Clear").
#[must_use]
pub fn by_name(name: &str) -> Option<AppCommand> {
    if name == "clear" {
        return Some(AppCommand::NewSession);
    }
    COMMANDS.iter().find(|c| c.name == name).map(|c| c.cmd)
}

/// Indices into [`COMMANDS`] matching `query`, best score first. An empty query
/// returns every command in registry order.
#[must_use]
pub fn matches(query: &str) -> Vec<usize> {
    let q = query.trim();
    if q.is_empty() {
        return (0..COMMANDS.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, c)| fuzzy_score(q, c.name).map(|s| (s, i)))
        .collect();
    // Higher score first; ties keep registry order (stable sort on index).
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// A small fuzzy subsequence score (case-insensitive). `None` if `query` is not
/// a subsequence of `candidate`. Contiguous runs and early matches score
/// higher — plenty for a handful of short command names.
fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let cand = candidate.to_ascii_lowercase();
    let mut cand_chars = cand.char_indices();
    let mut score = 0i32;
    let mut streak = 0i32;
    for qc in query.to_ascii_lowercase().chars() {
        loop {
            let (idx, cc) = cand_chars.next()?;
            if cc == qc {
                // Reward contiguity and matching at the start of the word.
                score += 10 + streak * 5 - i32::try_from(idx).unwrap_or(0).min(10);
                streak += 1;
                break;
            }
            streak = 0;
        }
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_name_resolves_slash_commands() {
        assert_eq!(by_name("help"), Some(AppCommand::Help));
        assert_eq!(by_name("quit"), Some(AppCommand::Quit));
        assert_eq!(by_name("nope"), None);
    }

    #[test]
    fn empty_query_lists_all_in_order() {
        assert_eq!(matches(""), (0..COMMANDS.len()).collect::<Vec<_>>());
    }

    #[test]
    fn fuzzy_filters_and_ranks() {
        // "he" matches "help" only.
        let m = matches("he");
        assert_eq!(m.len(), 1);
        assert_eq!(COMMANDS[m[0]].name, "help");
        // A prefix ranks its command first.
        let m = matches("sid");
        assert_eq!(COMMANDS[m[0]].name, "sidebar");
    }

    #[test]
    fn non_subsequence_is_filtered_out() {
        // No command name contains 'z'.
        assert!(matches("zzz").is_empty());
    }
}
