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
use emberly_core::ConfigProvenance;
use emberly_providers::Pricing;
use serde::Deserialize;

/// The baked-in system prompt (Requirements C-1) — overridable at
/// `.agents/prompts/system.md` (or a per-family `system.<family>.md`, P-7).
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are Emberly Code, a careful terminal coding agent. \
Use the provided tools to read, write, and edit files and to run commands, all within the project \
root. Prefer small, verifiable steps. Explain what you are about to do before risky actions, and \
never work outside the project without the user's approval. Keep replies concise.";

/// A parsed `config.toml`. All fields optional so files can be partial.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ConfigFile {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    /// Per-model pricing, keyed by model id (USD per million tokens).
    #[serde(default)]
    pub pricing: HashMap<String, PricingEntry>,
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

    /// Overlay `higher` (higher precedence) onto `self`.
    fn merge(&mut self, higher: ConfigFile) {
        if higher.provider.is_some() {
            self.provider = higher.provider;
        }
        if higher.model.is_some() {
            self.model = higher.model;
        }
        if higher.base_url.is_some() {
            self.base_url = higher.base_url;
        }
        if higher.context_window.is_some() {
            self.context_window = higher.context_window;
        }
        if higher.max_output.is_some() {
            self.max_output = higher.max_output;
        }
        self.pricing.extend(higher.pricing); // higher-precedence entries win
    }

    /// Resolve pricing for the configured model, if present.
    fn resolved_pricing(&self) -> Option<Pricing> {
        let model = self.model.as_ref()?;
        self.pricing.get(model).map(|p| Pricing {
            input_per_mtok: p.input,
            output_per_mtok: p.output,
        })
    }
}

/// The fully-resolved configuration the binary wires from.
pub struct Resolved {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    pub pricing: Option<Pricing>,
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
}

/// Command-line overrides (`--provider`/`--model`) — the highest-precedence
/// tier (Tech Spec §10).
#[derive(Default)]
pub struct CliOverrides {
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Environment overrides (`EMBERLY_*`), read once so provenance and the final
/// value agree.
#[derive(Default)]
struct EnvOverrides {
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    context_window: Option<u32>,
    max_output: Option<u32>,
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
            base_url: std::env::var("EMBERLY_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            context_window: env_u32("EMBERLY_CONTEXT_WINDOW"),
            max_output: env_u32("EMBERLY_MAX_OUTPUT"),
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

    // Final values via the existing merge, then env, then CLI on top.
    let mut merged = ConfigFile::default();
    if let Some(g) = &global {
        merged.merge(g.clone());
    }
    if let Some(p) = &project {
        merged.merge(p.clone());
    }
    apply_env(&mut merged, &env);
    if let Some(p) = &cli.provider {
        merged.provider = Some(p.clone());
    }
    if let Some(m) = &cli.model {
        merged.model = Some(m.clone());
    }

    let mut provenance = Vec::new();
    // Source for each scalar: cli > env > project > global > default.
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
    record(
        &mut provenance,
        "base_url",
        src(
            false,
            env.base_url.is_some(),
            field(&project, |c| c.base_url.is_some()),
            field(&global, |c| c.base_url.is_some()),
        ),
        merged.base_url.is_some(),
    );
    record(
        &mut provenance,
        "context_window",
        src(
            false,
            env.context_window.is_some(),
            field(&project, |c| c.context_window.is_some()),
            field(&global, |c| c.context_window.is_some()),
        ),
        merged.context_window.is_some(),
    );
    record(
        &mut provenance,
        "max_output",
        src(
            false,
            env.max_output.is_some(),
            field(&project, |c| c.max_output.is_some()),
            field(&global, |c| c.max_output.is_some()),
        ),
        merged.max_output.is_some(),
    );

    let pricing = merged.resolved_pricing();
    if pricing.is_some() {
        let has = |c: &ConfigFile| {
            merged
                .model
                .as_ref()
                .is_some_and(|m| c.pricing.contains_key(m))
        };
        record(
            &mut provenance,
            "pricing",
            src(false, false, field(&project, has), field(&global, has)),
            true,
        );
    }

    // Prompts (P-7): per-model-family variant, then the plain name, then the
    // baked-in default. The family comes from the resolved model.
    let prompts_dir = project_root.join(".agents").join("prompts");
    let family = family_of(merged.model.as_deref().unwrap_or_default());

    let (system_base, system_src) = load_prompt(&prompts_dir, "system", family)
        .map_or((DEFAULT_SYSTEM_PROMPT.to_string(), "default"), |text| {
            (text, "project")
        });
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

    // Project instructions (C-1): AGENTS.md native; CLAUDE.md as a fallback;
    // both present → AGENTS.md wins with a notice.
    let mut notices = Vec::new();
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
        base_url: merged.base_url,
        context_window: merged.context_window,
        max_output: merged.max_output,
        pricing,
        system_prompt: Some(system_prompt),
        summary_prompt,
        provenance,
        notices,
    })
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

/// Apply env overrides onto a merged config.
fn apply_env(config: &mut ConfigFile, env: &EnvOverrides) {
    if let Some(v) = &env.provider {
        config.provider = Some(v.clone());
    }
    if let Some(v) = &env.model {
        config.model = Some(v.clone());
    }
    if let Some(v) = &env.base_url {
        config.base_url = Some(v.clone());
    }
    if let Some(v) = env.context_window {
        config.context_window = Some(v);
    }
    if let Some(v) = env.max_output {
        config.max_output = Some(v);
    }
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
    print_value("provider", resolved.provider.as_deref());
    print_value("model", resolved.model.as_deref());
    print_value("base_url", resolved.base_url.as_deref());
    print_value(
        "context_window",
        resolved.context_window.map(|v| v.to_string()).as_deref(),
    );
    print_value(
        "max_output",
        resolved.max_output.map(|v| v.to_string()).as_deref(),
    );
    println!(
        "  pricing:         {}",
        if resolved.pricing.is_some() {
            "configured"
        } else {
            "none"
        }
    );
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
    println!();
    println!("secrets (never printed):");
    println!("  anthropic_api_key: {}", key_status("anthropic"));
    println!("  openai_api_key:    {}", key_status("openai"));
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
    println!("  {name:<16} {}", value.unwrap_or("(unset)"));
}

fn key_status(provider: &str) -> &'static str {
    match api_key(provider) {
        Ok(Some(_)) => "set",
        _ => "unset",
    }
}

