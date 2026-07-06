//! Rendering of unified diffs (Design §4.2). Diffs are first-class: we own both
//! sides (the edit tool produced the diff — no external diff binary), so this is
//! pure presentation. Colours follow universal convention — green additions,
//! red deletions — via the theme's semantic roles, with `+`/`-`/`@@` prefixes
//! carrying the meaning so it survives with colour stripped (Design §7).
//!
//! Reused by the inline edit view, the diff overlay, and the edit permission
//! prompt (group 7).

use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Render a unified-diff string into styled rows (one row per diff line; long
/// lines clip at the pane edge rather than reflowing — a wrapped diff misleads).
#[must_use]
pub fn render_unified(unified: &str, theme: &Theme) -> Vec<Line<'static>> {
    unified.lines().map(|l| diff_line(l, theme)).collect()
}

/// Render at most `max` rows of a diff, returning the rows and the number of
/// hidden lines (for an inline "… N more" affordance pointing to the overlay).
#[must_use]
pub fn render_unified_capped(
    unified: &str,
    theme: &Theme,
    max: usize,
) -> (Vec<Line<'static>>, usize) {
    let total = unified.lines().count();
    let rows: Vec<Line<'static>> = unified
        .lines()
        .take(max)
        .map(|l| diff_line(l, theme))
        .collect();
    (rows, total.saturating_sub(max))
}

fn diff_line(line: &str, theme: &Theme) -> Line<'static> {
    // File headers first (they start with --- / +++, which would otherwise be
    // read as deletion/addition).
    let style = if line.starts_with("+++") || line.starts_with("---") {
        theme.chrome()
    } else if line.starts_with("@@") {
        theme.dim_accent()
    } else if line.starts_with('+') {
        theme.diff_add()
    } else if line.starts_with('-') {
        theme.diff_del()
    } else {
        // Context lines: dimmed so the +/- changes stand out.
        theme.chrome()
    };
    Line::from(Span::styled(line.to_string(), style))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 fn main() {
-    old();
+    new();
 }";

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn renders_every_line_once() {
        let out = render_unified(SAMPLE, &Theme::plain());
        assert_eq!(out.len(), 7);
        let t = texts(&out);
        assert_eq!(t[4], "-    old();");
        assert_eq!(t[5], "+    new();");
    }

    #[test]
    fn add_and_delete_use_semantic_colours() {
        let theme = Theme::rich();
        let out = render_unified(SAMPLE, &theme);
        assert_eq!(
            out[5].spans[0].style.fg,
            theme.diff_add().fg,
            "addition green"
        );
        assert_eq!(
            out[4].spans[0].style.fg,
            theme.diff_del().fg,
            "deletion red"
        );
    }

    #[test]
    fn cap_reports_hidden_lines() {
        let (rows, hidden) = render_unified_capped(SAMPLE, &Theme::plain(), 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(hidden, 4);
    }
}
