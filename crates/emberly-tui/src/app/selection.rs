//! Drag-to-select-and-copy (Design §3.4/§8.12, Tech Spec §9 — 0.5.3): the
//! selection lifecycle and the "next input clears the last one" rule shared
//! by the copy notice. Entirely frontend-local — the engine has no concept of
//! either (Tech Spec §3.1, corrected v0.17).
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    /// Start tracking a potential selection at an unmodified left-button
    /// `Down` (Design §3.4). This runs *alongside* the existing click dispatch
    /// (`on_click`), never instead of it: a `Down` immediately followed by an
    /// `Up` with no real movement in between must still behave exactly like an
    /// ordinary click (`finish_selection` returns `None` for that case).
    pub fn begin_selection(&mut self, col: u16, row: u16) {
        self.clear_transients();
        self.selection = Some(Selection {
            anchor: (col, row),
            current: (col, row),
            dragging: false,
        });
    }

    /// Extend the in-progress selection to the pointer's new position on an
    /// unmodified `Drag` event. A no-op if no selection is being tracked (e.g.
    /// the preceding `Down` carried a modifier and was left to the terminal).
    pub fn extend_selection(&mut self, col: u16, row: u16) {
        if let Some(sel) = self.selection.as_mut() {
            sel.current = (col, row);
            sel.dragging = true;
        }
    }

    /// Resolve and finish the selection on an unmodified `Up`. Returns the
    /// selected text when a genuine drag resolved to something non-empty — the
    /// caller writes that to the clipboard and arms the copy notice. A plain
    /// click (never dragged) or a drag that resolved to no text (e.g. entirely
    /// outside the pane) returns `None` and clears the selection, so nothing
    /// lingers highlighted for a click and nothing is claimed for an empty
    /// selection (Design §8.12).
    pub fn finish_selection(&mut self) -> Option<String> {
        let sel = self.selection.take()?;
        if !sel.dragging {
            return None;
        }
        let text = self.text_map.text_between(sel.anchor, sel.current);
        if text.is_empty() {
            return None;
        }
        // The highlight stays visible (frozen, not live-updating) until the
        // next input clears it — the user can see what "Copied N characters."
        // refers to.
        self.selection = Some(Selection {
            dragging: false,
            ..sel
        });
        Some(text)
    }

    /// Clear the selection highlight and the copy-flash notice — "the next
    /// input" in Design §8.12's "gone on the next input rather than
    /// lingering." Also correctness-critical for the selection itself: a
    /// scroll or resize changes what text is at which screen row, so a frozen
    /// selection's coordinates would otherwise point at the wrong content.
    pub fn clear_transients(&mut self) {
        self.selection = None;
        self.copy_flash = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn app() -> App {
        App::new(
            SessionInfo::default(),
            PathBuf::new(),
            Vec::new(),
            String::new(),
            String::new(),
            String::new(),
            test_provider_writer(),
        )
    }

    #[test]
    fn a_plain_click_selects_nothing() {
        let mut a = app();
        a.begin_selection(5, 5);
        // No `extend_selection` call — an unmoved Down-then-Up.
        assert_eq!(a.finish_selection(), None);
        assert!(a.selection.is_none());
    }

    #[test]
    fn a_real_drag_with_no_resolvable_text_selects_nothing() {
        let mut a = app();
        // No conversation pane rendered yet (`text_map` is empty), so any
        // drag resolves to empty text regardless of coordinates.
        a.begin_selection(0, 0);
        a.extend_selection(5, 0);
        assert_eq!(a.finish_selection(), None);
        assert!(a.selection.is_none());
    }

    #[test]
    fn a_real_drag_over_rendered_text_is_copied_and_stays_highlighted() {
        let mut a = app();
        a.text_map.set(
            Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 1,
            },
            vec!["hello world".to_string()],
        );
        a.begin_selection(0, 0);
        a.extend_selection(5, 0);
        assert_eq!(a.finish_selection().as_deref(), Some("hello"));
        // The selection is still on record (frozen) so the highlight persists
        // until the next input, per Design §8.12.
        assert!(a.selection.is_some());
    }

    #[test]
    fn clearing_transients_drops_both_selection_and_flash() {
        let mut a = app();
        a.text_map.set(
            Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 1,
            },
            vec!["hello world".to_string()],
        );
        a.begin_selection(0, 0);
        a.extend_selection(5, 0);
        a.finish_selection();
        a.copy_flash = Some(5);
        a.clear_transients();
        assert!(a.selection.is_none());
        assert!(a.copy_flash.is_none());
    }

    #[test]
    fn beginning_a_new_selection_clears_the_previous_copy_flash() {
        let mut a = app();
        a.copy_flash = Some(3);
        a.begin_selection(1, 1);
        assert!(a.copy_flash.is_none());
    }
}
