//! [`ConfigWriter`] — the binary-side implementation of
//! [`emberly_core::ProviderProfileWriter`], the guided setup wizard's write
//! seam (Requirements C-7, Tech Spec §8/§9). Writes a new
//! `[providers.<name>]` table into the project's `.agents/config.toml` and
//! the matching secret into the global `keys.toml`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use emberly_core::{NewProviderProfile, ProviderProfileWriter};

use crate::config::{self, CliOverrides};

pub struct ConfigWriter {
    project_root: PathBuf,
}

impl ConfigWriter {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }
}

impl ProviderProfileWriter for ConfigWriter {
    fn write_profile(&self, profile: NewProviderProfile) -> Result<(), String> {
        // Create-only scope (Tech Spec §16 open item): refuse a name that
        // already resolves to a profile, pointing at raw editing (C-5) for
        // changing an existing one.
        let resolved = config::load(&self.project_root, &CliOverrides::default())
            .map_err(|e| e.to_string())?;
        if resolved.providers.contains_key(&profile.name) {
            return Err(format!(
                "a profile named '{}' already exists — edit .agents/config.toml directly to change it",
                profile.name
            ));
        }

        let config_path = self.project_root.join(".agents").join("config.toml");
        write_provider_toml(&config_path, &profile).map_err(|e| e.to_string())?;

        let keys_path = config::global_keys_path()
            .ok_or_else(|| "no home directory found for the keys file".to_string())?;
        write_key_entry(&keys_path, &profile.name, &profile.api_key).map_err(|e| e.to_string())?;

        Ok(())
    }
}

/// Insert/replace `[providers.<name>]` (plus an empty
/// `[providers.<name>.models."<model_id>"]`, so the wizard's model id is
/// known to the picker fix in `emberly-tui`) in the project config at `path`.
/// Reserializes the whole file (Tech Spec §12: plain `toml` crate, no
/// `toml_edit` — comments/formatting elsewhere in the file, and table/key
/// order, are not guaranteed to survive; the accepted tradeoff).
fn write_provider_toml(path: &Path, profile: &NewProviderProfile) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut root: toml::Table = if text.trim().is_empty() {
        toml::Table::new()
    } else {
        text.parse().context("parsing .agents/config.toml")?
    };

    // Matches `builtin_profiles()`'s convention: anthropic's native scheme is
    // `x-api-key`, OpenAI-compatible (including third-party/local) is `bearer`.
    let scheme = match profile.adapter.as_str() {
        "anthropic" => "x-api-key",
        _ => "bearer",
    };
    let mut auth = toml::Table::new();
    auth.insert("scheme".into(), scheme.into());
    auth.insert("key".into(), profile.name.clone().into());

    let mut models = toml::Table::new();
    models.insert(
        profile.model_id.clone(),
        toml::Value::Table(toml::Table::new()),
    );

    let mut table = toml::Table::new();
    table.insert("adapter".into(), profile.adapter.clone().into());
    if let Some(base_url) = profile.base_url.as_ref().filter(|s| !s.is_empty()) {
        table.insert("base_url".into(), base_url.clone().into());
    }
    table.insert("auth".into(), toml::Value::Table(auth));
    table.insert("models".into(), toml::Value::Table(models));

    let providers = root
        .entry("providers")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(providers_table) = providers.as_table_mut() else {
        anyhow::bail!(".agents/config.toml has a `providers` key that is not a table");
    };
    providers_table.insert(profile.name.clone(), toml::Value::Table(table));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let out = toml::to_string(&root).context("serializing .agents/config.toml")?;
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}

/// Read-merge-write one `reference = "secret"` entry into the keys file,
/// creating it at `0600` if absent (Tech Spec §8: same enforcement as the
/// read path). Uses `OpenOptions` with the mode set at creation, not a
/// write-then-chmod, so there is no window where a fresh file is
/// group/world-readable.
fn write_key_entry(path: &Path, reference: &str, secret: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut keys: std::collections::HashMap<String, String> = if path.exists() {
        config::enforce_private_permissions(path)?;
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).context("parsing keys.toml")?
    } else {
        std::collections::HashMap::new()
    };
    keys.insert(reference.to_string(), secret.to_string());
    let out = toml::to_string(&keys).context("serializing keys.toml")?;
    write_private(path, &out)
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("emberly-provwrite-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> NewProviderProfile {
        NewProviderProfile {
            name: "deepseek".to_string(),
            adapter: "openai".to_string(),
            base_url: Some("https://api.deepseek.com/v1".to_string()),
            model_id: "deepseek-chat".to_string(),
            api_key: "sk-test-secret".to_string(),
        }
    }

    #[test]
    fn writes_a_fresh_provider_toml_when_none_exists() {
        let dir = tmp();
        let path = dir.join("config.toml");
        write_provider_toml(&path, &sample()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        let deepseek = table["providers"]["deepseek"].as_table().unwrap();
        assert_eq!(deepseek["adapter"].as_str(), Some("openai"));
        assert_eq!(
            deepseek["base_url"].as_str(),
            Some("https://api.deepseek.com/v1")
        );
        assert_eq!(deepseek["auth"]["scheme"].as_str(), Some("bearer"));
        assert_eq!(deepseek["auth"]["key"].as_str(), Some("deepseek"));
        assert!(deepseek["models"]
            .as_table()
            .unwrap()
            .contains_key("deepseek-chat"));
    }

    #[test]
    fn preserves_other_tables_and_existing_profiles() {
        let dir = tmp();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "reasoning = \"expanded\"\n\n[providers.anthropic]\nadapter = \"anthropic\"\n",
        )
        .unwrap();
        write_provider_toml(&path, &sample()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(table["reasoning"].as_str(), Some("expanded"));
        assert!(table["providers"]
            .as_table()
            .unwrap()
            .contains_key("anthropic"));
        assert!(table["providers"]
            .as_table()
            .unwrap()
            .contains_key("deepseek"));
    }

    #[test]
    fn anthropic_adapter_gets_x_api_key_scheme() {
        let dir = tmp();
        let path = dir.join("config.toml");
        let mut profile = sample();
        profile.adapter = "anthropic".to_string();
        write_provider_toml(&path, &profile).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(
            table["providers"]["deepseek"]["auth"]["scheme"].as_str(),
            Some("x-api-key")
        );
    }

    #[test]
    fn writes_key_at_0600_and_preserves_other_entries() {
        let dir = tmp();
        let path = dir.join("keys.toml");
        write_key_entry(&path, "openai", "sk-openai").unwrap();
        write_key_entry(&path, "deepseek", "sk-deepseek").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let keys: std::collections::HashMap<String, String> = toml::from_str(&text).unwrap();
        assert_eq!(keys.get("openai").map(String::as_str), Some("sk-openai"));
        assert_eq!(
            keys.get("deepseek").map(String::as_str),
            Some("sk-deepseek")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn overwrites_an_existing_entry_for_the_same_reference() {
        let dir = tmp();
        let path = dir.join("keys.toml");
        write_key_entry(&path, "deepseek", "sk-old").unwrap();
        write_key_entry(&path, "deepseek", "sk-new").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let keys: std::collections::HashMap<String, String> = toml::from_str(&text).unwrap();
        assert_eq!(keys.get("deepseek").map(String::as_str), Some("sk-new"));
    }
}
