//! The Agents inspector (FR-9, Design §3.1/§4.13).
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    /// `/agents` — open the Agents inspector. The alive-subagent list is
    /// already cached (`SubagentSpawned`/`SubagentEnded`), so the list overlay
    /// opens immediately with no engine round trip; only a selected
    /// subagent's *activity* is fetched on demand (§4.13).
    pub(super) fn open_agents_inspector(&mut self) -> Action {
        self.push_overlay(Overlay {
            title: crate::strings::agents::TITLE.into(),
            content: OverlayContent::AgentList {
                agents: self.agents.clone(),
                selected: 0,
            },
            scroll: 0,
        });
        Action::None
    }

    /// Apply an `AgentActivity` reply (§4.13 — inspectable, never a black
    /// box). If the top overlay is already showing this **same** subagent's
    /// activity — the periodic-refresh case (`tui::run`, "live-updating...
    /// as it happens") — its text is updated **in place**, keeping the
    /// user's scroll position instead of stacking a fresh overlay each time.
    /// Otherwise (the first Enter on this subagent) it opens a new overlay.
    /// A reply for a different id than the one currently shown is a stale,
    /// already-abandoned request and is dropped rather than surprising the
    /// user with a switch they did not ask for.
    pub(super) fn apply_agent_activity(&mut self, id: &str, name: &str, text: String) {
        if let Some(Overlay {
            content:
                OverlayContent::AgentActivity {
                    id: shown_id,
                    name: shown_name,
                    text: shown_text,
                },
            ..
        }) = self.overlays.last_mut()
        {
            if shown_id == id {
                *shown_name = name.to_string();
                *shown_text = text;
            }
            return;
        }
        self.push_overlay(Overlay {
            title: name.to_string(),
            content: OverlayContent::AgentActivity {
                id: id.to_string(),
                name: name.to_string(),
                text,
            },
            scroll: 0,
        });
    }

    pub(super) fn set_agent_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::AgentList { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    /// Keys for the Agents inspector (FR-9, Design §4.13): ↑/↓ move, Enter
    /// fetches and shows the selected subagent's activity read-only, Esc/q
    /// dismiss. There is no edit/delete — a subagent's activity is a record
    /// of what happened, not something the user authors.
    pub(super) fn on_agents_inspector_key(&mut self, key: KeyEvent) -> Action {
        let (len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::AgentList { agents, selected }) => {
                (agents.len(), *selected, agents.get(*selected).cloned())
            }
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_agent_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_agent_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => match chosen {
                Some(agent) => Action::Command(Command::InspectAgent { id: agent.id }),
                None => Action::None,
            },
            _ => Action::None,
        }
    }
}
