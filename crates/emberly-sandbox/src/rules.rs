//! The permission rule engine (Requirements §6.1, §6.2, §6.5, §6.6; Tech Spec
//! §6.1) — the *convenience* layer that decides **when to ask**. It is not the
//! security boundary (that is OS confinement, [`crate::probe`]); it must never
//! be described as one (Requirements §6.1).
//!
//! A [`Rule`] is `(tool, matcher) -> `[`Decision`]. Rules come from four
//! sources with a fixed precedence — built-in defaults, global config, project
//! `permissions.toml`, in-memory session grants — and are evaluated
//! **most-specific-first within the highest-precedence source that matches**
//! (see [`RuleEngine::decide`]). The result is then adjusted for the two hard,
//! non-waivable facts the rule layer still honors: paths **outside the project
//! root** can never be auto-allowed (HC-4), and the current [`Mode`] may only
//! ever *relax* an `Ask` to `Allow`, never touch a `Deny`.
//!
//! Bash matchers are **prefix** matchers on the command string — explicitly
//! convenience-tier (Requirements §6.5). No shell parsing: subshells, chaining,
//! and expansions are not analyzed, because doing so as a security mechanism
//! cannot be won and breeds false confidence. Real containment is the sandbox.
//!
//! Pure logic, no OS calls — unit-testable on any platform.

use serde::{Deserialize, Serialize};

use crate::mode::Mode;

/// A permission decision: allow silently, ask the user, or refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Run without prompting (no UI churn).
    Allow,
    /// Prompt the user (the Phase 1 always-on behavior, now rule-driven).
    Ask,
    /// Refuse, returning the reason to the model as data (HC-6).
    Deny,
}

/// A permission query in neutral, tool-independent terms. The engine (core)
/// builds this from a tool's `PermissionRequest`; the rule crate stays
/// independent of `emberly-tools`.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    /// Tool name (`bash`, `read_file`, `write_file`, `edit_file`, `glob`, …).
    pub tool: &'a str,
    /// The full command string, for bash prefix matching. `None` for non-bash.
    pub command: Option<&'a str>,
    /// Whether any affected path is outside the project root (HC-4).
    pub outside_root: bool,
}

/// Which tool a rule applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSelector {
    /// Any tool (`tool = "*"` in config).
    Any,
    /// A specific tool by name.
    Named(String),
}

impl ToolSelector {
    fn matches(&self, tool: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Named(name) => name == tool,
        }
    }
}

/// How a rule matches a query's command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Matcher {
    /// Matches any invocation of the selected tool.
    Any,
    /// Matches when the (trimmed) command begins with this token-boundary
    /// prefix — `git status` matches `git status --short` but not `git statusx`
    /// (Requirements §6.5: prefix, not shell parsing).
    BashPrefix(String),
}

impl Matcher {
    fn matches(&self, query: &Query) -> bool {
        match self {
            Self::Any => true,
            Self::BashPrefix(prefix) => query
                .command
                .is_some_and(|command| command_has_prefix(command, prefix)),
        }
    }

    /// Higher = more specific. A prefix is more specific than "any", and among
    /// prefixes the longer one wins.
    fn specificity(&self) -> usize {
        match self {
            Self::Any => 0,
            Self::BashPrefix(prefix) => 1 + prefix.len(),
        }
    }
}

/// Token-boundary prefix match: the command, trimmed, equals `prefix` or
/// continues with whitespace after it. Avoids `git status` matching `gitx`.
fn command_has_prefix(command: &str, prefix: &str) -> bool {
    let command = command.trim_start();
    let prefix = prefix.trim();
    match command.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => rest.starts_with(char::is_whitespace),
        None => false,
    }
}

/// Where a rule came from — sets its precedence (Session > Project > Global >
/// Builtin) and its human-readable "why".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuleSource {
    Builtin,
    Global,
    Project,
    Session,
}

impl RuleSource {
    fn label(self) -> &'static str {
        match self {
            Self::Builtin => "default rule",
            Self::Global => "your global config",
            Self::Project => "this project's permissions.toml",
            Self::Session => "a grant you made this session",
        }
    }
}

/// One `(tool, matcher) -> decision` rule, tagged with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tool: ToolSelector,
    pub matcher: Matcher,
    pub action: Decision,
    pub source: RuleSource,
}

impl Rule {
    fn matches(&self, query: &Query) -> bool {
        self.tool.matches(query.tool) && self.matcher.matches(query)
    }

