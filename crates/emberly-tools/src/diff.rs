//! First-party diff helpers over `similar` (Tech Spec §9, §12): line-delta
//! counts for `FileModified`, unified diffs for edit/write permission prompts
//! (Design §5), and the closest-region hint for a failed edit (T-3 — model
//! recovery quality depends on this).

use similar::{ChangeTag, TextDiff};

/// Count inserted and deleted lines between `old` and `new` — the `(adds,
/// dels)` for a [`FileChange`](crate::tool::FileChange).
#[must_use]
pub fn line_deltas(old: &str, new: &str) -> (u32, u32) {
    let diff = TextDiff::from_lines(old, new);
    let mut adds = 0u32;
    let mut dels = 0u32;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => adds = adds.saturating_add(1),
            ChangeTag::Delete => dels = dels.saturating_add(1),
            ChangeTag::Equal => {}
        }
    }
    (adds, dels)
}

/// A unified diff for a permission prompt or inline display.
#[must_use]
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut unified = diff.unified_diff();
    unified.context_radius(3);
    unified.header(&format!("a/{path}"), &format!("b/{path}"));
    unified.to_string()
}

/// When an exact edit match fails, point at the file line most similar to the
/// target so the model can adjust (T-3). Returns `None` if nothing is close.
#[must_use]
pub fn closest_region_hint(haystack: &str, needle: &str) -> Option<String> {
    let needle_line = needle
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(needle);

    let mut best: Option<(f32, usize, String)> = None;
    for (index, line) in haystack.lines().enumerate() {
        let ratio = TextDiff::from_chars(line, needle_line).ratio();
        let better = best
            .as_ref()
            .is_none_or(|(best_ratio, _, _)| ratio > *best_ratio);
        if better {
            best = Some((ratio, index + 1, line.to_string()));
        }
    }

    best.and_then(|(ratio, line_no, line)| {
        (ratio > 0.5).then(|| format!("closest existing line is {line_no}: {line:?}"))
    })
}
