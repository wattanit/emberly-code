//! Configuration and secrets for the binary (Tech Spec §8; Phase 3 group 6).
//!
//! Resolution order (lowest → highest): built-in defaults → global
//! `~/.config/emberly/config.toml` → project `.agents/config.toml` →
//! `EMBERLY_*` environment overrides. API keys are read from the environment
//! first, else from `~/.config/emberly/keys.toml`, which must be `0600`
//! (secrets never live in project config and are never printed). Phase 5 adds
//! provenance tracking (C-3, [`show`]), project instructions (AGENTS.md/
//! CLAUDE.md, C-1), and per-model-family prompt resolution (P-7).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use emberly_core::{parse_rules, ConfigProvenance, Rule, RuleSource};
use serde::Deserialize;

/// A parsed `config.toml`. All fields optional so files can be partial.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ConfigFile {
    /// The active provider profile — which `[providers.<name>]` to use.
    pub provider: Option<String>,
    /// The active model id (looked up within the active profile).
    pub model: Option<String>,
    /// Named provider profiles (P-8, Tech Spec §4.5). Merged across tiers by
    /// name; baked-in profiles are injected at the lowest tier.
    #[serde(default)]
    pub providers: HashMap<String, ProfileFile>,
    /// Sandbox policy knobs (Requirements §6.7).
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// Reasoning-trail view default (Design §4.4): `collapsed` | `expanded` |
    /// `hidden`. A view choice only — the trace is always recorded (P-10).
    pub reasoning: Option<String>,
    /// `[ui]` presentation toggles (Tech Spec §8).
    #[serde(default)]
    pub ui: UiConfig,
    /// `[trust]` workspace-trust settings (FR-1). **Honored only from the global
    /// tier** — a project `[trust]` is ignored (a repo cannot self-trust); see
    /// [`global_trust_dirs`] and the notice in [`load`].
    #[serde(default)]
    pub trust: TrustConfig,
    /// `[loop]` guardrail settings (S-5). Threaded to the engine (Tech Spec §7).
    #[serde(default, rename = "loop")]
    pub loop_: LoopConfig,
}

/// `[ui]` — presentation toggles that shape what the interface shows without
/// changing what the agent may do (Tech Spec §8).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct UiConfig {
    /// Tool-call explanation line (T-9, Design §4.5). When on, the model is
    /// asked to caption non-obvious calls; when off, the schema property and the
    /// prompt instruction are both omitted so no tokens are spent. Default
    /// `true`.
    pub tool_explanations: Option<bool>,
}

/// `[trust]` — workspace trust (FR-1). `trusted_dirs` pre-declares folders
/// trusted without a prompt. **Global-tier only**: project config cannot
/// contribute here, so a repository can never pre-declare itself trusted.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct TrustConfig {
    #[serde(default)]
    pub trusted_dirs: Vec<String>,
}

/// `[loop]` — the loop-breaking guardrail (S-5, Tech Spec §7). All optional;
/// the engine applies defaults. Wired to the engine in a later group.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct LoopConfig {
    pub enabled: Option<bool>,
    pub repeat_window: Option<u32>,
    pub max_no_progress_turns: Option<u32>,
}

/// A `[providers.<name>]` profile: an adapter (wire format) plus the endpoint
/// and auth to reach it, so a new provider is configuration, not code (P-8).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ProfileFile {
    /// Wire-format adapter: `"anthropic"` or `"openai"`.
    pub adapter: Option<String>,
    /// Endpoint base URL. The `anthropic` adapter has a built-in default; the
    /// `openai` adapter defaults to the public API base.
    pub base_url: Option<String>,
    /// Authentication scheme + key reference (never an inline secret).
    pub auth: Option<AuthFile>,
    /// Optional per-model metadata, keyed by model id
    /// (`[providers.<name>.models."model-id"]`).
    #[serde(default)]
    pub models: HashMap<String, ModelFile>,
}