/// Keys file schema (`keys.toml`). Never sourced from project config.
#[derive(Debug, Default, Deserialize)]
struct KeysFile {
    anthropic: Option<String>,
    openai: Option<String>,
}

/// Resolve the API key for `provider`: environment first, then `keys.toml`
/// (which must be `0600`). Returns `None` if unset.
pub fn api_key(provider: &str) -> anyhow::Result<Option<String>> {
    let env_var = match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        _ => "OPENAI_API_KEY",
    };
    if let Ok(key) = std::env::var(env_var) {
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
    let keys: KeysFile = toml::from_str(&text).context("failed to parse keys.toml")?;
    Ok(match provider {
        "anthropic" => keys.anthropic,
        _ => keys.openai,
    })
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

/// Refuse a secrets file readable by group/other (Tech Spec §8).
#[cfg(unix)]
fn enforce_private_permissions(path: &Path) -> anyhow::Result<()> {
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
fn enforce_private_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
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
        // base_url is unlikely to be set in a dev's global config, so its
        // provenance reliably points at the project tier here.
        std::fs::write(
            agents.join("config.toml"),
            "provider = \"openai\"\nbase_url = \"http://localhost:1234/v1\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("AGENTS.md"), "be careful").unwrap();

        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        assert_eq!(
            resolved.base_url.as_deref(),
            Some("http://localhost:1234/v1")
        );
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "base_url" && p.source.starts_with("project")));
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
    fn parses_and_resolves_pricing_for_the_model() {
        let text = r#"
            provider = "anthropic"
            model = "claude-x"
            [pricing."claude-x"]
            input = 3.0
            output = 15.0
        "#;
        let config = ConfigFile::parse(text).expect("parse");
        let pricing = config.resolved_pricing().expect("pricing for model");
        assert!((pricing.input_per_mtok - 3.0).abs() < f64::EPSILON);
        assert!((pricing.output_per_mtok - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn no_pricing_when_model_absent_from_table() {
        let text = r#"
            model = "gpt-x"
            [pricing."other"]
            input = 1.0
            output = 2.0
        "#;
        let config = ConfigFile::parse(text).expect("parse");
        assert!(config.resolved_pricing().is_none());
    }

    #[test]
    fn merge_gives_higher_precedence_priority() {
        let mut base = ConfigFile::parse("provider = \"openai\"\nmodel = \"a\"").expect("base");
        let higher = ConfigFile::parse("model = \"b\"").expect("higher");
        base.merge(higher);
        assert_eq!(base.provider.as_deref(), Some("openai")); // untouched
        assert_eq!(base.model.as_deref(), Some("b")); // overridden
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
