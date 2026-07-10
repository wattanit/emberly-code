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
use std::sync::Arc;

use anyhow::{anyhow, bail, Context};
use emberly_providers::{AnthropicProvider, Auth, ModelInfo, OpenAiProvider, Pricing, Provider};

use crate::config::{AuthFile, ProfileFile, Resolved};

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
            Some(base) => Arc::new(AnthropicProvider::new(client, auth, base.clone(), model_info)),
            None => Arc::new(AnthropicProvider::with_default_url(client, auth, model_info)),
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
