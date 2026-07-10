//! Provider selection and the pure-Rust HTTPS client (Phase 3 groups 6–7).
//!
//! The binary is the composition root: it installs the **pure-Rust** RustCrypto
//! rustls provider (HC-2 — no C toolchain needed to build) and constructs the
//! `reqwest::Client` that the provider clients borrow.
//!
//! Configuration comes from [`crate::config`] (config files + `EMBERLY_*`
//! overrides + `keys.toml`). When no provider is configured, the caller falls
//! back to the offline placeholder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context};
use emberly_providers::{AnthropicProvider, Auth, ModelInfo, OpenAiProvider, Pricing, Provider};

use crate::config::{self, AuthFile, CliOverrides, ProfileFile, Resolved};

/// A chosen live provider plus display/label info.
pub struct Selection {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub label: String,
}

/// Build a live provider by resolving the active profile (Tech Spec §4.5), or
/// `None` to fall back to the offline placeholder (when no profile is active).
/// Adding a provider that reuses an existing adapter is config only — nothing
/// here is vendor-specific (P-8).
pub fn build(resolved: &Resolved) -> anyhow::Result<Option<Selection>> {
    let Some(profile_name) = resolved.provider.clone() else {
        return Ok(None);
    };
    let model = resolved.model.clone().context(
        "a model must be configured when a provider is set (EMBERLY_MODEL, --model, or config.toml)",
    )?;
    let provider = build_profile(&resolved.providers, &profile_name, &model)?;
    Ok(Some(Selection {
        provider,
        label: format!("{profile_name}/{model}"),
        model,
    }))
}