    /// Format this rule as a `permissions.toml` `[[rule]]` block, for the
    /// "always allow in project" write-back that is shown to the user (§6.6).
    #[must_use]
    pub fn to_toml_block(&self) -> String {
        let tool = match &self.tool {
            ToolSelector::Any => "*",
            ToolSelector::Named(name) => name.as_str(),
        };
        let action = match self.action {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        };
        match &self.matcher {
            Matcher::Any => {
                format!("[[rule]]\ntool = \"{tool}\"\naction = \"{action}\"\n")
            }
            Matcher::BashPrefix(prefix) => {
                format!(
                    "[[rule]]\ntool = \"{tool}\"\nmatch = \"{prefix}\"\naction = \"{action}\"\n"
                )
            }
        }
    }
}

/// The outcome of evaluating a query: a decision plus the plain-language reason
/// it was reached, shown as the dimmed "why" line at the prompt (Design §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub decision: Decision,
    pub reason: String,
}

/// The compiled rule set for a session: built-in defaults plus config and
/// session rules, with the bash-allowlist toggle the degradation policy flips
/// (Requirements §6.7 — suspended when confinement is unavailable).
#[derive(Debug, Clone)]
pub struct RuleEngine {
    rules: Vec<Rule>,
}

impl RuleEngine {
    /// Build the engine: built-in defaults (the bash allowlist included only
    /// when `bash_allowlist_active`) then `config_rules` (global + project, in
    /// that order). Session grants are added later via
    /// [`add_session_grant`](RuleEngine::add_session_grant).
    #[must_use]
    pub fn new(config_rules: Vec<Rule>, bash_allowlist_active: bool) -> Self {
        let mut rules = builtin_defaults(bash_allowlist_active);
        rules.extend(config_rules);
        Self { rules }
    }

    /// Add an in-memory session grant ("allow for this session").
    pub fn add_session_grant(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Evaluate a query under the current [`Mode`]: the base rule decision,
    /// then the two hard adjustments — outside-root can never be auto-allowed
    /// (HC-4), and the mode may only relax an in-root `Ask` to `Allow`.
    #[must_use]
    pub fn evaluate(&self, query: &Query, mode: Mode) -> Outcome {
        let base = self.decide(query);

        // HC-4: paths outside the root are ask-always, never auto-allowed by
        // any rule or mode. A Deny still stands (a rule may hard-refuse them).
        if query.outside_root {
            return match base.decision {
                Decision::Deny => base,
                _ => Outcome {
                    decision: Decision::Ask,
                    reason: "this action affects paths OUTSIDE the project root".to_string(),
                },
            };
        }

        // Mode relaxations only ever turn Ask -> Allow, never touch Deny.
        if base.decision == Decision::Ask {
            if let Some(reason) = mode_auto_allow(mode, query) {
                return Outcome {
                    decision: Decision::Allow,
                    reason,
                };
            }
        }
        base
    }

    /// The base rule decision, ignoring mode and outside-root: the
    /// highest-precedence source that has any matching rule decides, and within
    /// that source the most-specific matcher wins (Requirements §6.1). No match
    /// anywhere falls back to `Ask` — the always-safe default.
    fn decide(&self, query: &Query) -> Outcome {
        let winner = self
            .rules
            .iter()
            .filter(|rule| rule.matches(query))
            .max_by(|a, b| {
                a.source
                    .cmp(&b.source)
                    .then(a.matcher.specificity().cmp(&b.matcher.specificity()))
            });

        match winner {
            Some(rule) => Outcome {
                decision: rule.action,
                reason: reason_for(rule, query),
            },
            None => Outcome {
                decision: Decision::Ask,
                reason: default_ask_reason(query),
            },
        }
    }
}

/// The human "why" for a matched rule.
fn reason_for(rule: &Rule, query: &Query) -> String {
    let verb = match rule.action {
        Decision::Allow => "allowed",
        Decision::Ask => "asks",
        Decision::Deny => "denied",
    };
    match (&rule.matcher, query.command) {
        (Matcher::BashPrefix(prefix), _) => {
            format!("`{prefix}` {verb} by {}", rule.source.label())
        }
        _ => format!("{} {verb} by {}", query.tool, rule.source.label()),
    }
}

/// The "why" when nothing matched and we fall back to asking.
fn default_ask_reason(query: &Query) -> String {
    match query.tool {
        "bash" => "this command is not on the allowlist".to_string(),
        "write_file" | "edit_file" => "writing files needs your approval".to_string(),
        other => format!("{other} needs your approval"),
    }
}

/// Whether the current mode auto-allows this in-root `Ask` (Requirements §6.4,
/// per the owner's tier definition). This is reached only for an in-root action
/// whose base decision is `Ask` — outside-root (HC-4) and `Deny` are already
/// resolved before it, so no mode can override either.
///
/// - `Normal`: nothing extra. Reads and allowlisted (non-destructive) bash
///   already `Allow` at the rule layer, so they run silently here too.
/// - `AutoAcceptEdits`: additionally the file write/edit tools.
/// - `Auto`: everything in-root. Safe because auto modes are reachable only
///   under active OS confinement, which enforces the hard lines the rule layer
///   no longer asks about (root confinement; `.git` writes only via genuine
///   git). File tools also keep their own hard `.git` refusal, belt-and-braces.
///
/// Kept as one function so the tier policy is easy to see and adjust.
fn mode_auto_allow(mode: Mode, query: &Query) -> Option<String> {
    match mode {
        Mode::Normal => None,
        Mode::AutoAcceptEdits => matches!(query.tool, "write_file" | "edit_file")
            .then(|| "auto-accept edits is on (change inside the project root)".to_string()),
        Mode::Auto => {
            Some("auto mode is on (runs inside the project root without asking)".to_string())
        }
    }
}

/// The default bash allowlist (Tech Spec §6.1): harmless, read-only commands.
/// User-extensible via project config; the final list is a living default.
pub const DEFAULT_BASH_ALLOWLIST: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "grep",
    "rg",
    "find",
    "pwd",
    "echo",
    "which",
    "git status",
    "git diff",
    "git log",
    "git show",
    "git branch",
    "cargo check",
    "cargo tree",
    "cargo metadata",
];

