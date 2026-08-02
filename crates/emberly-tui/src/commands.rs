//! The single command registry (Design §3.3, Tech Spec §9). One source of
//! truth — name, keybinding, one-line description, action, and whether line
//! mode can run it — that serves every way of reaching functionality: the
//! Ctrl+P palette, `/command` parsing in **both** frontends, and `/help`.
//! New-user help teaches the palette first.
//!
//! Keeping this a plain table means adding a command is one entry here plus a
//! match arm in [`crate::app::App::run_command`]; the palette, both slash
//! parsers, and both help lists update automatically.
//!
//! Both frontends resolve a typed `/name` through [`parse_slash`], so which
//! names exist and where a name ends and its arguments begin is decided here
//! and nowhere else. Each command also declares a [`Plain`] behaviour, so line
//! mode has a defined, testable answer for every name — rather than omitting
//! the ones it cannot render and passing the unrecognized `/name` to the model
//! as a prompt (Design §7 degradation).

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
    /// List available skills and inspect a skill's instructions read-only
    /// (FR-7, Design §4.9) — "what could this skill tell the model to do" is
    /// inspectable before it ever runs.
    Skills,
    /// Manually compact the conversation — summarize older turns into a
    /// summary at a clean boundary (Requirements §8.3, Tech Spec §7).
    Compact,
    /// Cancel the in-flight turn.
    Cancel,
    /// Exit emberly.
    Quit,
}

/// What line mode does with a command (Design §7 degradation). Declared per
/// entry, beside the name, so a difference between the frontends is a stated
/// fact in the registry rather than something implied by an omission in a
/// parser — which is how line mode came to silently swallow commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plain {
    /// Runs, and [`CommandSpec::desc`] describes it accurately.
    Same,
    /// Runs, but does something different enough to need its own description —
    /// degraded mode has no `$EDITOR` and no keybindings.
    Differs(&'static str),
    /// Needs an overlay only the rich TUI has. Line mode says so rather than
    /// ignoring the line.
    Unavailable,
}

/// A registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// The `/name` used to invoke it, and the palette label.
    pub name: &'static str,
    /// The keybinding hint, if it has one.
    pub key: Option<&'static str>,
    pub desc: &'static str,
    pub cmd: AppCommand,
    /// This command in line mode (Design §7).
    pub plain: Plain,
}

impl CommandSpec {
    /// How line mode describes this command, or `None` when line mode cannot
    /// run it at all.
    #[must_use]
    pub fn plain_desc(&self) -> Option<&'static str> {
        match self.plain {
            Plain::Same => Some(self.desc),
            Plain::Differs(desc) => Some(desc),
            Plain::Unavailable => None,
        }
    }

    /// Whether line mode can run this command.
    #[must_use]
    pub fn runs_plain(&self) -> bool {
        self.plain_desc().is_some()
    }
}

/// The v1 command set. Order is the palette's default — grouped by frequency
/// of use (owner's call): session start/switch first, then the runtime
/// switches reached for constantly mid-session, then inspection/control,
/// then setup/tuning (reached for rarely, once things are configured), with
/// `help`/`quit` last.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "new",
        plain: Plain::Same,
        key: None,
        desc: "Start a fresh session (saves the current one)",
        cmd: AppCommand::NewSession,
    },
    CommandSpec {
        name: "session",
        plain: Plain::Unavailable,
        key: None,
        desc: "List saved sessions and switch to one",
        cmd: AppCommand::Session,
    },
    CommandSpec {
        name: "compact",
        plain: Plain::Same,
        key: None,
        desc: "Summarize older turns to reclaim context space",
        cmd: AppCommand::Compact,
    },
    CommandSpec {
        name: "model",
        plain: Plain::Differs("Switch the active provider/model (/model <profile> [model])"),
        key: None,
        desc: "Switch the active provider/model",
        cmd: AppCommand::Model,
    },
    CommandSpec {
        name: "mode",
        plain: Plain::Same,
        key: Some("Shift-Tab"),
        desc: "Cycle permission mode (normal / auto-accept edits / auto)",
        cmd: AppCommand::CycleMode,
    },
    CommandSpec {
        name: "effort",
        plain: Plain::Differs("Set the reasoning-effort level (/effort <low|medium|high|max>)"),
        key: None,
        desc: "Set the reasoning-effort level (/effort low|medium|high|max)",
        cmd: AppCommand::Effort,
    },
    CommandSpec {
        name: "view",
        plain: Plain::Unavailable,
        key: None,
        desc: "View the last assistant message in full",
        cmd: AppCommand::View,
    },
    CommandSpec {
        name: "diff",
        plain: Plain::Unavailable,
        key: Some("Ctrl-O"),
        desc: "Open the latest file diff",
        cmd: AppCommand::Diff,
    },
    CommandSpec {
        name: "files",
        plain: Plain::Unavailable,
        key: None,
        desc: "List files changed this session",
        cmd: AppCommand::Files,
    },
    CommandSpec {
        name: "sidebar",
        plain: Plain::Unavailable,
        key: Some("Ctrl-B"),
        desc: "Toggle the sidebar",
        cmd: AppCommand::ToggleSidebar,
    },
    CommandSpec {
        name: "cancel",
        plain: Plain::Same,
        key: None,
        desc: "Cancel the current turn",
        cmd: AppCommand::Cancel,
    },
    CommandSpec {
        name: "config",
        plain: Plain::Differs("Print the .agents/config.toml path to edit, then /reload"),
        key: None,
        desc: "Edit .agents/config.toml in $EDITOR",
        cmd: AppCommand::Config,
    },
    CommandSpec {
        name: "prompt",
        plain: Plain::Differs("Print a prompt file path to edit (/prompt system|compact)"),
        key: None,
        desc: "Edit a prompt file in $EDITOR (/prompt system|compact)",
        cmd: AppCommand::Prompt,
    },
    CommandSpec {
        name: "reload",
        plain: Plain::Same,
        key: None,
        desc: "Re-read config & prompts from disk and apply them",
        cmd: AppCommand::Reload,
    },
    CommandSpec {
        name: "memory",
        plain: Plain::Same,
        key: None,
        desc: "Inspect, edit, and delete stored memory",
        cmd: AppCommand::Memory,
    },
    CommandSpec {
        name: "skills",
        plain: Plain::Same,
        key: None,
        desc: "List available skills and inspect a skill's instructions",
        cmd: AppCommand::Skills,
    },
    CommandSpec {
        name: "help",
        plain: Plain::Differs("List commands"),
        key: Some("Ctrl-P"),
        desc: "List commands and keybindings",
        cmd: AppCommand::Help,
    },
    CommandSpec {
        name: "quit",
        plain: Plain::Same,
        key: Some("Ctrl-D"),
        desc: "Exit emberly",
        cmd: AppCommand::Quit,
    },
];