/// A profile's `auth = { scheme, header?, key }`. `key` is a *reference*
/// resolved from env / `keys.toml` ([`api_key`]), never a literal secret.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct AuthFile {
    /// `"bearer"`, `"x-api-key"`, `"header"`, or `"none"`.
    pub scheme: Option<String>,
    /// Header name when `scheme = "header"`.
    pub header: Option<String>,
    /// Key reference name (e.g. `"anthropic"`, `"zai"`).
    pub key: Option<String>,
}

/// Optional per-model metadata inside a profile.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ModelFile {
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    pub pricing: Option<PricingEntry>,
    /// The default reasoning-effort level (P-9): `low|medium|high|max`. Its
    /// presence declares that this model has an effort control; absent ⇒ no
    /// control and the effort picker is hidden.
    pub effort: Option<String>,
    /// The effort levels this model offers, if a subset. Omitted ⇒ the full
    /// ladder (`low|medium|high|max`) when `effort` is set.
    pub effort_levels: Option<Vec<String>>,
}

impl ProfileFile {
    /// Field-merge `higher` (higher precedence) onto `self`, so a user tier can
    /// tweak one field of a baked-in profile without respecifying the rest.
    fn merge(&mut self, higher: ProfileFile) {
        if higher.adapter.is_some() {
            self.adapter = higher.adapter;
        }
        if higher.base_url.is_some() {
            self.base_url = higher.base_url;
        }
        if let Some(higher_auth) = higher.auth {
            match &mut self.auth {
                Some(cur) => cur.merge(higher_auth),
                None => self.auth = Some(higher_auth),
            }
        }
        self.models.extend(higher.models);
    }
}

impl AuthFile {
    fn merge(&mut self, higher: AuthFile) {
        if higher.scheme.is_some() {
            self.scheme = higher.scheme;
        }
        if higher.header.is_some() {
            self.header = higher.header;
        }
        if higher.key.is_some() {
            self.key = higher.key;
        }
    }
}

/// Baked-in provider profiles (Requirements C-1) — usable out of the box; a
/// user only supplies the key. `emberly init` materializes these for editing
/// (C-2). Z.ai's concrete profile is added in Phase 1 group 4.
fn builtin_profiles() -> HashMap<String, ProfileFile> {
    let auth = |scheme: &str, key: &str| {
        Some(AuthFile {
            scheme: Some(scheme.to_string()),
            header: None,
            key: Some(key.to_string()),
        })
    };
    let profile = |adapter: &str, base_url: Option<&str>, auth: Option<AuthFile>| ProfileFile {
        adapter: Some(adapter.to_string()),
        base_url: base_url.map(str::to_string),
        auth,
        models: HashMap::new(),
    };
    HashMap::from([
        (
            "anthropic".to_string(),
            profile("anthropic", None, auth("x-api-key", "anthropic")),
        ),
        (
            "openai".to_string(),
            profile(
                "openai",
                Some("https://api.openai.com/v1"),
                auth("bearer", "openai"),
            ),
        ),
        (
            // Z.ai coding plan — OpenAI-compatible (bearer). Ready to use with
            // ZAI_API_KEY / keys.toml; the user only supplies the key.
            "zai".to_string(),
            profile(
                "openai",
                Some("https://api.z.ai/api/paas/v4"),
                auth("bearer", "zai"),
            ),
        ),
        (
            // A local OpenAI-compatible server (Ollama default); keyless.
            "local".to_string(),
            profile("openai", Some("http://localhost:11434/v1"), None),
        ),
    ])
}

/// The `[sandbox]` config section (Requirements §6.7).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SandboxConfig {
    /// `require = true` refuses to start a session without kernel confinement
    /// (default `false` — permissive but honest). Optional so a higher-precedence
    /// file can override a lower one either way.
    pub require: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PricingEntry {
    pub input: f64,
    pub output: f64,
}