/// Build a provider from a single profile + model — the shared resolution used
/// by [`build`] and by the in-session [`ProviderFactory`](emberly_core::ProviderFactory).
/// Adding a provider that reuses an existing adapter is config only (P-8).
fn build_profile(
    providers: &HashMap<String, ProfileFile>,
    profile_name: &str,
    model: &str,
) -> anyhow::Result<Arc<dyn Provider>> {
    let profile = providers.get(profile_name).ok_or_else(|| {
        let mut known: Vec<&String> = providers.keys().collect();
        known.sort();
        anyhow!(
            "unknown provider profile '{profile_name}' (configured: {})",
            known
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let adapter = profile.adapter.as_deref().ok_or_else(|| {
        anyhow!("provider profile '{profile_name}' has no `adapter` (expected \"anthropic\" or \"openai\")")
    })?;

    let auth = resolve_auth(profile.auth.as_ref(), profile_name)?;
    let meta = profile.models.get(model);
    let model_info = ModelInfo {
        model: model.to_string(),
        context_window: meta.and_then(|m| m.context_window).unwrap_or(200_000),
        max_output_tokens: meta.and_then(|m| m.max_output).unwrap_or(4_096),
        pricing: meta.and_then(|m| m.pricing).map(|p| Pricing {
            input_per_mtok: p.input,
            output_per_mtok: p.output,
        }),
    };
    let client = build_https_client()?;

    let provider: Arc<dyn Provider> = match adapter {
        "anthropic" => match &profile.base_url {
            Some(base) => Arc::new(AnthropicProvider::new(
                client,
                auth,
                base.clone(),
                model_info,
            )),
            None => Arc::new(AnthropicProvider::with_default_url(
                client, auth, model_info,
            )),
        },
        "openai" => {
            let base = profile
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            Arc::new(OpenAiProvider::new(client, auth, base, model_info))
        }
        other => bail!(
            "unknown adapter '{other}' in profile '{profile_name}' \
             (expected \"anthropic\" or \"openai\")"
        ),
    };
    Ok(provider)
}

/// A [`ProviderFactory`](emberly_core::ProviderFactory) over the resolved
/// profiles, so the engine can switch models in-session (C-6) without
/// depending on config/wiring. Holds the merged profile map (cheap to clone).
pub struct ConfiguredProviders {
    providers: HashMap<String, ProfileFile>,
}

impl ConfiguredProviders {
    #[must_use]
    pub fn new(resolved: &Resolved) -> Self {
        Self {
            providers: resolved.providers.clone(),
        }
    }
}

impl emberly_core::ProviderFactory for ConfiguredProviders {
    fn build(&self, profile: &str, model: &str) -> Result<emberly_core::ProviderChoice, String> {
        let provider = build_profile(&self.providers, profile, model).map_err(|e| e.to_string())?;
        Ok(emberly_core::ProviderChoice {
            provider,
            profile: profile.to_string(),
            model: model.to_string(),
        })
    }

    fn profiles(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }
}

/// A [`ConfigReloader`](emberly_core::ConfigReloader) that re-runs
/// [`config::load`] for the project on an in-app edit (C-5), so `/config` and
/// `/prompt` take effect on the running session. Holds the launch-time
/// `sandbox.require` to detect a restart-only change.
pub struct ConfiguredReloader {
    project_root: PathBuf,
    provider: Option<String>,
    model: Option<String>,
    launch_sandbox_require: bool,
}

impl ConfiguredReloader {
    #[must_use]
    pub fn new(project_root: PathBuf, cli: &CliOverrides, launch_sandbox_require: bool) -> Self {
        Self {
            project_root,
            provider: cli.provider.clone(),
            model: cli.model.clone(),
            launch_sandbox_require,
        }
    }
}

impl emberly_core::ConfigReloader for ConfiguredReloader {
    fn reload(&self) -> Result<emberly_core::ReloadedConfig, String> {
        let cli = CliOverrides {
            provider: self.provider.clone(),
            model: self.model.clone(),
        };
        let resolved = config::load(&self.project_root, &cli).map_err(|e| e.to_string())?;
        let mut profiles: Vec<String> = resolved.providers.keys().cloned().collect();
        profiles.sort();
        let mut restart_notes = Vec::new();
        if resolved.sandbox_require != self.launch_sandbox_require {
            restart_notes.push("sandbox.require changed — restart to apply".to_string());
        }
        Ok(emberly_core::ReloadedConfig {
            system: resolved.system_prompt.clone(),
            summary_prompt: resolved.summary_prompt.clone(),
            provider_factory: Arc::new(ConfiguredProviders::new(&resolved)),
            profiles,
            restart_notes,
        })
    }
}

/// Turn a profile's `auth` config into an [`Auth`], resolving the key
/// *reference* from env / `keys.toml`. A configured key reference that does not
/// resolve is a clear early error rather than a silent 401 later.
fn resolve_auth(auth: Option<&AuthFile>, profile: &str) -> anyhow::Result<Auth> {
    let Some(auth) = auth else {
        return Ok(Auth::None);
    };
    let scheme = auth.scheme.as_deref().unwrap_or("none");
    let key = match &auth.key {
        Some(reference) => crate::config::api_key(reference)?.ok_or_else(|| {
            anyhow!(
                "profile '{profile}' needs the '{reference}' key, but none is set \
                 (export {}_API_KEY or add `{reference} = \"…\"` to keys.toml)",
                reference.to_ascii_uppercase()
            )
        })?,
        None => String::new(),
    };
    Ok(match scheme {
        "none" => Auth::None,
        "bearer" => Auth::Bearer(key),
        "x-api-key" => Auth::XApiKey(key),
        "header" => Auth::Header {
            name: auth.header.clone().unwrap_or_default(),
            value: key,
        },
        other => bail!(
            "unknown auth scheme '{other}' in profile '{profile}' \
             (expected bearer, x-api-key, header, or none)"
        ),
    })
}

/// Build an HTTPS-capable client backed by the pure-Rust crypto provider.
fn build_https_client() -> anyhow::Result<reqwest::Client> {
    // Install the RustCrypto provider as the process default (idempotent — a
    // second call returns Err, which we ignore). reqwest's rustls integration,
    // built with the `*-no-provider` feature, uses this default.
    let _ = rustls_rustcrypto::provider().install_default();
    reqwest::Client::builder()
        .build()
        .context("failed to build the HTTPS client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelFile;

    fn profile(adapter: &str, base_url: Option<&str>) -> ProfileFile {
        ProfileFile {
            adapter: Some(adapter.to_string()),
            base_url: base_url.map(str::to_string),
            auth: None, // Auth::None → no key needed, keeps the test hermetic.
            models: HashMap::new(),
        }
    }

    /// The P-8 property, as a unit test: which client is built is decided purely
    /// by the profile's `adapter` — no vendor branching, no network.
    #[test]
    fn build_profile_selects_adapter_from_config_only() {
        let mut providers = HashMap::new();
        providers.insert("a".to_string(), profile("anthropic", None));
        providers.insert(
            "o".to_string(),
            profile("openai", Some("http://localhost:0/v1")),
        );

        let anth = build_profile(&providers, "a", "m1").expect("anthropic builds");
        assert_eq!(anth.id().to_string(), "anthropic");
        assert_eq!(anth.model_info().model, "m1");

        let oai = build_profile(&providers, "o", "m2").expect("openai builds");
        assert_eq!(oai.id().to_string(), "openai-compat");
        assert_eq!(oai.model_info().model, "m2");
    }

    #[test]
    fn build_profile_errors_are_clear() {
        // `Arc<dyn Provider>` isn't Debug, so extract the error message by hand.
        fn err(result: anyhow::Result<Arc<dyn Provider>>) -> String {
            match result {
                Ok(_) => panic!("expected an error"),
                Err(e) => e.to_string(),
            }
        }

        let empty = HashMap::new();
        assert!(err(build_profile(&empty, "nope", "m")).contains("unknown provider profile"));

        let mut weird = HashMap::new();
        weird.insert("x".to_string(), profile("weird", None));
        assert!(err(build_profile(&weird, "x", "m")).contains("unknown adapter"));

        let mut no_adapter = HashMap::new();
        no_adapter.insert("y".to_string(), ProfileFile::default());
        assert!(err(build_profile(&no_adapter, "y", "m")).contains("no `adapter`"));
    }

    #[test]
    fn per_model_metadata_feeds_model_info() {
        let mut prof = profile("openai", Some("http://localhost:0/v1"));
        prof.models.insert(
            "m".to_string(),
            ModelFile {
                context_window: Some(123_456),
                max_output: Some(4_321),
                pricing: None,
            },
        );
        let mut providers = HashMap::new();
        providers.insert("p".to_string(), prof);
        let info = build_profile(&providers, "p", "m")
            .expect("builds")
            .model_info();
        assert_eq!(info.context_window, 123_456);
        assert_eq!(info.max_output_tokens, 4_321);
    }
}