/// The built-in default rules (lowest precedence). Reads in-root allow; the
/// bash allowlist allows (when active); everything else falls through to `Ask`.
/// Writes/edits and off-list bash have no rule here — they hit the `Ask`
/// fallback, which the mode layer may relax for edits.
fn builtin_defaults(bash_allowlist_active: bool) -> Vec<Rule> {
    let mut rules = vec![Rule {
        tool: ToolSelector::Named("read_file".to_string()),
        matcher: Matcher::Any,
        action: Decision::Allow,
        source: RuleSource::Builtin,
    }];
    if bash_allowlist_active {
        for prefix in DEFAULT_BASH_ALLOWLIST {
            rules.push(Rule {
                tool: ToolSelector::Named("bash".to_string()),
                matcher: Matcher::BashPrefix((*prefix).to_string()),
                action: Decision::Allow,
                source: RuleSource::Builtin,
            });
        }
    }
    rules
}

/// The `permissions.toml` schema (also the global-config `[[rule]]` shape). All
/// deserialization lives with the engine so the format is defined once.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PermissionsFile {
    #[serde(default, rename = "rule")]
    pub rules: Vec<RuleSpec>,
}

/// One `[[rule]]` entry from config: `tool`, optional `match` (bash prefix),
/// and `action`.
#[derive(Debug, Clone, Deserialize)]
pub struct RuleSpec {
    pub tool: String,
    #[serde(rename = "match")]
    #[serde(default)]
    pub matcher: Option<String>,
    pub action: Decision,
}

impl RuleSpec {
    /// Convert a parsed config entry into a [`Rule`] tagged with its `source`.
    #[must_use]
    pub fn into_rule(self, source: RuleSource) -> Rule {
        let tool = if self.tool == "*" {
            ToolSelector::Any
        } else {
            ToolSelector::Named(self.tool)
        };
        let matcher = match self.matcher {
            Some(prefix) if !prefix.trim().is_empty() => {
                // Accept a trailing `*` as convenience sugar (`cargo *`); the
                // match is a prefix regardless (§6.5), so strip it.
                Matcher::BashPrefix(prefix.trim().trim_end_matches('*').trim().to_string())
            }
            _ => Matcher::Any,
        };
        Rule {
            tool,
            matcher,
            action: self.action,
            source,
        }
    }
}

/// Parse a `permissions.toml`/global-config rule file into rules tagged with
/// `source`. Returns an empty vec for an absent/empty file; a parse error is
/// surfaced so the caller can warn (a broken rules file must never silently
/// widen access).
pub fn parse_rules(text: &str, source: RuleSource) -> Result<Vec<Rule>, toml::de::Error> {
    let file: PermissionsFile = toml::from_str(text)?;
    Ok(file
        .rules
        .into_iter()
        .map(|spec| spec.into_rule(source))
        .collect())
}

/// Build a session-grant rule for a bash command prefix (from "allow for this
/// session" on a bash prompt).
#[must_use]
pub fn bash_session_grant(prefix: impl Into<String>) -> Rule {
    Rule {
        tool: ToolSelector::Named("bash".to_string()),
        matcher: Matcher::BashPrefix(prefix.into()),
        action: Decision::Allow,
        source: RuleSource::Session,
    }
}

