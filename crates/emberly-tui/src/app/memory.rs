//! The memory inspector (FR-6, T-13): listing entries, reading a body,
//! and the edit/confirm flow.
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// `/memory` — open the memory inspector. Requests the grouped entry list;
    /// the overlay opens (or refreshes) when the `MemoryEntries` reply arrives.
    /// The engine owns the store, so the TUI never reads memory files directly.
    pub(super) fn open_memory_inspector(&mut self) -> Action {
        Action::Command(Command::MemoryList)
    }

    /// Apply a `MemoryEntries` reply: refresh an already-open inspector in place
    /// (so a post-mutation re-list updates the list without a flash), or open a
    /// fresh one. The project group is simply empty on an untrusted root (FR-1).
    pub(super) fn apply_memory_entries(
        &mut self,
        user: Vec<EntrySummary>,
        project: Vec<EntrySummary>,
    ) {
        let total = user.len() + project.len();
        let existing = self
            .overlays
            .iter()
            .rposition(|o| matches!(o.content, OverlayContent::MemoryEntries { .. }));
        match existing {
            Some(i) => {
                if let OverlayContent::MemoryEntries {
                    user: u,
                    project: p,
                    selected,
                    confirm_delete,
                } = &mut self.overlays[i].content
                {
                    *u = user;
                    *p = project;
                    *selected = (*selected).min(total.saturating_sub(1));
                    // A refresh cancels any half-finished confirm — the list it
                    // referred to just changed under it.
                    *confirm_delete = false;
                }
            }
            None => self.push_overlay(Overlay {
                title: crate::strings::memory::TITLE.into(),
                content: OverlayContent::MemoryEntries {
                    user,
                    project,
                    selected: 0,
                    confirm_delete: false,
                },
                scroll: 0,
            }),
        }
    }

    /// Apply a `MemoryBody` reply, dispatched by the intent recorded when the
    /// fetch was issued: view opens the body read-only; edit stages it for the
    /// `$EDITOR` handoff the frontend loop performs. A reply that no longer
    /// matches the recorded target (a stale/late arrival) is ignored.
    pub(super) fn apply_memory_body(&mut self, scope: MemoryScope, name: &str, body: String) {
        let Some((intent, target)) = self.memory_fetch.take() else {
            return;
        };
        if target.scope != scope || target.name != name {
            return;
        }
        match intent {
            MemoryFetchIntent::View => {
                let title = format!("{} · {}", target.name, memory_scope_label(scope));
                let shown = if body.trim().is_empty() {
                    crate::strings::memory::EMPTY_BODY.to_string()
                } else {
                    body
                };
                self.open_text_overlay(title, shown);
            }
            MemoryFetchIntent::Edit => {
                self.pending_memory_edit = Some(PendingMemoryEdit {
                    scope,
                    name: target.name.clone(),
                    description: Some(target.description.clone()).filter(|d| !d.is_empty()),
                    type_: target.type_.clone(),
                    body,
                });
            }
        }
    }

    /// The entry currently highlighted in the memory inspector (flattened over
    /// the user then project groups), if any.
    fn selected_memory_entry(&self) -> Option<EntrySummary> {
        match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::MemoryEntries {
                user,
                project,
                selected,
                ..
            }) => user.iter().chain(project.iter()).nth(*selected).cloned(),
            _ => None,
        }
    }

    /// Issue a `MemoryView` for the highlighted entry, recording the intent so
    /// the `MemoryBody` reply is routed to view or edit.
    fn begin_memory_fetch(&mut self, intent: MemoryFetchIntent) -> Action {
        match self.selected_memory_entry() {
            Some(target) => {
                let cmd = Command::MemoryView {
                    scope: target.scope,
                    name: target.name.clone(),
                };
                self.memory_fetch = Some((intent, target));
                Action::Command(cmd)
            }
            None => Action::None,
        }
    }

    pub(super) fn set_memory_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::MemoryEntries { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    fn set_memory_confirm(&mut self, on: bool) {
        if let Some(Overlay {
            content: OverlayContent::MemoryEntries { confirm_delete, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *confirm_delete = on;
        }
    }

    /// Drain a staged memory edit for the frontend loop's `$EDITOR` handoff
    /// (FR-6, §4.6).
    pub fn take_pending_memory_edit(&mut self) -> Option<PendingMemoryEdit> {
        self.pending_memory_edit.take()
    }

    /// Keys for the memory inspector (FR-6, Design §4.9): ↑/↓ move, Enter views
    /// the body, `e` edits it via `$EDITOR`, `d` starts a confirmed delete,
    /// Esc/q dismiss. Delete is destructive, so it takes an explicit y/N step —
    /// never a lone key (§3.4 spirit).
    pub(super) fn on_memory_inspector_key(&mut self, key: KeyEvent) -> Action {
        let (user_len, project_len, selected, confirm) =
            match self.overlays.last().map(|o| &o.content) {
                Some(OverlayContent::MemoryEntries {
                    user,
                    project,
                    selected,
                    confirm_delete,
                }) => (user.len(), project.len(), *selected, *confirm_delete),
                _ => return Action::None,
            };
        let total = user_len + project_len;

        // The confirm-delete step owns the keyboard until resolved: only `y`
        // deletes; every other key cancels (deny-by-default for a destructive
        // action).
        if confirm {
            self.set_memory_confirm(false);
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                if let Some(target) = self.selected_memory_entry() {
                    return Action::Command(Command::MemoryMutate {
                        op: MemoryOp::Remove,
                        scope: target.scope,
                        name: target.name,
                        description: None,
                        type_: None,
                        body: None,
                    });
                }
            }
            return Action::None;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_memory_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_memory_selection((selected + 1).min(total.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => self.begin_memory_fetch(MemoryFetchIntent::View),
            KeyCode::Char('e') => self.begin_memory_fetch(MemoryFetchIntent::Edit),
            KeyCode::Char('d') => {
                if total > 0 {
                    self.set_memory_confirm(true);
                }
                Action::None
            }
            _ => Action::None,
        }
    }
}
