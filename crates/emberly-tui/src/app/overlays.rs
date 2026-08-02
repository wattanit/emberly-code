//! The overlay stack: opening one, pushing it, and reading the active
//! one.
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// Open the diff overlay for the most-recently-modified file, if any.
    pub fn open_last_diff(&mut self) {
        if let Some(path) = self.files.last.clone() {
            if let Some(unified) = self.files.diffs.get(&path) {
                let title = format!("diff: {path}");
                self.push_overlay(Overlay {
                    title,
                    content: OverlayContent::Diff(unified.clone()),
                    scroll: 0,
                });
            }
        }
    }

    /// Push an arbitrary text overlay (help, untruncated output — group 8).
    pub fn open_text_overlay(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push_overlay(Overlay {
            title: title.into(),
            content: OverlayContent::Text(body.into()),
            scroll: 0,
        });
    }

    /// Push an overlay and start its brief ease-in (Design §6.4).
    pub(super) fn push_overlay(&mut self, overlay: Overlay) {
        self.overlays.push(overlay);
        if self.anim.active {
            self.anim.overlay_ease = EASE_FRAMES;
        }
    }

    /// The overlay on top, if any (read by the renderer).
    #[must_use]
    pub fn active_overlay(&self) -> Option<&Overlay> {
        self.overlays.last()
    }
}
