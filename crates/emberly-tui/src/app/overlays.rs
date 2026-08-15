//! The overlay stack: opening one, pushing it, and reading the active
//! one.
//!
//! Part of the `App` inherent impl.

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

    /// Push an arbitrary text overlay (help, untruncated output).
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

    /// The id of the subagent whose activity overlay is currently on top, if
    /// any (Design §4.13). What the rich TUI's periodic refresh (`tui::run`)
    /// polls to decide whether to re-issue `Command::InspectAgent` — `None`
    /// means no activity overlay is open, so nothing gets re-fetched.
    #[must_use]
    pub fn watched_agent_id(&self) -> Option<String> {
        match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::AgentActivity { id, .. }) => Some(id.clone()),
            _ => None,
        }
    }
}
