//! Provider selection and the pure-Rust HTTPS client (Phase 3 groups 6–7).
//!
//! The binary is the composition root: it installs the **pure-Rust** RustCrypto
//! rustls provider (HC-2 — no C toolchain needed to build) and constructs the
//! `reqwest::Client` that the provider clients borrow.
//!
//! Configuration is env-based for now (the full two-tier config file + secrets
//! handling is the rest of group 6 / Phase 5):
//! - `EMBERLY_PROVIDER` = `anthropic` | `openai` (unset → offline placeholder)
//! - `EMBERLY_MODEL`    = model id (required when a provider is set)
//! - `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`
//! - `EMBERLY_BASE_URL` (OpenAI-compatible base, default the public API)
//! - `EMBERLY_CONTEXT_WINDOW` / `EMBERLY_MAX_OUTPUT` (optional)

use std::sync::Arc;

use anyhow::{bail, Context};
use emberly_providers::{AnthropicProvider, ModelInfo, OpenAiProvider, Provider};

/// A chosen live provider plus display/label info.
pub struct Selection {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub label: String,
}

/// Select a live provider from the environment, or `None` to fall back to the
/// offline placeholder.
pub fn select_provider() -> anyhow::Result<Option<Selection>> {
    let Ok(kind) = std::env::var("EMBERLY_PROVIDER") else {
        return Ok(None);
    };
    let model = std::env::var("EMBERLY_MODEL")
        .context("EMBERLY_MODEL must be set when EMBERLY_PROVIDER is set")?;
    let model_info = ModelInfo {
        model: model.clone(),
        context_window: env_u32("EMBERLY_CONTEXT_WINDOW").unwrap_or(200_000),
        max_output_tokens: env_u32("EMBERLY_MAX_OUTPUT").unwrap_or(4_096),
        pricing: None,
    };
    let client = build_https_client()?;

    let provider: Arc<dyn Provider> = match kind.as_str() {
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .context("ANTHROPIC_API_KEY must be set for the anthropic provider")?;
            Arc::new(AnthropicProvider::with_default_url(client, key, model_info))
        }
        "openai" | "openai-compat" => {
            // Key may be empty for local servers (Ollama/vLLM).
            let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
            let base = std::env::var("EMBERLY_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
            Arc::new(OpenAiProvider::new(client, key, base, model_info))
        }
        other => bail!("unknown EMBERLY_PROVIDER '{other}' (expected 'anthropic' or 'openai')"),
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

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}