/// Build a session-grant rule allowing a tool (used for file writes/edits, which
/// have no command to prefix-match — the grant is per-tool for the session).
#[must_use]
pub fn tool_session_grant(tool: impl Into<String>) -> Rule {
    Rule {
        tool: ToolSelector::Named(tool.into()),
        matcher: Matcher::Any,
        action: Decision::Allow,
        source: RuleSource::Session,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A confined-mode engine on the built-in defaults only.
    fn confined() -> RuleEngine {
        RuleEngine::new(Vec::new(), true)
    }

    fn bash<'a>(command: &'a str) -> Query<'a> {
        Query {
            tool: "bash",
            command: Some(command),
            outside_root: false,
        }
    }

    fn file<'a>(tool: &'a str, outside_root: bool) -> Query<'a> {
        Query {
            tool,
            command: None,
            outside_root,
        }
    }

    #[test]
    fn reads_in_root_allow_writes_ask() {
        let e = confined();
        assert_eq!(
            e.evaluate(&file("read_file", false), Mode::Normal).decision,
            Decision::Allow
        );
        assert_eq!(
            e.evaluate(&file("write_file", false), Mode::Normal)
                .decision,
            Decision::Ask
        );
        assert_eq!(
            e.evaluate(&file("edit_file", false), Mode::Normal).decision,
            Decision::Ask
        );
    }

    #[test]
    fn allowlisted_bash_allows_offlist_asks() {
        let e = confined();
        assert_eq!(
            e.evaluate(&bash("git status --short"), Mode::Normal)
                .decision,
            Decision::Allow
        );
        assert_eq!(
            e.evaluate(&bash("cargo check"), Mode::Normal).decision,
            Decision::Allow
        );
        assert_eq!(
            e.evaluate(&bash("grep -n foo src/lib.rs"), Mode::Normal)
                .decision,
            Decision::Allow
        );
        assert_eq!(
            e.evaluate(&bash("rm -rf /"), Mode::Normal).decision,
            Decision::Ask
        );
        // Prefix must be at a token boundary: `git statusx` is NOT `git status`.
        assert_eq!(
            e.evaluate(&bash("git statusx"), Mode::Normal).decision,
            Decision::Ask
        );
        // A lookalike that only shares a leading substring does not match.
        assert_eq!(
            e.evaluate(&bash("lsof"), Mode::Normal).decision,
            Decision::Ask
        );
    }

    #[test]
    fn degraded_suspends_the_bash_allowlist() {
        let degraded = RuleEngine::new(Vec::new(), false);
        // Allowlisted command now asks: the convenience tier is suspended.
        assert_eq!(
            degraded
                .evaluate(&bash("git status"), Mode::Normal)
                .decision,
            Decision::Ask
        );
        // Reads still allow — that rule never depended on the kernel.
        assert_eq!(
            degraded
                .evaluate(&file("read_file", false), Mode::Normal)
                .decision,
            Decision::Allow
        );
    }

    #[test]
    fn outside_root_never_auto_allows() {
        let e = confined();
        // Even a read — normally Allow — must ask when it escapes the root.
        assert_eq!(
            e.evaluate(&file("read_file", true), Mode::Normal).decision,
            Decision::Ask
        );
        // And no mode can relax it.
        assert_eq!(
            e.evaluate(&file("write_file", true), Mode::Auto).decision,
            Decision::Ask
        );
    }

    #[test]
    fn outside_root_deny_still_denies() {
        let deny_outside = RuleSpec {
            tool: "write_file".into(),
            matcher: None,
            action: Decision::Deny,
        }
        .into_rule(RuleSource::Project);
        let e = RuleEngine::new(vec![deny_outside], true);
        assert_eq!(
            e.evaluate(&file("write_file", true), Mode::Auto).decision,
            Decision::Deny
        );
    }

    #[test]
    fn project_overrides_builtin_session_overrides_project() {
        // Project denies all bash; a builtin allowlist entry cannot re-allow it.
        let project_deny = RuleSpec {
            tool: "bash".into(),
            matcher: None,
            action: Decision::Deny,
        }
        .into_rule(RuleSource::Project);
        let mut e = RuleEngine::new(vec![project_deny], true);
        assert_eq!(
            e.evaluate(&bash("git status"), Mode::Normal).decision,
            Decision::Deny,
        );
        // A session grant (highest precedence) re-allows a specific command.
        e.add_session_grant(bash_session_grant("git status"));
        assert_eq!(
            e.evaluate(&bash("git status"), Mode::Normal).decision,
            Decision::Allow,
        );
        // But the blanket project deny still governs other bash.
        assert_eq!(
            e.evaluate(&bash("ls"), Mode::Normal).decision,
            Decision::Deny,
        );
    }

    #[test]
    fn auto_accept_edits_relaxes_only_edits() {
        let e = confined();
        for tool in ["write_file", "edit_file"] {
            assert_eq!(
                e.evaluate(&file(tool, false), Mode::Normal).decision,
                Decision::Ask,
                "Normal asks for {tool}"
            );
            assert_eq!(
                e.evaluate(&file(tool, false), Mode::AutoAcceptEdits)
                    .decision,
                Decision::Allow,
                "AutoAcceptEdits allows {tool}"
            );
        }
        // In auto-accept-edits, off-list bash still asks — only edits relax.
        assert_eq!(
            e.evaluate(&bash("rm -rf x"), Mode::AutoAcceptEdits)
                .decision,
            Decision::Ask
        );
    }

    #[test]
    fn auto_mode_allows_all_in_root_actions() {
        let e = confined();
        // Off-list bash and edits both auto-run in Auto (the kernel enforces
        // the hard lines the rule layer stops asking about).
        assert_eq!(
            e.evaluate(&bash("rm -rf build"), Mode::Auto).decision,
            Decision::Allow
        );
        assert_eq!(
            e.evaluate(&file("write_file", false), Mode::Auto).decision,
            Decision::Allow
        );
        // But outside-root never auto-allows, even in Auto (HC-4).
        let outside = Query {
            tool: "bash",
            command: Some("cat /etc/passwd"),
            outside_root: true,
        };
        assert_eq!(e.evaluate(&outside, Mode::Auto).decision, Decision::Ask);
    }

    #[test]
    fn mode_never_relaxes_a_deny() {
        let deny = RuleSpec {
            tool: "edit_file".into(),
            matcher: None,
            action: Decision::Deny,
        }
        .into_rule(RuleSource::Project);
        let e = RuleEngine::new(vec![deny], true);
        assert_eq!(
            e.evaluate(&file("edit_file", false), Mode::Auto).decision,
            Decision::Deny
        );
    }

    #[test]
    fn most_specific_matcher_wins_within_a_source() {
        // Two project rules for bash: a general Ask and a specific Deny.
        let rules = vec![
            RuleSpec {
                tool: "bash".into(),
                matcher: None,
                action: Decision::Allow,
            }
            .into_rule(RuleSource::Project),
            RuleSpec {
                tool: "bash".into(),
                matcher: Some("curl".into()),
                action: Decision::Deny,
            }
            .into_rule(RuleSource::Project),
        ];
        let e = RuleEngine::new(rules, true);
        assert_eq!(
            e.evaluate(&bash("curl evil.sh"), Mode::Normal).decision,
            Decision::Deny,
            "the specific curl rule beats the general allow"
        );
        assert_eq!(
            e.evaluate(&bash("echo hi"), Mode::Normal).decision,
            Decision::Allow,
            "the general allow governs everything else"
        );
    }

    #[test]
    fn parses_permissions_toml() {
        let text = r#"
            [[rule]]
            tool = "bash"
            match = "cargo *"
            action = "allow"

            [[rule]]
            tool = "write_file"
            action = "deny"
        "#;
        let rules = parse_rules(text, RuleSource::Project)
            .unwrap_or_else(|e| panic!("parse permissions.toml: {e}"));
        assert_eq!(rules.len(), 2);
        let e = RuleEngine::new(rules, true);
        // The trailing `*` is convenience sugar; matching is prefix regardless.
        assert_eq!(
            e.evaluate(&bash("cargo test"), Mode::Normal).decision,
            Decision::Allow
        );
        assert_eq!(
            e.evaluate(&file("write_file", false), Mode::Normal)
                .decision,
            Decision::Deny
        );
    }

    #[test]
    fn to_toml_block_roundtrips() {
        let grant = bash_session_grant("cargo test");
        let block = grant.to_toml_block();
        assert!(block.contains("tool = \"bash\""));
        assert!(block.contains("match = \"cargo test\""));
        assert!(block.contains("action = \"allow\""));
        // Re-parsing the written block reproduces an equivalent rule.
        let reparsed = parse_rules(&block, RuleSource::Session)
            .unwrap_or_else(|e| panic!("reparse written block: {e}"));
        assert_eq!(reparsed.len(), 1);
        assert_eq!(reparsed[0].action, Decision::Allow);
    }

    #[test]
    fn invalid_rules_file_is_an_error_not_a_silent_widening() {
        assert!(parse_rules("this is not = toml [[[", RuleSource::Project).is_err());
    }
}
