//! Configuration and secrets for the binary (Tech Spec §8; Phase 3 group 6).
//!
//! Resolution order (lowest → highest): built-in defaults → global
//! `~/.config/emberly/config.toml` → project `.agents/config.toml` →
//! `EMBERLY_*` environment overrides. API keys are read from the environment
//! first, else from `~/.config/emberly/keys.toml`, which must be `0600`
//! (secrets never live in project config and are never printed). The full
//! two-tier config + `--show-config` provenance system is Phase 5; this reads
//! only what Phase 3 needs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use emberly_providers::Pricing;
use serde::Deserialize;

/// A parsed `config.toml`. All fields optional so files can be partial.
#[derive(Debug, Default, Deserialize)]
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

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("EMBERLY_PROVIDER") {
            self.provider = Some(v);
        }
        if let Ok(v) = std::env::var("EMBERLY_MODEL") {
            self.model = Some(v);
        }
        if let Ok(v) = std::env::var("EMBERLY_BASE_URL") {
            self.base_url = Some(v);
        }
        if let Some(v) = env_u32("EMBERLY_CONTEXT_WINDOW") {
            self.context_window = Some(v);
        }
        if let Some(v) = env_u32("EMBERLY_MAX_OUTPUT") {
            self.max_output = Some(v);
        }
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
}

/// Load and resolve configuration for a session rooted at `project_root`.
pub fn load(project_root: &Path) -> anyhow::Result<Resolved> {
    let mut config = ConfigFile::default();

    if let Some(global) = global_config_path() {
        if global.exists() {
            let text = std::fs::read_to_string(&global)
                .with_context(|| format!("reading {}", global.display()))?;
            config.merge(ConfigFile::parse(&text)?);
        }
    }

    let project = project_root.join(".agents").join("config.toml");
    if project.exists() {
        let text = std::fs::read_to_string(&project)
            .with_context(|| format!("reading {}", project.display()))?;
        config.merge(ConfigFile::parse(&text)?);
    }

    config.apply_env();
    let pricing = config.resolved_pricing();

    Ok(Resolved {
        provider: config.provider,
        model: config.model,
        base_url: config.base_url,
        context_window: config.context_window,
        max_output: config.max_output,
        pricing,
    })
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
