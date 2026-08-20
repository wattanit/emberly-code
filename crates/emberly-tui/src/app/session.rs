//! Session switching and the small text projections the frontend
//! derives from conversation state.
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    pub(super) fn last_assistant_text(&self) -> Option<String> {
        self.timeline
            .items
            .iter()
            .rev()
            .find_map(|item| match item {
                ConvItem::Assistant(text) => Some(text.clone()),
                _ => None,
            })
    }

    pub(super) fn files_text(&self) -> String {
        if self.files.modified.is_empty() {
            return "No files changed yet.".to_string();
        }
        self.files
            .modified
            .iter()
            .map(|f| format!("{}  +{} -{}", f.path, f.adds, f.dels))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Reset the timeline and identity for a brand-new session started in place
    /// (`/new`). Driven by the frontend once the engine has been told; the
    /// engine's follow-up context-usage event refines the counters.
    pub fn begin_new_session(&mut self, id: SessionId) {
        self.reset_for_switch(id, String::new());
        self.timeline
            .items
            .push(ConvItem::Notice("started a new session".into()));
    }

    /// Reset and reseed the timeline for a resumed session (`/resume` from the
    /// picker), restoring its history so it is not a blank pane.
    pub fn begin_resumed_session(
        &mut self,
        id: SessionId,
        title: String,
        records: &[TranscriptRecord],
    ) {
        self.reset_for_switch(id, title);
        self.seed_history(records);
        self.timeline
            .items
            .push(ConvItem::Notice("resumed session".into()));
    }

    /// Push a harness-voice notice into the timeline (used by the frontend for
    /// out-of-band feedback such as a failed session switch).
    pub fn notice(&mut self, message: impl Into<String>) {
        self.timeline.items.push(ConvItem::Notice(message.into()));
    }

    /// Shared reset for both switch paths: clear the conversation and per-session
    /// view state, adopt the new identity.
    fn reset_for_switch(&mut self, id: SessionId, title: String) {
        self.session.session_id = id;
        self.session.title = title;
        self.timeline.items.clear();
        self.files.modified.clear();
        self.tasks.clear();
        self.memory.user = 0;
        self.memory.project = 0;
        self.skills.clear();
        self.agents.clear();
        self.mcp_servers.clear();
        self.completion_status.clear();
        // Staged-but-unsent attachments (FR-10) belong to the composing
        // message, not the session it was composed in; a switch drops them
        // exactly like the engine's own `Engine::pending_attachments` does
        // (a fresh/resumed session starts with no staged image).
        self.pending_attachments.clear();
        self.memory.fetch = None;
        self.memory.pending_edit = None;
        self.files.diffs.clear();
        self.files.last = None;
        self.timeline.scroll = 0;
        self.timeline.streaming = false;
        self.usage.context_pct = 0;
        self.usage.context_tokens = 0;
        self.usage.tokens = TokenUsage::default();
        self.usage.cost_usd = 0.0;
        self.usage.cost_known = false;
        self.overlays.clear();
    }
}