impl ConfigFile {
    fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).context("failed to parse config.toml")
    }

    /// Overlay `higher` (higher precedence) onto `self`. Profiles field-merge
    /// by name; the active `provider`/`model` selectors are replaced.
    fn merge(&mut self, higher: ConfigFile) {
        if higher.provider.is_some() {
            self.provider = higher.provider;
        }
        if higher.model.is_some() {
            self.model = higher.model;
        }
        for (name, profile) in higher.providers {
            self.providers.entry(name).or_default().merge(profile);
        }
        if higher.sandbox.require.is_some() {
            self.sandbox.require = higher.sandbox.require;
        }
        if higher.reasoning.is_some() {
            self.reasoning = higher.reasoning;
        }
        if higher.ui.tool_explanations.is_some() {
            self.ui.tool_explanations = higher.ui.tool_explanations;
        }
        // `[loop]` (S-5) merges normally — a project may tune the guardrail.
        if higher.loop_.enabled.is_some() {
            self.loop_.enabled = higher.loop_.enabled;
        }
        if higher.loop_.repeat_window.is_some() {
            self.loop_.repeat_window = higher.loop_.repeat_window;
        }
        if higher.loop_.max_no_progress_turns.is_some() {
            self.loop_.max_no_progress_turns = higher.loop_.max_no_progress_turns;
        }
        // `[trust]` is deliberately NOT merged — it is read only from the global
        // tier (FR-1); see `global_trust_dirs` and the project-[trust] notice.
    }
}

/// The fully-resolved configuration the binary wires from.
pub struct Resolved {
    /// Active provider profile name (looked up in `providers`), or `None` to
    /// run the offline placeholder.
    pub provider: Option<String>,
    /// Active model id.
    pub model: Option<String>,
    /// All provider profiles (baked-in + user tiers, merged). `provider_setup`
    /// resolves the active one (Tech Spec §4.5).
    pub providers: HashMap<String, ProfileFile>,
    /// The system prompt = base prompt + project instructions (C-1). Always
    /// `Some` (the baked-in default at minimum).
    pub system_prompt: Option<String>,
    /// A `/compact` summarization-prompt override, or `None` for the engine
    /// default (P-7).
    pub summary_prompt: Option<String>,
    /// Every active configuration piece whose source is not the baked-in
    /// default (C-3), for `session_start` and `config show`.
    pub provenance: Vec<ConfigProvenance>,
    /// One-time notices to surface at startup (e.g. AGENTS.md over CLAUDE.md).
    pub notices: Vec<String>,
    /// `sandbox.require`: refuse to start without kernel confinement (§6.7).
    pub sandbox_require: bool,
    /// The reasoning-trail view default (`collapsed`|`expanded`|`hidden`,
    /// Design §4.4), or `None` for the built-in default (`collapsed`).
    pub reasoning: Option<String>,
    /// Whether tool-call explanations are enabled (T-9, Design §4.5). Default
    /// `true`; when `false` the schema property and prompt instruction are both
    /// omitted (no tokens spent).
    pub tool_explanations: bool,
    /// Resolved loop-breaking guardrail tunables (S-5), ready for the engine.
    pub loop_config: emberly_core::LoopConfig,
}

