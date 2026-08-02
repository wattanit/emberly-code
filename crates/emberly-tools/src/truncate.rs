//! Truncation at ingestion (Requirements §8.1, Tech Spec §5.3).
//!
//! When a tool result would be appended to the conversation, oversized output
//! is trimmed to head + tail with the middle elided behind an explicit marker.
//! Deterministic, per-event, **no model call** — this is the first line of
//! context defense, applied to bash output and file reads alike.
//!
//! This function only computes the in-context view. The sidecar file holding
//! the full output and the `full_output_ref` that points at it are written by
//! the transcript layer; here [`Truncation`] simply reports whether
//! truncation happened and the original size.

use crate::ctx::TruncateConfig;

/// The outcome of truncating one tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    /// The content to place into the conversation view (with an elision marker
    /// when truncated).
    pub content: String,
    /// Whether any elision occurred.
    pub truncated: bool,
    /// Line count of the original, pre-truncation output.
    pub original_lines: usize,
    /// Byte length of the original, pre-truncation output.
    pub original_bytes: usize,
}

/// Truncate `output` per `config`, keeping head and tail and eliding the
/// middle. Line-oversized output is trimmed line-wise; if the result is still
/// byte-oversized (e.g. a few enormous lines), a byte-wise pass follows.
#[must_use]
pub fn truncate_output(output: &str, config: &TruncateConfig) -> Truncation {
    let original_lines = output.lines().count();
    let original_bytes = output.len();

    let mut content = output.to_string();
    let mut truncated = false;

    if original_lines > config.max_lines {
        content = elide_lines(&content, config);
        truncated = true;
    }
    if content.len() > config.max_bytes {
        content = elide_bytes(&content, config);
        truncated = true;
    }

    Truncation {
        content,
        truncated,
        original_lines,
        original_bytes,
    }
}

/// Keep `head_lines` from the top and `tail_lines` from the bottom, with a
/// marker naming how many lines were removed.
fn elide_lines(output: &str, config: &TruncateConfig) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let total = lines.len();
    let head = config.head_lines.min(total);
    let tail = config.tail_lines.min(total.saturating_sub(head));
    let elided = total.saturating_sub(head + tail);
    if elided == 0 {
        return output.to_string();
    }

    let mut out = String::new();
    for line in &lines[..head] {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&marker(elided, "lines"));
    out.push('\n');
    for line in &lines[total - tail..] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Keep head and tail byte budgets (split by the head:tail line ratio) with a
/// marker, never splitting a UTF-8 code point.
fn elide_bytes(s: &str, config: &TruncateConfig) -> String {
    let ratio_denom = (config.head_lines + config.tail_lines).max(1);
    let head_budget = config.max_bytes.saturating_mul(config.head_lines) / ratio_denom;
    let tail_budget = config.max_bytes.saturating_sub(head_budget);

    let head_end = floor_char_boundary(s, head_budget);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(tail_budget));
    if tail_start <= head_end {
        // Budgets overlap the whole string; nothing meaningful to elide.
        return s.to_string();
    }

    let elided = tail_start - head_end;
    let mut out = String::new();
    out.push_str(&s[..head_end]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&marker(elided, "bytes"));
    out.push('\n');
    out.push_str(&s[tail_start..]);
    out
}

fn marker(count: usize, unit: &str) -> String {
    format!("[... {count} {unit} elided — /view to open full output ...]")
}

/// Largest char-boundary index `<= idx`.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char-boundary index `>= idx`.
fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TruncateConfig {
        TruncateConfig::default()
    }

    #[test]
    fn short_output_is_untouched() {
        let out = truncate_output("one\ntwo\nthree\n", &config());
        assert!(!out.truncated);
        assert_eq!(out.content, "one\ntwo\nthree\n");
        assert_eq!(out.original_lines, 3);
    }

    #[test]
    fn at_line_limit_is_not_truncated() {
        let text = "x\n".repeat(400); // exactly max_lines
        let out = truncate_output(&text, &config());
        assert!(!out.truncated, "400 lines is at the limit, not over it");
    }

    #[test]
    fn over_line_limit_keeps_head_and_tail() {
        let text = (0..500)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = truncate_output(&text, &config());
        assert!(out.truncated);
        assert_eq!(out.original_lines, 500);
        // 500 - 150 head - 100 tail = 250 elided.
        assert!(out.content.contains("250 lines elided"), "{}", out.content);
        assert!(out.content.contains("/view to open full output"));
        assert!(out.content.starts_with("line0\n"));
        assert!(out.content.contains("line499"));
        assert!(!out.content.contains("line300"), "middle should be gone");
    }

    #[test]
    fn giant_single_line_is_byte_truncated_on_char_boundary() {
        // One line, within the line limit, but far over the byte budget, made
        // of multibyte characters so a naive byte split would panic.
        let mut cfg = config();
        cfg.max_bytes = 100;
        let text = "ก".repeat(500); // 3 bytes each = 1500 bytes, 1 line
        let out = truncate_output(&text, &cfg);
        assert!(out.truncated);
        assert!(out.content.contains("bytes elided"), "{}", out.content);
        // Round-trips as valid UTF-8 by construction (it is a String); assert
        // the surviving pieces are whole Thai characters.
        assert!(out.content.starts_with('ก'));
        assert!(out.content.ends_with('ก'));
    }

    #[test]
    fn reports_original_sizes() {
        let text = "abcd\nefgh\n";
        let out = truncate_output(text, &config());
        assert_eq!(out.original_bytes, text.len());
        assert_eq!(out.original_lines, 2);
    }
}
