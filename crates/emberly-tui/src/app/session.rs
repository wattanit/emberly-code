//! Session switching and the small text projections the frontend
//! derives from conversation state.
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    pub(super) fn last_assistant_text(&self) -> Option<String> {
        self.conversation.iter().rev().find_map(|item| match item {
            ConvItem::Assistant(text) => Some(text.clone()),
            _ => None,
        })
    }

    pub(super) fn files_text(&self) -> String {
        if self.modified_files.is_empty() {
            return "No files changed yet.".to_string();
        }
        self.modified_files
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
        self.conversation
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
        self.conversation
            .push(ConvItem::Notice("resumed session".into()));
    }

    /// Push a harness-voice notice into the timeline (used by the frontend for
    /// out-of-band feedback such as a failed session switch).
    pub fn notice(&mut self, message: impl Into<String>) {
        self.conversation.push(ConvItem::Notice(message.into()));
    }

    /// Shared reset for both switch paths: clear the conversation and per-session
    /// view state, adopt the new identity.
    fn reset_for_switch(&mut self, id: SessionId, title: String) {
        self.session.session_id = id;
        self.session.title = title;
        self.conversation.clear();
        self.modified_files.clear();
        self.tasks.clear();
        self.memory_user = 0;
        self.memory_project = 0;
        self.skills.clear();
        self.completion_status.clear();
        self.memory_fetch = None;
        self.pending_memory_edit = None;
        self.latest_diffs.clear();
        self.last_modified = None;
        self.scroll = 0;
        self.streaming = false;
        self.context_pct = 0;
        self.context_tokens = 0;
        self.session_usage = TokenUsage::default();
        self.cost_usd = 0.0;
        self.cost_known = false;
        self.overlays.clear();
    }
}
