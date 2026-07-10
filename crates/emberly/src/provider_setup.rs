//! Provider selection and the pure-Rust HTTPS client (Phase 3 groups 6–7).
//!
//! The binary is the composition root: it installs the **pure-Rust** RustCrypto
//! rustls provider (HC-2 — no C toolchain needed to build) and constructs the
//! `reqwest::Client` that the provider clients borrow.
//!
//! Configuration comes from [`crate::config`] (config files + `EMBERLY_*`
//! overrides + `keys.toml`). When no provider is configured, the caller falls
//! back to the offline placeholder.

use std::sync::Arc;

use anyhow::{bail, Context};
use emberly_providers::{AnthropicProvider, Auth, ModelInfo, OpenAiProvider, Provider};

use crate::config::Resolved;

/// A chosen live provider plus display/label info.
pub struct Selection {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub label: String,
}

/// Build a live provider from resolved config, or `None` to fall back to the
/// offline placeholder (when no provider is configured).
pub fn build(resolved: &Resolved) -> anyhow::Result<Option<Selection>> {
    let Some(kind) = resolved.provider.clone() else {
        return Ok(None);
    };
    let model = resolved.model.clone().context(
        "a model must be configured when a provider is set (EMBERLY_MODEL or config.toml)",
    )?;
    let model_info = ModelInfo {
        model: model.clone(),
        context_window: resolved.context_window.unwrap_or(200_000),
        max_output_tokens: resolved.max_output.unwrap_or(4_096),
        pricing: resolved.pricing,
    };
    let client = build_https_client()?;

    let provider: Arc<dyn Provider> = match kind.as_str() {
        "anthropic" => {
            let key = crate::config::api_key("anthropic")?
                .context("no Anthropic API key (set ANTHROPIC_API_KEY or keys.toml)")?;
            Arc::new(AnthropicProvider::with_default_url(
                client,
                Auth::XApiKey(key),
                model_info,
            ))
        }
        "openai" | "openai-compat" => {
            // Key may be empty for local servers (Ollama/vLLM).
            let key = crate::config::api_key("openai")?.unwrap_or_default();
            let base = resolved
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            Arc::new(OpenAiProvider::new(client, Auth::Bearer(key), base, model_info))
        }
        other => bail!("unknown provider '{other}' (expected 'anthropic' or 'openai')"),
    };

    Ok(Some(Selection {
        provider,
        label: format!("{kind}/{model}"),
        model,
    }))
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
