//! Animation and busy state: what is in flight, what is still easing,
//! and which spinner frame to show (Design §6.3).
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// Whether the model is actively working — drives the spinner and the
    /// streaming accent glow. Off during a permission prompt or a question
    /// prompt, and when motion is disabled (those screens are perfectly still).
    #[must_use]
    pub fn is_working(&self) -> bool {
        self.anim.active && self.anim.busy && !self.is_deciding()
    }

    /// Whether *anything* is animating right now — the working spinner/glow, or
    /// a transient effect (overlay ease-in, sidebar settle). The ticker redraws
    /// only while this is true, so idle screens stay quiet. Always false during
    /// a decision prompt or with motion off.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        if !self.anim.active || self.is_deciding() {
            return false;
        }
        self.anim.busy || self.anim.overlay_ease > 0 || self.anim.sidebar_settle > 0
    }

    /// Whether a decision prompt (permission, question, loop halt, or
    /// completion-gate halt) is open — those screens are perfectly still
    /// (Design §5, §5.1, §6.4, §8.5, §8.7).
    #[must_use]
    pub(super) fn is_deciding(&self) -> bool {
        self.prompts.permission.is_some()
            || self.prompts.ask.is_some()
            || self.prompts.loop_halt.is_some()
            || self.prompts.completion_gate.is_some()
    }

    /// Advance one animation frame. Called by the ticker only while
    /// [`is_animating`](Self::is_animating) — so the frame count doubles as an
    /// elapsed timer without a wall clock. Transient effects count down here.
    pub fn tick(&mut self) {
        self.anim.frame = self.anim.frame.wrapping_add(1);
        self.anim.overlay_ease = self.anim.overlay_ease.saturating_sub(1);
        self.anim.sidebar_settle = self.anim.sidebar_settle.saturating_sub(1);
    }

    /// Ease-in progress for the overlay, 0.0 (just opened) → 1.0 (settled). Used
    /// to scale the overlay's size for its brief expansion.
    #[must_use]
    pub fn overlay_ease_progress(&self) -> f32 {
        if self.anim.overlay_ease == 0 {
            return 1.0;
        }
        1.0 - f32::from(self.anim.overlay_ease) / f32::from(EASE_FRAMES)
    }

    /// Whether the newest modified-file entry is still settling (highlighted).
    #[must_use]
    pub fn sidebar_settling(&self) -> bool {
        self.anim.sidebar_settle > 0
    }

    /// The current animation frame (for phase-based effects like the glow).
    #[must_use]
    pub fn anim_frame(&self) -> usize {
        self.anim.frame
    }

    /// The current spinner glyph.
    #[must_use]
    pub fn spinner_glyph(&self) -> &'static str {
        SPINNER[self.anim.frame % SPINNER.len()]
    }

    /// Elapsed seconds shown next to the spinner, once past the threshold
    /// (Design §6.3). `None` before then.
    #[must_use]
    pub fn spinner_elapsed(&self) -> Option<usize> {
        let secs = self.anim.frame / ANIM_FPS;
        (secs >= ELAPSED_AFTER_SECS).then_some(secs)
    }

    /// A dull, truthful verb phrase for the spinner (Design §6.3).
    #[must_use]
    pub fn spinner_verb(&self) -> &'static str {
        let tool_running = self
            .timeline
            .items
            .iter()
            .rev()
            .any(|i| matches!(i, ConvItem::Tool { done: None, .. }));
        if tool_running {
            "working"
        } else if self.timeline.streaming {
            "responding"
        } else {
            "thinking"
        }
    }
}
