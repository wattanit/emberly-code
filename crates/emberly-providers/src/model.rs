//! Provider and model description types, plus token/cost accounting units
//! (Requirements P-6, Tech Spec §4.1, §4.4).

use serde::{Deserialize, Serialize};

/// Stable identifier for a provider backend (e.g. `anthropic`,
/// `openai-compat`, `fake`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub String);

impl ProviderId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the engine needs to know about the active model: its window (for the
/// context budget, Requirements §8.4) and optional pricing (for the cost
/// estimate, P-6). Populated from provider defaults and config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: String,
    /// Total context window in tokens.
    pub context_window: u32,
    /// Maximum tokens the model may produce in one completion. Also informs
    /// the reserved-output budget (Tech Spec §7).
    pub max_output_tokens: u32,
    /// Per-model pricing, when known. Cost figures are always labeled "est."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
}

/// Per-model pricing in USD per million tokens (Tech Spec §4.4). Sourced from
/// config; never hardcoded per provider.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl Pricing {
    /// Estimated USD cost for the given usage. Always an estimate (P-6).
    #[must_use]
    pub fn estimate_usd(&self, usage: TokenUsage) -> f64 {
        let per = |tokens: u64, price: f64| (tokens as f64) / 1_000_000.0 * price;
        per(usage.input, self.input_per_mtok) + per(usage.output, self.output_per_mtok)
    }
}

/// Prompt/completion token counts for a completion. Provider-reported when
/// available; estimated otherwise (Tech Spec §4.4). Drives the context
/// indicator and cost estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

impl TokenUsage {
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

/// Result of [`Provider::count_tokens`](crate::Provider::count_tokens). The
/// requirement is a reliable trigger, not exactness, so `approximate` flags a
/// chars/4-style estimate to callers (P-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenEstimate {
    pub tokens: u64,
    pub approximate: bool,
}