/// Command-line overrides (`--provider`/`--model`) — the highest-precedence
/// tier (Tech Spec §10).
#[derive(Default)]
pub struct CliOverrides {
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Environment overrides (`EMBERLY_*`), read once so provenance and the final
/// value agree. Profile-internal fields (base URL, window, pricing) live in
/// `[providers.*]` now, so only the active-selection knobs remain here.
#[derive(Default)]
struct EnvOverrides {
    provider: Option<String>,
    model: Option<String>,
}

impl EnvOverrides {
    fn read() -> Self {
        Self {
            provider: std::env::var("EMBERLY_PROVIDER")
                .ok()
                .filter(|s| !s.is_empty()),
            model: std::env::var("EMBERLY_MODEL")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }
}

/// Load and resolve configuration for a session rooted at `project_root`,
/// tracking where each active value came from (C-3). Resolution order (lowest →
/// highest): baked-in defaults → global → project → `EMBERLY_*` env.
pub fn load(project_root: &Path, cli: &CliOverrides) -> anyhow::Result<Resolved> {
    let global = read_config(global_config_path().as_deref())?;
    let project = read_config(Some(&project_root.join(".agents").join("config.toml")))?;
    let env = EnvOverrides::read();

    // Baked-in profiles are the lowest tier (C-1); user tiers field-merge onto
    // them, then env, then CLI select the active profile/model.
    let mut merged = ConfigFile {
        providers: builtin_profiles(),
        ..ConfigFile::default()
    };
    if let Some(g) = &global {
        merged.merge(g.clone());
    }
    if let Some(p) = &project {
        merged.merge(p.clone());
    }
    if let Some(v) = &env.provider {
        merged.provider = Some(v.clone());
    }
    if let Some(v) = &env.model {
        merged.model = Some(v.clone());
    }
    if let Some(p) = &cli.provider {
        merged.provider = Some(p.clone());
    }
    if let Some(m) = &cli.model {
        merged.model = Some(m.clone());
    }

    let mut provenance = Vec::new();
    // Source for each selector: cli > env > project > global > default.
    let src = |cli_set: bool, env_set: bool, project_has: bool, global_has: bool| {
        source_of(cli_set, env_set, project_has, global_has)
    };
    record(
        &mut provenance,
        "provider",
        src(
            cli.provider.is_some(),
            env.provider.is_some(),
            field(&project, |c| c.provider.is_some()),
            field(&global, |c| c.provider.is_some()),
        ),
        merged.provider.is_some(),
    );
    record(
        &mut provenance,
        "model",
        src(
            cli.model.is_some(),
            env.model.is_some(),
            field(&project, |c| c.model.is_some()),
            field(&global, |c| c.model.is_some()),
        ),
        merged.model.is_some(),
    );

    // Prompts (P-7): per-model-family variant, then the plain name, then the
    // baked-in default. The family comes from the resolved model.
    let prompts_dir = project_root.join(".agents").join("prompts");
    let family = family_of(merged.model.as_deref().unwrap_or_default());

    let (system_base, system_src) = load_prompt(&prompts_dir, "system", family).map_or_else(
        || (emberly_core::prompts::system().to_string(), "default"),
        |text| (text, "project"),
    );
    record(
        &mut provenance,
        "system_prompt",
        system_src.to_string(),
        true,
    );

    let (summary_prompt, summary_src) = match load_prompt(&prompts_dir, "compact", family) {
        Some(text) => (Some(text), "project"),
        None => (None, "default"),
    };
    if summary_src != "default" {
        record(
            &mut provenance,
            "compact_prompt",
            summary_src.to_string(),
            true,
        );
    }

    // Tool-call explanations (T-9): on unless a user tier turned it off; record
    // provenance only on a deviation from the default (speech about deviations).
    record(
        &mut provenance,
        "tool_explanations",
        source_of(
            false,
            false,
            field(&project, |c| c.ui.tool_explanations.is_some()),
            field(&global, |c| c.ui.tool_explanations.is_some()),
        ),
        merged.ui.tool_explanations.is_some(),
    );

    // Project instructions (C-1): AGENTS.md native; CLAUDE.md as a fallback;
    // both present → AGENTS.md wins with a notice.
    let mut notices = Vec::new();
    // A project `[trust]` is ignored — trust is global-only so a repo can't
    // pre-declare itself trusted (FR-1). Say so rather than silently dropping it.
    if project
        .as_ref()
        .is_some_and(|p| !p.trust.trusted_dirs.is_empty())
    {
        notices.push(
            "ignoring [trust] in project config — workspace trust is global-only (FR-1)".into(),
        );
    }
    let mut system_prompt = system_base;
    if let Some((instructions, source, notice)) = load_project_instructions(project_root)? {
        system_prompt.push_str("\n\n# Project instructions\n\n");
        system_prompt.push_str(&instructions);
        record(&mut provenance, "project_instructions", source, true);
        if let Some(notice) = notice {
            notices.push(notice);
        }
    }

    Ok(Resolved {
        provider: merged.provider,
        model: merged.model,
        providers: merged.providers,
        system_prompt: Some(system_prompt),
        summary_prompt,
        provenance,
        notices,
        sandbox_require: merged.sandbox.require.unwrap_or(false),
        reasoning: merged.reasoning,
        tool_explanations: merged.ui.tool_explanations.unwrap_or(true),
        loop_config: {
            // Override only the fields the user set; the engine owns the
            // defaults (S-5, Tech Spec §7 — "initial; tune with use").
            let d = emberly_core::LoopConfig::default();
            emberly_core::LoopConfig {
                enabled: merged.loop_.enabled.unwrap_or(d.enabled),
                repeat_window: merged
                    .loop_
                    .repeat_window
                    .map_or(d.repeat_window, |v| v as usize),
                max_no_progress_turns: merged
                    .loop_
                    .max_no_progress_turns
                    .map_or(d.max_no_progress_turns, |v| v as usize),
            }
        },
    })
}

/// Load permission rules from the global and project `permissions.toml` files
/// (Requirements §6.1, precedence order global → project). A malformed file is
/// skipped with a warning, never silently applied — a broken rules file must
/// not quietly widen *or* narrow access.
pub fn load_permission_rules(project_root: &Path) -> (Vec<Rule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let mut load = |path: Option<PathBuf>, source: RuleSource| {
        let Some(path) = path else { return };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        match parse_rules(&text, source) {
            Ok(mut parsed) => rules.append(&mut parsed),
            Err(error) => warnings.push(format!("ignoring {}: {error}", path.display())),
        }
    };
    load(global_permissions_path(), RuleSource::Global);
    load(
        Some(project_root.join(".agents").join("permissions.toml")),
        RuleSource::Project,
    );
    (rules, warnings)
}

fn global_permissions_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("permissions.toml"))
}

