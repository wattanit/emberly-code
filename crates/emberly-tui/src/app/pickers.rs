//! Selection surfaces: the command palette and the model / effort /
//! mode / session / choice pickers.
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// Open the model/provider picker (`/model` with no args, the palette, or a
    /// keybinding — C-6). Rows are the configured profiles, the active one
    /// marked, plus a trailing row into the guided setup wizard (Requirements
    /// C-7, Design §4.6) — shown even with zero profiles configured, since
    /// that's exactly when a user most needs it. Enter on a profile row issues
    /// a `SwitchModel`; Enter on the trailing row starts the wizard instead.
    pub(super) fn open_model_picker(&mut self) {
        let active = self.session.provider.clone();
        let mut rows: Vec<ChoiceRow> = self
            .profiles
            .iter()
            .map(|name| ChoiceRow {
                label: name.clone(),
                current: *name == active,
            })
            .collect();
        let selected = rows.iter().position(|r| r.current).unwrap_or(0);
        rows.push(ChoiceRow {
            label: ADD_PROVIDER_ROW.to_string(),
            current: false,
        });
        self.push_overlay(Overlay {
            title: "switch model".into(),
            content: OverlayContent::Choices {
                kind: ChoiceKind::Model,
                rows,
                selected,
            },
            scroll: 0,
        });
    }

    /// Open the reasoning-effort picker (`/effort`, P-9): the active model's
    /// levels, current one marked; Enter issues a `SetEffort`. A model with no
    /// effort control declines with a calm notice.
    pub(super) fn open_effort_picker(&mut self) {
        if self.effort_levels.is_empty() {
            self.timeline.items.push(ConvItem::Notice(
                "this model has no reasoning-effort control".into(),
            ));
            return;
        }
        let rows: Vec<ChoiceRow> = self
            .effort_levels
            .iter()
            .map(|level| ChoiceRow {
                label: level.as_str().to_string(),
                current: self.effort == Some(*level),
            })
            .collect();
        let selected = rows.iter().position(|r| r.current).unwrap_or(0);
        self.push_overlay(Overlay {
            title: "reasoning effort".into(),
            content: OverlayContent::Choices {
                kind: ChoiceKind::Effort,
                rows,
                selected,
            },
            scroll: 0,
        });
    }

    /// Open the permission-mode picker (`/mode` with no args or the palette).
    /// Rows are the three tiers, the current one marked; the auto tiers are
    /// marked unavailable (and not selectable) when OS confinement is not
    /// active — the engine would refuse them anyway, so the picker says so up
    /// front (Requirements §6.4, §6.7).
    pub(super) fn open_mode_picker(&mut self) {
        let auto_ok = self
            .sandbox
            .as_ref()
            .is_some_and(SandboxStatus::allows_auto_modes);
        let rows: Vec<ChoiceRow> = [Mode::Normal, Mode::AutoAcceptEdits, Mode::Auto]
            .iter()
            .map(|&m| {
                let label = mode_label(m, auto_ok);
                ChoiceRow {
                    label,
                    current: m == self.mode,
                }
            })
            .collect();
        let selected = rows.iter().position(|r| r.current).unwrap_or(0);
        self.push_overlay(Overlay {
            title: "permission mode".into(),
            content: OverlayContent::Choices {
                kind: ChoiceKind::Mode,
                rows,
                selected,
            },
            scroll: 0,
        });
    }

    /// Build the session-picker rows from the transcripts on disk, newest
    /// first, marking the one we are currently in.
    pub(super) fn session_rows(&self) -> Vec<SessionRow> {
        resume::list_sessions(&self.sessions_dir)
            .into_iter()
            .map(|s| {
                let flag = if s.interrupted { " · interrupted" } else { "" };
                SessionRow {
                    current: s.id == self.session.session_id,
                    title: s.title.unwrap_or_else(|| "(untitled)".into()),
                    subtitle: format!("{}/{} · {} events{flag}", s.provider, s.model, s.events),
                    id: s.id,
                }
            })
            .collect()
    }

    /// Keys while the palette is open: type to filter, ↑/↓ to move, Enter to
    /// run the selection, Esc/Ctrl+P to dismiss.
    pub(super) fn on_palette_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let filtered = commands::matches(self.palette_query());
        match key.code {
            KeyCode::Esc => {
                self.palette = None;
                Action::None
            }
            KeyCode::Char('p') if ctrl => {
                self.palette = None;
                Action::None
            }
            KeyCode::Enter => {
                let chosen = self
                    .palette
                    .as_ref()
                    .and_then(|p| filtered.get(p.selected).copied());
                self.palette = None;
                match chosen {
                    // Dispatch by name so argument-taking commands (`/mode`,
                    // `/model`, `/effort`) open their picker from the palette,
                    // just as their no-arg slash forms do.
                    Some(i) => self.run_slash(commands::COMMANDS[i].name),
                    None => Action::None,
                }
            }
            KeyCode::Up => {
                if let Some(p) = self.palette.as_mut() {
                    p.selected = p.selected.saturating_sub(1);
                }
                Action::None
            }
            KeyCode::Down => {
                if let Some(p) = self.palette.as_mut() {
                    let last = filtered.len().saturating_sub(1);
                    p.selected = (p.selected + 1).min(last);
                }
                Action::None
            }
            KeyCode::Backspace => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
                Action::None
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    pub(super) fn palette_query(&self) -> &str {
        self.palette.as_ref().map_or("", |p| p.query.as_str())
    }

    /// Keys for the session picker: ↑/↓ move the selection, Enter resumes the
    /// highlighted session (a no-op on the current one), Esc/q dismiss.
    pub(super) fn on_session_picker_key(&mut self, key: KeyEvent) -> Action {
        // Read the selection and the chosen row without holding a borrow across
        // the mutation the arms perform.
        let (len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::Sessions { rows, selected }) => {
                (rows.len(), *selected, rows.get(*selected).cloned())
            }
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_picker_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_picker_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => match chosen {
                Some(row) if row.current => {
                    self.overlays.pop();
                    self.timeline
                        .items
                        .push(ConvItem::Notice("already in this session".into()));
                    Action::None
                }
                Some(row) if self.anim.busy => {
                    self.overlays.pop();
                    self.timeline.items.push(ConvItem::Notice(
                        "finish or cancel the current turn before switching sessions".into(),
                    ));
                    let _ = row;
                    Action::None
                }
                Some(row) => {
                    self.overlays.pop();
                    Action::ResumeSession(row.id)
                }
                None => Action::None,
            },
            _ => Action::None,
        }
    }

    pub(super) fn set_picker_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::Sessions { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    /// Keys for the generic choice picker (`/model` now): ↑/↓ move, Enter
    /// applies the highlighted choice via the command its `kind` maps to,
    /// Esc/q dismiss.
    pub(super) fn on_choice_picker_key(&mut self, key: KeyEvent) -> Action {
        let (kind, len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::Choices {
                kind,
                rows,
                selected,
            }) => (*kind, rows.len(), *selected, rows.get(*selected).cloned()),
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_choice_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_choice_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => {
                let Some(row) = chosen else {
                    return Action::None;
                };
                self.overlays.pop();
                match kind {
                    ChoiceKind::Model => {
                        if row.label == ADD_PROVIDER_ROW {
                            self.wizard.pending = Some(ProviderWizard::new());
                            Action::None
                        } else if row.current {
                            self.timeline
                                .items
                                .push(ConvItem::Notice(format!("already using {}", row.label)));
                            Action::None
                        } else if self.anim.busy {
                            self.timeline.items.push(ConvItem::Notice(
                                "finish or cancel the current turn before switching models".into(),
                            ));
                            Action::None
                        } else {
                            // A model just added by this session's wizard is
                            // known locally (Requirements C-7) even though the
                            // picker otherwise has no per-profile default model
                            // to fall back to — everything else keeps today's
                            // behavior of reusing the previously active model.
                            let model = self.wizard.created_models.get(&row.label).cloned();
                            Action::Command(Command::SwitchModel {
                                profile: row.label,
                                model,
                            })
                        }
                    }
                    ChoiceKind::Effort => {
                        if row.current {
                            Action::None // already at this level
                        } else {
                            match Effort::parse(&row.label) {
                                Some(effort) => Action::Command(Command::SetEffort { effort }),
                                None => Action::None,
                            }
                        }
                    }
                    ChoiceKind::Mode => {
                        if row.current {
                            Action::None // already in this mode
                        } else {
                            // The label is the mode name, possibly with a
                            // `(needs OS confinement)` suffix when the auto
                            // tier is unavailable — parse the leading name.
                            let name = row.label.split_whitespace().next().unwrap_or("");
                            match parse_mode(name) {
                                Some(mode) if mode == Mode::Normal => {
                                    Action::Command(Command::SetMode { mode })
                                }
                                Some(mode) => {
                                    let auto_ok = self
                                        .sandbox
                                        .as_ref()
                                        .is_some_and(SandboxStatus::allows_auto_modes);
                                    if auto_ok {
                                        Action::Command(Command::SetMode { mode })
                                    } else {
                                        self.timeline.items.push(ConvItem::Notice(
                                            "auto-accept modes are unavailable without OS confinement"
                                                .into(),
                                        ));
                                        Action::None
                                    }
                                }
                                None => Action::None,
                            }
                        }
                    }
                }
            }
            _ => Action::None,
        }
    }

    pub(super) fn set_choice_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::Choices { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }
}
