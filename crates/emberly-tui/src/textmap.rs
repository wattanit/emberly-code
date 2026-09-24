//! Per-frame conversation-pane text, for hit-testing a drag selection against
//! the text actually on screen (Design §3.4/§8.12, Tech Spec §9 — 0.5.3).
//!
//! A sibling to [`crate::hit::HitMap`] — same "rebuilt every frame from the
//! real render geometry, one source of truth" idea — but a different query
//! shape: `HitMap` resolves a point to a whole-row `ClickTarget`; this
//! resolves a point (or a span between two points) to the underlying plain
//! text at that screen cell. [`crate::render::frame`] populates it only while
//! the conversation pane is actually drawn (never during a permission/
//! overlay/wizard screen, which have no conversation pane to select from —
//! selecting overlay content is deferred, Tech Spec §16).

use ratatui::layout::Rect;

use crate::text;

/// The conversation pane's rendered text as last drawn: one plain-text row
/// per screen row inside `area`.
#[derive(Debug, Default, Clone)]
pub struct TextMap {
    area: Rect,
    rows: Vec<String>,
}

impl TextMap {
    /// An empty map (a fresh one is built each frame; stays empty whenever the
    /// conversation pane isn't the thing on screen this frame).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the map with this frame's pane geometry and its visible rows'
    /// plain text, top to bottom.
    pub fn set(&mut self, area: Rect, rows: Vec<String>) {
        self.area = area;
        self.rows = rows;
    }

    /// The text strictly between two absolute screen points, order-independent
    /// (a drag can move in any direction) and joined with `\n` across rows.
    /// Points outside the pane clamp to its nearest edge/row, so a drag that
    /// strays outside the pane while still held resolves sensibly rather than
    /// resolving to nothing. Empty (no selectable text, or the map is empty
    /// because no conversation pane was drawn this frame) when there is
    /// nothing to select — Design §8.12: nothing to copy, nothing claimed.
    #[must_use]
    pub fn text_between(&self, a: (u16, u16), b: (u16, u16)) -> String {
        if self.rows.is_empty() {
            return String::new();
        }
        let last_row = self.rows.len() - 1;
        let clamp_row = |row: u16| -> usize {
            if row < self.area.y {
                0
            } else {
                usize::from(row - self.area.y).min(last_row)
            }
        };
        let clamp_col = |col: u16, row_idx: usize| -> usize {
            let w = text::width(&self.rows[row_idx]);
            if col < self.area.x {
                0
            } else {
                usize::from(col - self.area.x).min(w)
            }
        };

        let (mut a_row, mut a_raw_col) = (clamp_row(a.1), a.0);
        let (mut b_row, mut b_raw_col) = (clamp_row(b.1), b.0);
        if (a_row, a_raw_col) > (b_row, b_raw_col) {
            std::mem::swap(&mut a_row, &mut b_row);
            std::mem::swap(&mut a_raw_col, &mut b_raw_col);
        }
        let a_col = clamp_col(a_raw_col, a_row);
        let b_col = clamp_col(b_raw_col, b_row);

        if a_row == b_row {
            return text::slice_cols(&self.rows[a_row], a_col, b_col.saturating_sub(a_col));
        }
        let mut out = String::new();
        let first_w = text::width(&self.rows[a_row]);
        out.push_str(&text::slice_cols(
            &self.rows[a_row],
            a_col,
            first_w.saturating_sub(a_col),
        ));
        for row in &self.rows[a_row + 1..b_row] {
            out.push('\n');
            out.push_str(row);
        }
        out.push('\n');
        out.push_str(&text::slice_cols(&self.rows[b_row], 0, b_col));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn map(area: Rect, rows: &[&str]) -> TextMap {
        let mut m = TextMap::new();
        m.set(area, rows.iter().map(|s| (*s).to_string()).collect());
        m
    }

    #[test]
    fn empty_map_resolves_to_empty_text() {
        let m = TextMap::new();
        assert_eq!(m.text_between((0, 0), (5, 0)), "");
    }

    #[test]
    fn single_row_selection_slices_between_columns() {
        let m = map(rect(0, 0, 20, 3), &["hello world"]);
        assert_eq!(m.text_between((0, 0), (5, 0)), "hello");
    }

    #[test]
    fn selection_is_order_independent() {
        let m = map(rect(0, 0, 20, 3), &["hello world"]);
        assert_eq!(m.text_between((5, 0), (0, 0)), "hello");
    }

    #[test]
    fn multi_row_selection_joins_with_newlines() {
        let m = map(rect(0, 0, 20, 3), &["first line", "second", "third line"]);
        assert_eq!(m.text_between((6, 0), (3, 2)), "line\nsecond\nthi");
    }

    #[test]
    fn points_outside_the_pane_clamp_to_the_nearest_edge() {
        let m = map(rect(2, 1, 20, 3), &["hello world"]);
        // Above/left of the pane clamps to row 0, col 0; (7, 1) is local
        // column 5 (area.x = 2), so the span is the first 5 characters.
        assert_eq!(m.text_between((0, 0), (7, 1)), "hello");
        // Past the end of the row clamps to its actual width, not padding.
        assert_eq!(m.text_between((2, 1), (999, 1)), "hello world");
    }

    #[test]
    fn thai_selection_is_grapheme_correct() {
        // "ที่" is one grapheme cluster / one display column (see text.rs).
        let m = map(rect(0, 0, 20, 1), &["ที่ไทย"]);
        assert_eq!(m.text_between((0, 0), (1, 0)), "ที่");
    }
}