/// Read a config file if it exists.
fn read_config(path: Option<&Path>) -> anyhow::Result<Option<ConfigFile>> {
    let Some(path) = path else { return Ok(None) };
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(ConfigFile::parse(&text)?))
}

/// `true` if `config` is present and the field predicate holds.
fn field(config: &Option<ConfigFile>, has: impl Fn(&ConfigFile) -> bool) -> bool {
    config.as_ref().is_some_and(has)
}

/// The winning tier's label for a field.
fn source_of(cli: bool, env: bool, project: bool, global: bool) -> String {
    if cli {
        "cli"
    } else if env {
        "env"
    } else if project {
        "project:.agents/config.toml"
    } else if global {
        "global:~/.config/emberly/config.toml"
    } else {
        "default"
    }
    .to_string()
}

/// Record a provenance entry when the value is active and non-default.
fn record(provenance: &mut Vec<ConfigProvenance>, piece: &str, source: String, active: bool) {
    if active && source != "default" {
        provenance.push(ConfigProvenance {
            piece: piece.to_string(),
            source,
        });
    }
}

/// The model family for prompt-variant resolution (P-7).
fn family_of(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        "claude"
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        "gpt"
    } else {
        "generic"
    }
}

/// Resolve a prompt: `<name>.<family>.md`, then `<name>.md`, else `None`
/// (caller uses the built-in default). P-7.
fn load_prompt(prompts_dir: &Path, name: &str, family: &str) -> Option<String> {
    for candidate in [format!("{name}.{family}.md"), format!("{name}.md")] {
        if let Ok(text) = std::fs::read_to_string(prompts_dir.join(&candidate)) {
            let text = text.trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Load project instructions (C-1): returns `(text, source, notice)`.
/// `AGENTS.md` is native; `CLAUDE.md` is read only when `AGENTS.md` is absent;
/// with both present, `AGENTS.md` wins and a notice is returned.
fn load_project_instructions(
    root: &Path,
) -> anyhow::Result<Option<(String, String, Option<String>)>> {
    let agents = root.join("AGENTS.md");
    let claude = root.join("CLAUDE.md");
    let read = |p: &Path| std::fs::read_to_string(p).map(|t| t.trim().to_string());

    if agents.exists() {
        let text = read(&agents).with_context(|| format!("reading {}", agents.display()))?;
        let notice = claude.exists().then(|| {
            "using AGENTS.md for project instructions (CLAUDE.md is present but ignored)"
                .to_string()
        });
        return Ok((!text.is_empty()).then_some((text, "AGENTS.md".to_string(), notice)));
    }
    if claude.exists() {
        let text = read(&claude).with_context(|| format!("reading {}", claude.display()))?;
        return Ok((!text.is_empty()).then_some((text, "CLAUDE.md".to_string(), None)));
    }
    Ok(None)
}

/// `emberly config show`: print every active piece with its provenance tier
/// (C-3). Secrets are reported as set/unset, never printed.
pub fn show(project_root: &Path) -> anyhow::Result<()> {
    let resolved = load(project_root, &CliOverrides::default())?;
    println!(
        "emberly configuration (project: {})",
        project_root.display()
    );
    println!();
    print_value("active provider", resolved.provider.as_deref());
    print_value("active model", resolved.model.as_deref());

    println!();
    println!("provider profiles (name → adapter, endpoint, key):");
    let mut names: Vec<&String> = resolved.providers.keys().collect();
    names.sort();
    for name in names {
        let p = &resolved.providers[name];
        let adapter = p.adapter.as_deref().unwrap_or("(unset)");
        let base = p.base_url.as_deref().unwrap_or("(adapter default)");
        let key_ref = p.auth.as_ref().and_then(|a| a.key.as_deref());
        let key = key_ref.map_or_else(
            || "no key".to_string(),
            |r| format!("key '{r}' {}", key_status(r)),
        );
        let active = if resolved.provider.as_deref() == Some(name) {
            " (active)"
        } else {
            ""
        };
        println!("  {name}{active}: {adapter} @ {base}, {key}");
    }

    println!();
    println!("prompts & instructions:");
    for piece in ["system_prompt", "compact_prompt", "project_instructions"] {
        let source = resolved
            .provenance
            .iter()
            .find(|p| p.piece == piece)
            .map_or("default", |p| p.source.as_str());
        println!("  {piece}: {source}");
    }
    if !resolved.provenance.is_empty() {
        println!();
        println!("overrides (non-default sources):");
        for entry in &resolved.provenance {
            println!("  {} ← {}", entry.piece, entry.source);
        }
    }
    Ok(())
}

fn print_value(name: &str, value: Option<&str>) {
    println!("  {name:<18} {}", value.unwrap_or("(unset)"));
}

/// Whether the key for `reference` is resolvable (never prints the secret).
fn key_status(reference: &str) -> &'static str {
    match api_key(reference) {
        Ok(Some(_)) => "set",
        _ => "unset",
    }
}

/// Resolve the API key for a key *reference* (a profile's `auth.key`, e.g.
/// `"anthropic"`, `"openai"`, `"zai"`): the environment variable
/// `<REF>_API_KEY` first, then the matching entry in `keys.toml` (which must
/// be `0600`). Returns `None` if unset. `keys.toml` is a flat table of
/// `ref = "secret"`, never sourced from project config.
pub fn api_key(reference: &str) -> anyhow::Result<Option<String>> {
    let env_var = format!("{}_API_KEY", reference.to_ascii_uppercase());
    if let Ok(key) = std::env::var(&env_var) {
        if !key.is_empty() {
            return Ok(Some(key));
        }
    }

    let Some(path) = global_keys_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    enforce_private_permissions(&path)?;
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let keys: HashMap<String, String> =
        toml::from_str(&text).context("failed to parse keys.toml")?;
    Ok(keys.get(reference).filter(|s| !s.is_empty()).cloned())
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("emberly"));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("emberly"))
}

