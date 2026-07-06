//! Grapheme- and width-correct text measurement (Tech Spec §9, Requirements
//! §2.1). Terminal layout must never assume one `char` == one column: Thai
//! combining vowel/tone marks are zero-width and stack onto a base cluster, and
//! CJK glyphs are two columns wide. Every width/wrap/cursor calculation in the
//! TUI goes through this module — **never** `str::len`, **never** chars-as-
//! columns.
//!
//! "Cluster" here means an *extended grapheme cluster* (`unicode-segmentation`
//! with `is_extended = true`): a base character plus any combining marks that
//! visually attach to it. A cursor position is a byte offset that always sits
//! on a cluster boundary.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The display width of `s` in terminal columns, summing per-cluster widths.
/// Combining marks contribute 0; wide (CJK) clusters contribute 2.
#[must_use]
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// The extended grapheme clusters of `s`, in order.
pub fn clusters(s: &str) -> impl Iterator<Item = &str> {
    s.graphemes(true)
}

/// The number of grapheme clusters (not chars, not bytes) in `s`.
#[must_use]
pub fn cluster_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// The byte offset of the cluster boundary at or before `byte`, moving one
/// cluster left. Returns 0 at the start. `byte` must be a cluster boundary.
#[must_use]
pub fn prev_boundary(s: &str, byte: usize) -> usize {
    if byte == 0 {
        return 0;
    }
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .take_while(|&i| i < byte)
        .last()
        .unwrap_or(0)
}

/// The byte offset of the next cluster boundary after `byte`. Returns `s.len()`
/// at the end. `byte` must be a cluster boundary.
#[must_use]
pub fn next_boundary(s: &str, byte: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i > byte)
        .unwrap_or(s.len())
}

/// The display column of the byte offset `byte` within `s` (i.e. the width of
/// everything before it). `byte` must be a cluster boundary.
#[must_use]
pub fn col_at(s: &str, byte: usize) -> usize {
    width(&s[..byte])
}

/// The byte offset in `s` at (or just past) display column `target_col`. Walks
/// clusters accumulating width; stops at the first boundary whose column
/// reaches `target_col`. Used to place the cursor when moving between lines of
/// different content.
#[must_use]
pub fn byte_at_col(s: &str, target_col: usize) -> usize {
    let mut col = 0;
    for (i, g) in s.grapheme_indices(true) {
        if col >= target_col {
            return i;
        }
        col += width(g);
    }
    s.len()
}

/// Whether a cluster is whitespace (used for word-wise motion).
#[must_use]
pub fn is_space(cluster: &str) -> bool {
    !cluster.is_empty() && cluster.chars().all(char::is_whitespace)
}

/// Word-aware wrap of `s` to `max` columns. Explicit newlines are honored;
/// long words with no break opportunity are hard-broken at cluster boundaries
/// so a single wide token can never overflow. Blank lines are preserved.
#[must_use]
pub fn wrap(s: &str, max: usize) -> Vec<String> {
    if max == 0 {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    for logical in s.split('\n') {
        wrap_line(logical, max, &mut out);
    }
    out
}

fn wrap_line(line: &str, max: usize, out: &mut Vec<String>) {
    let mut cur = String::new();
    let mut cur_w = 0usize;
    // Byte offset in `cur` of the most recent space, for word-break rollback.
    let mut last_space: Option<usize> = None;

    for g in line.graphemes(true) {
        let gw = width(g);
        if cur_w + gw > max && !cur.is_empty() {
            match last_space {
                Some(sb) => {
                    let tail = cur.split_off(sb);
                    out.push(cur.trim_end().to_string());
                    cur = tail.trim_start_matches(' ').to_string();
                    cur_w = width(&cur);
                    last_space = None;
                }
                None => {
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
            }
        }
        if g == " " {
            last_space = Some(cur.len());
        }
        cur.push_str(g);
        cur_w += gw;
    }
    out.push(cur);
}

/// A horizontally-scrolled slice of `s` covering display columns
/// `[start_col, start_col + width)`, cluster-aligned. Clusters that would
/// straddle either edge are dropped so the slice never splits a cluster or
/// overflows the window. For rendering a long line that is scrolled under a
/// fixed-width viewport (the input editor, overlays).
#[must_use]
pub fn slice_cols(s: &str, start_col: usize, win: usize) -> String {
    let end_col = start_col + win;
    let mut out = String::new();
    let mut col = 0;
    for g in s.graphemes(true) {
        let gw = width(g);
        if col + gw <= start_col {
            col += gw;
            continue;
        }
        if col >= end_col || col + gw > end_col {
            break;
        }
        out.push_str(g);
        col += gw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Thai fixtures with stacked combining marks (§14 correctness anchors).
    // "ที่" = ท (U+0E17) + sara ii (U+0E35, combining) + mai ek (U+0E48,
    //   combining) — 3 chars, ONE grapheme cluster, ONE display column.
    const THAI_STACKED: &str = "ที่";
    // "ไทย" = sara ai (U+0E44, spacing) + ท + ย — 3 clusters, 3 columns.
    const THAI_WORD: &str = "ไทย";

    #[test]
    fn stacked_marks_are_one_cluster_one_column() {
        assert_eq!(THAI_STACKED.chars().count(), 3, "three scalar values");
        assert_eq!(cluster_count(THAI_STACKED), 1, "but one grapheme cluster");
        assert_eq!(width(THAI_STACKED), 1, "and one display column");
    }

    #[test]
    fn spacing_vowels_count_as_columns() {
        assert_eq!(cluster_count(THAI_WORD), 3);
        assert_eq!(width(THAI_WORD), 3);
    }

    #[test]
    fn boundaries_step_over_whole_clusters() {
        // Moving right from 0 jumps the whole stacked cluster (all 9 bytes).
        let end = next_boundary(THAI_STACKED, 0);
        assert_eq!(end, THAI_STACKED.len());
        assert_eq!(prev_boundary(THAI_STACKED, end), 0);
    }

    #[test]
    fn wide_chars_are_two_columns() {
        assert_eq!(width("世界"), 4);
        assert_eq!(cluster_count("世界"), 2);
    }

    #[test]
    fn col_and_byte_round_trip_on_thai() {
        let s = THAI_WORD;
        let b = byte_at_col(s, 2);
        assert_eq!(col_at(s, b), 2);
    }

    #[test]
    fn wrap_breaks_on_words() {
        let out = wrap("hello world foo", 6);
        assert_eq!(out, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn wrap_hard_breaks_overlong_words() {
        let out = wrap("abcdefgh", 3);
        assert_eq!(out, vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrap_preserves_blank_lines() {
        let out = wrap("a\n\nb", 10);
        assert_eq!(out, vec!["a", "", "b"]);
    }

    #[test]
    fn slice_cols_is_cluster_aligned() {
        // Window of 2 columns starting at col 1 over a wide-char string.
        assert_eq!(slice_cols("世界", 0, 2), "世");
        assert_eq!(slice_cols("abcdef", 2, 3), "cde");
    }
}
