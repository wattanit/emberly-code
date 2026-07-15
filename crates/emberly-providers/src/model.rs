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

/// A normalized reasoning-effort level (Requirements P-9, Tech Spec §4.6).
/// Ordered low→max so a UI presents the levels in order. Each adapter maps
/// this to its provider's native control (a thinking-budget token count, a
/// `reasoning_effort` field) or drops it — a model without such a control
/// ignores the setting, which is never an error (P-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    Max,
}

impl Effort {
    /// The levels in ascending order — for a UI that offers "all supported".
    pub const ALL: [Effort; 4] = [Effort::Low, Effort::Medium, Effort::High, Effort::Max];

    /// A stable lowercase token for logs, the transcript, and CLI parsing.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Max => "max",
        }
    }

    /// Parse a level from a lowercase token (CLI/`/effort` arg). `None` for an
    /// unrecognized value — the caller turns that into a calm notice.
    #[must_use]
    pub fn parse(s: &str) -> Option<Effort> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Effort::Low),
            "medium" => Some(Effort::Medium),
            "high" => Some(Effort::High),
            "max" => Some(Effort::Max),
            _ => None,
        }
    }
}

impl std::fmt::Display for Effort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
    /// The reasoning-effort levels this model exposes, ascending (P-9,
    /// Tech Spec §4.6). Empty ⇒ the model has no effort control and the UI
    /// hides the picker. `default`s to empty for older `ModelInfo` values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effort_levels: Vec<Effort>,
    /// The effort level to use when the user has not chosen one. `None` ⇒ send
    /// no effort (the provider's own default). `default`s to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<Effort>,
    /// Whether the model accepts image input (P-11, Tech Spec §4.1). **Default
    /// `false`** so an image is never sent to a model not declared
    /// vision-capable. `default`s to `false` for older `ModelInfo` values.
    #[serde(default)]
    pub vision: bool,
    /// Whether the model accepts document input (P-12, Tech Spec §4.1).
    /// **Default `false`** so a document is never sent to a model not
    /// declared document-capable. `default`s to `false` for older `ModelInfo`
    /// values.
    #[serde(default)]
    pub documents: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn to_json<T: Serialize>(value: &T) -> String {
        match serde_json::to_string(value) {
            Ok(s) => s,
            Err(e) => panic!("serialize: {e}"),
        }
    }

    fn from_json<T: for<'de> Deserialize<'de>>(json: &str) -> T {
        match serde_json::from_str(json) {
            Ok(v) => v,
            Err(e) => panic!("deserialize: {e}"),
        }
    }

    #[test]
    fn effort_round_trips_through_serde_snake_case() {
        for level in Effort::ALL {
            let json = to_json(&level);
            assert_eq!(json, format!("\"{}\"", level.as_str()));
            assert_eq!(from_json::<Effort>(&json), level);
        }
    }

    #[test]
    fn effort_parses_case_insensitively_and_rejects_unknown() {
        assert_eq!(Effort::parse("HIGH"), Some(Effort::High));
        assert_eq!(Effort::parse("  max "), Some(Effort::Max));
        assert_eq!(Effort::parse("turbo"), None);
    }

    #[test]
    fn effort_is_ordered_low_to_max() {
        assert!(Effort::Low < Effort::Medium);
        assert!(Effort::Medium < Effort::High);
        assert!(Effort::High < Effort::Max);
    }

    #[test]
    fn model_info_without_effort_fields_deserializes_to_empty() {
        // A ModelInfo serialized before Phase 3 has no effort keys.
        let json = r#"{"model":"m","context_window":1000,"max_output_tokens":100}"#;
        let info: ModelInfo = from_json(json);
        assert!(info.effort_levels.is_empty());
        assert_eq!(info.default_effort, None);
        assert!(!info.vision);
        assert!(!info.documents);
    }

    #[test]
    fn model_info_effort_fields_round_trip() {
        let info = ModelInfo {
            model: "m".into(),
            context_window: 1000,
            max_output_tokens: 100,
            pricing: None,
            effort_levels: vec![Effort::Low, Effort::High],
            default_effort: Some(Effort::Low),
            vision: false,
            documents: false,
        };
        let json = to_json(&info);
        assert_eq!(from_json::<ModelInfo>(&json), info);
    }
}
