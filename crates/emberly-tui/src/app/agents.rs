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

    /// Apply an `AgentActivity` reply: open the subagent's activity
    /// **read-only** on top of the list (§4.13 — inspectable, never a black
    /// box). An unknown/ended id still gets a body, naming why.
    pub(super) fn apply_agent_activity(&mut self, name: &str, text: String) {
        self.open_text_overlay(name, text);
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