fn global_config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

fn global_keys_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("keys.toml"))
}

/// The global workspace-trust store (FR-1, Tech Spec §6.7). Global-only by
/// construction — `config_dir()` never consults the project root.
#[must_use]
pub fn global_trust_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("trust.toml"))
}

/// The `trust.trusted_dirs` pre-trust allowlist, read **only** from the global
/// config tier (FR-1 — project config cannot contribute). Missing/unparyable
/// global config yields an empty list; the gate then relies on the store alone.
#[must_use]
pub fn global_trust_dirs() -> Vec<String> {
    let Some(path) = global_config_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    ConfigFile::parse(&text)
        .map(|c| c.trust.trusted_dirs)
        .unwrap_or_default()
}

/// Refuse a secrets/trust file readable by group/other (Tech Spec §8, §6.7).
#[cfg(unix)]
pub(crate) fn enforce_private_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("reading permissions of {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} is group/world-accessible (mode {:o}); run `chmod 600 {}` — \
             secret keys must be private",
            path.display(),
            mode & 0o777,
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn enforce_private_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-cfg-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn family_maps_models_to_prompt_variants() {
        assert_eq!(family_of("claude-sonnet-5"), "claude");
        assert_eq!(family_of("gpt-5.2"), "gpt");
        assert_eq!(family_of("o3-mini"), "gpt");
        assert_eq!(family_of("llama-3"), "generic");
    }

    #[test]
    fn agents_md_wins_over_claude_md_with_a_notice() {
        let dir = tmp();
        std::fs::write(dir.join("AGENTS.md"), "agents rules").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "claude rules").unwrap();
        let (text, source, notice) = load_project_instructions(&dir).unwrap().unwrap();
        assert_eq!(text, "agents rules");
        assert_eq!(source, "AGENTS.md");
        assert!(notice.is_some(), "a notice explains CLAUDE.md is ignored");
    }

    #[test]
    fn claude_md_is_the_fallback() {
        let dir = tmp();
        std::fs::write(dir.join("CLAUDE.md"), "claude rules").unwrap();
        let (text, source, notice) = load_project_instructions(&dir).unwrap().unwrap();
        assert_eq!(text, "claude rules");
        assert_eq!(source, "CLAUDE.md");
        assert!(notice.is_none());
        // No instructions at all → None.
        assert!(load_project_instructions(&tmp()).unwrap().is_none());
    }

    #[test]
    fn prompt_prefers_family_variant_then_falls_back() {
        let dir = tmp();
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("system.claude.md"), "claude system").unwrap();
        std::fs::write(prompts.join("system.md"), "generic system").unwrap();
        assert_eq!(
            load_prompt(&prompts, "system", "claude").as_deref(),
            Some("claude system")
        );
        assert_eq!(
            load_prompt(&prompts, "system", "gpt").as_deref(),
            Some("generic system")
        );
        assert_eq!(load_prompt(&prompts, "missing", "claude"), None);
    }

    #[test]
    fn load_records_project_provenance_and_builds_system_prompt() {
        let dir = tmp();
        let agents = dir.join(".agents");
        std::fs::create_dir_all(&agents).unwrap();
        // Select a profile in the project tier and field-merge a base_url onto
        // the baked-in `openai` profile.
        std::fs::write(
            agents.join("config.toml"),
            "provider = \"openai\"\nmodel = \"llama\"\n\
             [providers.openai]\nbase_url = \"http://localhost:1234/v1\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("AGENTS.md"), "be careful").unwrap();

        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        // The active profile selector came from the project tier.
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "provider" && p.source.starts_with("project")));
        // Field-merge kept the baked-in adapter and applied the project base_url.
        let openai = resolved.providers.get("openai").expect("openai profile");
        assert_eq!(openai.adapter.as_deref(), Some("openai"));
        assert_eq!(openai.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "project_instructions" && p.source == "AGENTS.md"));
        // The system prompt is the default plus the project instructions.
        let system = resolved.system_prompt.unwrap();
        assert!(system.contains("Emberly Code"));
        assert!(system.contains("be careful"));
    }

    #[test]
    fn parses_provider_profile_with_model_metadata() {
        let text = r#"
            provider = "zai"
            model = "glm-x"
            [providers.zai]
            adapter = "anthropic"
            base_url = "https://example.test/api"
            auth = { scheme = "x-api-key", key = "zai" }
            [providers.zai.models."glm-x"]
            context_window = 128000
            max_output = 8192
            pricing = { input = 1.0, output = 2.0 }
        "#;
        let config = ConfigFile::parse(text).expect("parse");
        let zai = config.providers.get("zai").expect("zai profile");
        assert_eq!(zai.adapter.as_deref(), Some("anthropic"));
        assert_eq!(zai.base_url.as_deref(), Some("https://example.test/api"));
        let auth = zai.auth.as_ref().expect("auth");
        assert_eq!(auth.scheme.as_deref(), Some("x-api-key"));
        assert_eq!(auth.key.as_deref(), Some("zai"));
        let meta = zai.models.get("glm-x").expect("model meta");
        assert_eq!(meta.context_window, Some(128_000));
        assert_eq!(meta.max_output, Some(8_192));
        let pricing = meta.pricing.expect("pricing");
        assert!((pricing.input - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn builtin_profiles_present_and_field_merge() {
        let mut base = ConfigFile {
            providers: builtin_profiles(),
            ..ConfigFile::default()
        };
        for name in ["anthropic", "openai", "local"] {
            assert!(base.providers.contains_key(name), "baked-in: {name}");
        }
        // A user tier tweaks one field; the rest of the baked-in profile stays.
        let higher =
            ConfigFile::parse("[providers.openai]\nbase_url = \"http://x/v1\"").expect("higher");
        base.merge(higher);
        let openai = base.providers.get("openai").expect("openai");
        assert_eq!(openai.adapter.as_deref(), Some("openai")); // preserved
        assert_eq!(openai.base_url.as_deref(), Some("http://x/v1")); // overridden
    }

    #[test]
    fn merge_replaces_active_selectors() {
        let mut base = ConfigFile::parse("provider = \"openai\"\nmodel = \"a\"").expect("base");
        let higher = ConfigFile::parse("model = \"b\"").expect("higher");
        base.merge(higher);
        assert_eq!(base.provider.as_deref(), Some("openai")); // untouched
        assert_eq!(base.model.as_deref(), Some("b")); // overridden
    }

    #[test]
    fn ui_tool_explanations_parses_and_merges() {
        // Absent unless set (resolves to the `true` default in `load`).
        assert_eq!(
            ConfigFile::parse("").expect("empty").ui.tool_explanations,
            None
        );
        // Parsed from the `[ui]` table.
        let off = ConfigFile::parse("[ui]\ntool_explanations = false").expect("ui");
        assert_eq!(off.ui.tool_explanations, Some(false));
        // A higher tier overrides a lower one.
        let mut base = ConfigFile::parse("[ui]\ntool_explanations = true").expect("base");
        base.merge(off);
        assert_eq!(base.ui.tool_explanations, Some(false));
    }

    #[test]
    fn trust_parses_but_is_not_merged_global_only() {
        // A `[trust]` section parses into the file struct…
        let cfg = ConfigFile::parse("[trust]\ntrusted_dirs = [\"/a\", \"~/code\"]").expect("trust");
        assert_eq!(cfg.trust.trusted_dirs, vec!["/a", "~/code"]);
        // …but merge does NOT carry it, so a project tier can never contribute
        // trust (FR-1 — a repo can't self-trust). The gate reads global only.
        let mut base = ConfigFile::default();
        base.merge(cfg);
        assert!(
            base.trust.trusted_dirs.is_empty(),
            "trust is never merged from a higher tier"
        );
    }

    #[test]
    fn loop_config_parses_and_merges() {
        let cfg = ConfigFile::parse("[loop]\nenabled = false\nrepeat_window = 5").expect("loop");
        assert_eq!(cfg.loop_.enabled, Some(false));
        assert_eq!(cfg.loop_.repeat_window, Some(5));
        // Unlike trust, [loop] merges (a project may tune the guardrail).
        let mut base = ConfigFile::default();
        base.merge(cfg);
        assert_eq!(base.loop_.enabled, Some(false));
        assert_eq!(base.loop_.repeat_window, Some(5));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_readable_keys_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("emberly-keys-{}", std::process::id()));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("mkdir: {e}");
        }
        let path = dir.join("keys.toml");
        if let Err(e) = std::fs::write(&path, "openai = \"sk-x\"\n") {
            panic!("write: {e}");
        }
        // 0644 = group/other readable → must be refused.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        assert!(enforce_private_permissions(&path).is_err());
        // 0600 → accepted.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        assert!(enforce_private_permissions(&path).is_ok());
    }
}