/// Look up a registry entry by exact `/name` (the slash already stripped).
/// `/clear` is an alias for `/new` — both start a fresh session (the user's
/// "New or Clear") — and resolves to the canonical `new` entry.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static CommandSpec> {
    let name = if name == "clear" { "new" } else { name };
    COMMANDS.iter().find(|c| c.name == name)
}

/// One `/command` line resolved against the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slash<'a> {
    /// A registry entry and the argument text following its name — empty when
    /// the name stood alone. What the arguments mean is the frontend's business.
    Command(&'static CommandSpec, &'a str),
    /// A `/name` no entry matches. Both frontends report it as a typo; neither
    /// sends it to the model as a prompt.
    Unknown(&'a str),
}

/// Resolve one `/command` line — the leading slash already stripped — into a
/// registry entry plus its argument text.
///
/// Both frontends parse here, so the registry decides once which names exist,
/// which aliases resolve, and where a name ends and its arguments begin. A name
/// must end at whitespace, so `/modelx` is a typo rather than a `/model` call
/// with a mangled argument.
///
/// What a command *does* with its arguments stays the frontend's: the rich TUI
/// opens a picker where line mode, which has none, requires an explicit
/// argument (C-6/P-9, Design §7).
#[must_use]
pub fn parse_slash(input: &str) -> Slash<'_> {
    let input = input.trim();
    let (name, args) = match input.find(char::is_whitespace) {
        Some(end) => (&input[..end], input[end..].trim()),
        None => (input, ""),
    };
    match by_name(name) {
        Some(spec) => Slash::Command(spec, args),
        None => Slash::Unknown(name),
    }
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
        assert_eq!(by_name("help").map(|s| s.cmd), Some(AppCommand::Help));
        assert_eq!(by_name("quit").map(|s| s.cmd), Some(AppCommand::Quit));
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
    fn parse_slash_splits_a_name_from_its_arguments() {
        let Slash::Command(spec, args) = parse_slash("model zai glm-4.6") else {
            panic!("expected a command");
        };
        assert_eq!(spec.cmd, AppCommand::Model);
        assert_eq!(args, "zai glm-4.6");

        // A bare name has empty arguments, and surrounding space is not one.
        let Slash::Command(spec, args) = parse_slash("  compact  ") else {
            panic!("expected a command");
        };
        assert_eq!(spec.cmd, AppCommand::Compact);
        assert_eq!(args, "");
    }

    #[test]
    fn parse_slash_requires_the_name_to_end_at_whitespace() {
        // `/modelx` must not resolve to `/model`; it is simply unknown.
        assert_eq!(parse_slash("modelx"), Slash::Unknown("modelx"));
        assert_eq!(parse_slash(""), Slash::Unknown(""));
    }

    #[test]
    fn parse_slash_resolves_the_clear_alias_to_the_canonical_entry() {
        let Slash::Command(spec, _) = parse_slash("clear") else {
            panic!("expected a command");
        };
        assert_eq!(spec.name, "new");
        assert_eq!(spec.cmd, AppCommand::NewSession);
    }

    #[test]
    fn non_subsequence_is_filtered_out() {
        // No command name contains 'z'.
        assert!(matches("zzz").is_empty());
    }
}
