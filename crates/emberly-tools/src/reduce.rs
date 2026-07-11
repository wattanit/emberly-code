//! Salient tool-result reduction at ingestion (FR-2, Tech Spec §5.3).
//!
//! Mirrors [`crate::truncate`]: a pure, deterministic transform with no I/O
//! and **no model call** (Requirements §8.5). The engine calls
//! [`reduce_output`] *before* [`truncate_output`] so the order is
//! salient-reduction → size backstop (§5.3).
//!
//! Each tool may register a reducer that keeps the parts that inform the
//! model's next step and elides the noise *by meaning*, not by position. A
//! tool with no registered reducer falls straight through to the size
//! backstop unchanged. The full, pre-reduction output is preserved in the
//! sidecar by the engine (Tech Spec §3.2); reduction only changes what is
//! sent to the model — never the transcript (HC-7).
//!
//! [`truncate_output`]: crate::truncate_output

/// The outcome of reducing one tool result to its salient content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reduction {
    /// The reduced content to place into the conversation view, with any
    /// reduction marker already embedded when [`reduced`](Self::reduced) is
    /// true.
    pub content: String,
    /// Whether any salient reduction occurred.
    pub reduced: bool,
    /// A brief description of what was withheld and why (e.g. "12 repetitive
    /// progress lines collapsed"), for logging and debugging. Empty when not
    /// reduced.
    pub withheld: String,
}

impl Reduction {
    /// No reduction — content unchanged, ready for the size backstop.
    #[must_use]
    fn passthrough(raw: &str) -> Self {
        Self {
            content: raw.to_string(),
            reduced: false,
            withheld: String::new(),
        }
    }
}

/// Reduce a tool result to its salient content by meaning (FR-2, Tech Spec
/// §5.3). Dispatches to a per-tool reducer keyed by tool name; a tool with no
/// registered reducer falls straight through unchanged to the size backstop.
///
/// Pure and deterministic — no I/O, no model call (Requirements §8.5).
#[must_use]
#[allow(clippy::match_single_binding)] // per-tool arms arrive in Group 2
pub fn reduce_output(tool_name: &str, raw: &str) -> Reduction {
    match tool_name {
        // Per-tool reducers arrive in Group 2; until then every tool passes
        // through to the size backstop unchanged.
        _ => Reduction::passthrough(raw),
    }
}

/// Build a reduction marker — distinct in wording from the size-truncation
/// marker ([`crate::truncate`]) so the model can tell *reduced-by-meaning*
/// from *trimmed-by-size*. Names what was withheld and offers `/view` to
/// recover the full output (Design §4.3/§8.6).
#[must_use]
#[allow(dead_code)] // per-tool reducers arrive in Group 2
fn reduction_marker(description: &str) -> String {
    format!("[reduced: {description} — /view for full output]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unregistered_tool_passes_through_unchanged() {
        let raw = "line one\nline two\n";
        let r = reduce_output("read_file", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
        assert!(r.withheld.is_empty());
    }

    #[test]
    fn unknown_tool_passes_through_unchanged() {
        let raw = "some output\n";
        let r = reduce_output("totally_unknown_tool", raw);
        assert!(!r.reduced);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn empty_output_passes_through() {
        let r = reduce_output("bash", "");
        assert!(!r.reduced);
        assert!(r.content.is_empty());
    }

    #[test]
    fn passthrough_content_is_identical() {
        let raw = "exit code: 0\n--- stdout ---\nhello\n";
        let r = reduce_output("bash", raw);
        assert_eq!(r.content, raw);
    }

    #[test]
    fn reduction_marker_is_distinct_from_truncation_marker() {
        let m = reduction_marker("3 progress lines collapsed");
        assert!(m.starts_with("[reduced:"));
        assert!(m.contains("/view"));
        // The truncation marker uses a different prefix.
        assert!(!m.contains("elided"));
    }
}
