//! The MCP inspector (FR-11, Design §4.15).
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    /// `/mcp` — open the MCP inspector. The connected-servers list (and each
    /// server's own tool list) is already cached from
    /// `UiEvent::McpServerConnected`, so the whole inspector opens with no
    /// engine round-trip at all — unlike `SkillList`/`AgentList`, not even a
    /// selected entry's "detail" needs one.
    pub(super) fn open_mcp_inspector(&mut self) -> Action {
        self.push_overlay(Overlay {
            title: crate::strings::mcp::TITLE.into(),
            content: OverlayContent::McpServerList {
                servers: self.mcp_servers.clone(),
                selected: 0,
            },
            scroll: 0,
        });
        Action::None
    }

    pub(super) fn set_mcp_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::McpServerList { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    /// Keys for the MCP inspector (FR-11, Design §4.15): ↑/↓ move, Enter
    /// shows the selected server's discovered tools **read-only**, Esc/q
    /// dismiss. There is no edit — a server is an externally-connected
    /// process, not something the harness owns.
    pub(super) fn on_mcp_inspector_key(&mut self, key: KeyEvent) -> Action {
        let (len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::McpServerList { servers, selected }) => {
                (servers.len(), *selected, servers.get(*selected).cloned())
            }
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_mcp_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_mcp_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => {
                if let Some(server) = chosen {
                    let title = server.name.clone();
                    let text = if server.tools.is_empty() {
                        "(this server advertised no tools)".to_string()
                    } else {
                        server.tools.join("\n")
                    };
                    self.open_text_overlay(title, text);
                }
                Action::None
            }
            _ => Action::None,
        }
    }
}
