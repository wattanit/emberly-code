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
/// heuristic (chars-per-token, script-weighted — see [`estimate_tokens`])
/// estimate to callers (P-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenEstimate {
    pub tokens: u64,
    pub approximate: bool,
}

/// Unicode ranges dense enough — no space-delimited words, denser BPE token
/// packing — that a flat chars/4 ratio badly underestimates them: Thai, Lao,
/// Myanmar, Khmer, CJK ideographs, Hiragana/Katakana, Hangul (issue #12).
fn is_dense_script(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0E7F   // Thai
        | 0x0E80..=0x0EFF // Lao
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x17FF // Khmer
        | 0x3040..=0x30FF // Hiragana + Katakana
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
    )
}

/// Estimate a text's token count without a real tokenizer (P-6, issue #12).
/// Plain Latin-script text is estimated at ~4 chars/token; text in a script
/// with no space-delimited words and denser BPE packing (Thai, CJK, ...) is
/// estimated at ~2 chars/token instead. Weighted per character rather than
/// classifying the whole string, so mixed-script text (e.g. Thai prose with
/// English terms) isn't miscounted as entirely one script or the other.
/// Still a heuristic, not a real tokenizer — always `approximate`.
#[must_use]
pub(crate) fn estimate_tokens(text: &str) -> TokenEstimate {
    let (dense, plain) = text.chars().fold((0u64, 0u64), |(dense, plain), c| {
        if is_dense_script(c) {
            (dense + 1, plain)
        } else {
            (dense, plain + 1)
        }
    });
    TokenEstimate {
        tokens: dense.div_ceil(2).saturating_add(plain.div_ceil(4)),
        approximate: true,
    }
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

    #[test]
    fn ascii_text_keeps_the_chars_over_4_ratio() {
        let est = estimate_tokens("12345678"); // 8 chars, no dense script
        assert_eq!(est.tokens, 2);
        assert!(est.approximate);
        assert_eq!(estimate_tokens("").tokens, 0);
    }

    #[test]
    fn thai_text_is_estimated_at_roughly_double_the_ascii_rate() {
        // "สวัสดีครับ" ("hello", polite male register) — 10 Thai codepoints,
        // no whitespace word boundaries (issue #12).
        let thai = "สวัสดีครับ";
        assert_eq!(thai.chars().count(), 10);
        assert_eq!(estimate_tokens(thai).tokens, 5); // 10 / 2, not 10 / 4
    }

    #[test]
    fn cjk_and_hangul_use_the_dense_ratio_too() {
        assert_eq!(estimate_tokens("你好世界").tokens, 2); // 4 chars / 2
        assert_eq!(estimate_tokens("안녕하세요").tokens, 3); // 5 chars, div_ceil(2)
    }

    #[test]
    fn mixed_script_text_is_weighted_per_character() {
        // 4 ASCII chars (÷4) + 4 Thai chars (÷2): neither ratio alone applies.
        let mixed = "test ไทย";
        let est = estimate_tokens(mixed);
        // "test " = 5 ASCII/space chars → div_ceil(4) = 2; "ไทย" = 3 Thai chars
        // → div_ceil(2) = 2. Total 4, well above a flat chars/4 of 8/4 = 2.
        assert_eq!(est.tokens, 4);
    }
}
