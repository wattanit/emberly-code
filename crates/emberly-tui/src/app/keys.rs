//! Raw input: keys, scroll, clicks, paste. Dispatches to the focused
//! surface; each surface's own handler lives with that surface.
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    /// Handle a key press, returning the action for the loop to carry out.
    ///
    /// While a permission prompt is open it owns the keyboard: only the
    /// deliberate allow keys approve, and everything else (including Enter and
    /// Esc) denies — deny is the safe default (Design §5).
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        // The command palette is modal while open (Design §3.3).
        if self.palette.is_some() {
            return self.on_palette_key(key);
        }
        // An open overlay is modal for navigation: scroll or dismiss (Design
        // §4.2). It sits above the permission check so a diff can be reviewed,
        // and no overlay is ever opened while a permission prompt is up.
        if !self.overlays.is_empty() {
            return self.on_overlay_key(key);
        }
        if let Some(id) = self.prompts.permission.as_ref().map(|(i, _)| *i) {
            return self.on_permission_key(id, key);
        }
        // The question prompt also owns the keyboard while open (Design §5.1),
        // but with opposite semantics: no unsafe default, Esc declines.
        if self.prompts.ask.is_some() {
            return self.on_ask_key(key);
        }
        // The loop-halt surface owns the keyboard too (Design §8.5) — the
        // harness stepping in; the user always decides what happens next.
        if self.prompts.loop_halt.is_some() {
            return self.on_loop_halt_key(key);
        }
        // The completion-gate halt surface owns the keyboard too (S-6, Design
        // §8.7) — the harness stepping in after a bounded number of failed
        // completion attempts.
        if self.prompts.completion_gate.is_some() {
            return self.on_completion_gate_key(key);
        }
        // The guided setup wizard owns the keyboard too (Requirements C-7,
        // Design §4.6) — a focused sequence of single-question screens.
        if self.wizard.pending.is_some() {
            return self.on_provider_wizard_key(key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // Newline (multi-line input) via Ctrl+J — a plain LF that every
            // terminal delivers, so multi-line entry works even where Shift+
            // Enter is indistinguishable from Enter.
            KeyCode::Char('j') if ctrl => self.edit(|e| e.newline()),
            // Ctrl-D on an empty line quits; on a non-empty line it deletes
            // forward (readline convention).
            KeyCode::Char('d') if ctrl => {
                if self.editor.is_empty() {
                    return Action::Quit;
                }
                self.editor.delete();
                Action::None
            }
            KeyCode::Char('c') if ctrl => {
                if self.anim.busy {
                    Action::Command(Command::Cancel)
                } else if self.editor.is_empty() {
                    Action::Quit
                } else {
                    self.editor.clear();
                    Action::None
                }
            }
            // Cancel an in-flight turn (Command::Cancel doc, README keybinding
            // table). No modal is open here (those handle their own Esc
            // above), so idle Esc has nothing to dismiss.
            KeyCode::Esc if self.anim.busy => Action::Command(Command::Cancel),
            KeyCode::Char('b') if ctrl => {
                self.sidebar_visible = !self.sidebar_visible;
                Action::None
            }
            // Shift+Tab cycles the auto-accept mode (crossterm delivers it as
            // BackTab). The engine gates the auto tiers on confinement.
            KeyCode::BackTab => self.run_command(AppCommand::CycleMode),
            // Open the command palette (Design §3.3).
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(PaletteState::default());
                Action::None
            }
            // Open the most-recently-modified file's diff in an overlay.
            KeyCode::Char('o') if ctrl => {
                self.open_last_diff();
                Action::None
            }
            // Toggle the most recent reasoning trail open/closed (Design §4.4).
            KeyCode::Char('r') if ctrl => {
                self.toggle_reasoning();
                Action::None
            }
            // Force a full repaint (README keybinding table) — a manual
            // escape hatch for display corruption, so a resize is never the
            // only way to clear it.
            KeyCode::Char('l') if ctrl => Action::ForceRedraw,
            // Emacs-style line editing.
            KeyCode::Char('a') if ctrl => self.edit(|e| e.home()),
            KeyCode::Char('e') if ctrl => self.edit(|e| e.end()),
            KeyCode::Char('k') if ctrl => self.edit(|e| e.kill_to_end()),
            KeyCode::Char('w') if ctrl => self.edit(|e| e.delete_word_back()),
            // Shift+Enter and Alt+Enter insert a newline where the terminal
            // reports the modifier; plain Enter submits.
            KeyCode::Enter if shift || alt => self.edit(|e| e.newline()),
            KeyCode::Enter => match self.editor.submit() {
                Some(text) => {
                    self.timeline.scroll = 0; // jump back to the latest output
                                              // A leading '/' is a slash command, not a message.
                    if let Some(name) = text.strip_prefix('/') {
                        self.run_slash(name)
                    } else {
                        // Echo the prompt into the timeline so the main pane is
                        // a single top-to-bottom transcript of both sides.
                        self.timeline.items.push(ConvItem::User(text.clone()));
                        // Enter the "working" state (Design §6.3); the spinner
                        // runs from frame 0 until TurnEnded.
                        self.anim.busy = true;
                        self.anim.frame = 0;
                        Action::Command(Command::UserInput { text })
                    }
                }
                None => Action::None,
            },
            // Scroll the conversation history.
            KeyCode::PageUp => {
                self.timeline.scroll = self.timeline.scroll.saturating_add(SCROLL_STEP);
                Action::None
            }
            KeyCode::PageDown => {
                self.timeline.scroll = self.timeline.scroll.saturating_sub(SCROLL_STEP);
                Action::None
            }
            KeyCode::Backspace => self.edit(|e| e.backspace()),
            KeyCode::Delete => self.edit(|e| e.delete()),
            KeyCode::Left if ctrl => self.edit(|e| e.word_left()),
            KeyCode::Right if ctrl => self.edit(|e| e.word_right()),
            KeyCode::Left => self.edit(|e| e.left()),
            KeyCode::Right => self.edit(|e| e.right()),
            KeyCode::Home => self.edit(|e| e.home()),
            KeyCode::End => self.edit(|e| e.end()),
            // Up/Down move between logical lines; at the top/bottom edge they
            // step through input history instead.
            KeyCode::Up => {
                if !self.editor.up() {
                    self.editor.history_prev();
                }
                Action::None
            }
            KeyCode::Down => {
                if !self.editor.down() {
                    self.editor.history_next();
                }
                Action::None
            }
            KeyCode::Char(c) => self.edit(|e| e.insert_char(c)),
            _ => Action::None,
        }
    }

    /// Run an editor mutation and report nothing observable to the loop.
    fn edit(&mut self, f: impl FnOnce(&mut LineEditor)) -> Action {
        f(&mut self.editor);
        Action::None
    }

    /// Handle a mouse-wheel scroll, routed to whatever is focused: an open
    /// overlay, the permission prompt, or the conversation history. `up` means
    /// scrolling toward older content.
    pub fn on_scroll(&mut self, up: bool) {
        let step = 3;
        // Modal priority mirrors `on_key` (palette > overlay > permission >
        // conversation, Design §3.3/§3.4): the wheel scrolls the focused
        // surface, so an open palette takes the wheel before any lower pane.
        if self.palette.is_some() {
            // The palette viewport follows `selected` (the render windows the
            // list around it), so moving the selection is exactly how the list
            // scrolls — the same action as the Up/Down keys (keyboard parity,
            // §3.4). One item per wheel notch, matching a single arrow press.
            let last = commands::matches(self.palette_query())
                .len()
                .saturating_sub(1);
            if let Some(p) = self.palette.as_mut() {
                p.selected = if up {
                    p.selected.saturating_sub(1)
                } else {
                    (p.selected + 1).min(last)
                };
            }
            return;
        }
        if let Some(o) = self.overlays.last_mut() {
            o.scroll = if up {
                o.scroll.saturating_sub(step)
            } else {
                o.scroll.saturating_add(step)
            };
        } else if self.prompts.permission.is_some() {
            self.prompts.permission_scroll = if up {
                self.prompts.permission_scroll.saturating_sub(step)
            } else {
                self.prompts.permission_scroll.saturating_add(step)
            };
        } else {
            // Conversation scroll is measured from the bottom: wheel-up moves
            // back into history (larger offset).
            self.timeline.scroll = if up {
                self.timeline.scroll.saturating_add(step)
            } else {
                self.timeline.scroll.saturating_sub(step)
            };
        }
    }

    /// Handle an unmodified left click at `(col, row)` (Design §3.4). A click is
    /// a shortcut for "focus + Enter": it resolves against the last frame's
    /// hit-map and then reuses the **exact same keyboard handler** the Enter key
    /// would — the mouse adds no capability the keyboard lacks (the §3.4
    /// invariant). A click on nothing interactive is inert. Returns the `Action`
    /// the keypress would, so the frontend loop routes it identically.
    pub fn on_click(&mut self, col: u16, row: u16) -> Action {
        let Some(target) = self.hit_map.hit(col, row) else {
            return Action::None;
        };
        match target {
            ClickTarget::PaletteRow(row) => {
                // Focus the clicked row, then activate it exactly as palette
                // Enter does (on_palette_key) — no separate dispatch path.
                if let Some(p) = self.palette.as_mut() {
                    p.selected = row;
                }
                self.on_palette_key(KeyEvent::from(KeyCode::Enter))
            }
            ClickTarget::ChoiceRow(row) => {
                // Focus the clicked choice, then confirm it exactly as picker
                // Enter does (on_choice_picker_key).
                self.set_choice_selection(row);
                self.on_choice_picker_key(KeyEvent::from(KeyCode::Enter))
            }
            ClickTarget::SessionRow(row) => {
                // Focus + Enter on the session picker (resume).
                self.set_picker_selection(row);
                self.on_session_picker_key(KeyEvent::from(KeyCode::Enter))
            }
            ClickTarget::MemoryRow(row) => {
                // Focus + Enter on the memory inspector (view the entry).
                self.set_memory_selection(row);
                self.on_memory_inspector_key(KeyEvent::from(KeyCode::Enter))
            }
            ClickTarget::SkillRow(row) => {
                // Focus + Enter on the skills inspector (read-only body view).
                self.set_skill_selection(row);
                self.on_skills_inspector_key(KeyEvent::from(KeyCode::Enter))
            }
            ClickTarget::ReasoningToggle => {
                // Exactly the Ctrl+R action — toggle the most recent trail.
                self.toggle_reasoning();
                Action::None
            }
            ClickTarget::OpenDiff => {
                // Exactly the Ctrl+O action — open the most-recent diff overlay.
                self.open_last_diff();
                Action::None
            }
            ClickTarget::OpenMemoryInspector => {
                // Exactly the `/memory` action (palette-reachable, §3.3).
                self.run_command(AppCommand::Memory)
            }
            ClickTarget::OpenSkillsInspector => {
                // Exactly the `/skills` action.
                self.run_command(AppCommand::Skills)
            }
            ClickTarget::PermissionChoice(choice) => {
                // Reuse `on_permission_key` EXACTLY (Design §3.4/§5): a click on
                // an affordance is the same deliberate act as its key, and can
                // do nothing the key cannot. Only lands here when the click hit
                // an affordance rect (the render only pushes those); a click
                // elsewhere on the prompt resolves to nothing → inert. It never
                // approves "whatever is focused," and it never bypasses the
                // unscrolled-content indicator the key path shows.
                let Some(id) = self.prompts.permission.as_ref().map(|(i, _)| *i) else {
                    return Action::None;
                };
                let code = match choice {
                    PermissionChoice::Allow => KeyCode::Char('y'),
                    PermissionChoice::Session => KeyCode::Char('s'),
                    PermissionChoice::Deny => KeyCode::Enter,
                };
                self.on_permission_key(id, KeyEvent::from(code))
            }
        }
    }

    /// Insert pasted text (bracketed paste) into the input, unless a permission
    /// prompt or overlay is open — nothing may be typed into a decision, and an
    /// overlay is read-only (Design §5, §4.2).
    pub fn on_paste(&mut self, text: &str) {
        if !self.is_deciding() && self.overlays.is_empty() && self.palette.is_none() {
            self.editor.insert_str(text);
        }
    }

    /// Keys while an overlay is open. The session picker is interactive (↑/↓
    /// move, Enter resumes); every other overlay is a scrollable, read-only
    /// pane (Esc/q dismiss; the rest scroll).
    fn on_overlay_key(&mut self, key: KeyEvent) -> Action {
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Sessions { .. })
        ) {
            return self.on_session_picker_key(key);
        }
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::Choices { .. })
        ) {
            return self.on_choice_picker_key(key);
        }
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::MemoryEntries { .. })
        ) {
            return self.on_memory_inspector_key(key);
        }
        if matches!(
            self.overlays.last().map(|o| &o.content),
            Some(OverlayContent::SkillList { .. })
        ) {
            return self.on_skills_inspector_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
            }
            KeyCode::Char('c') if ctrl => {
                self.overlays.pop();
            }
            KeyCode::Up => self.scroll_overlay(-1),
            KeyCode::Down => self.scroll_overlay(1),
            KeyCode::PageUp => self.scroll_overlay(-(SCROLL_STEP as isize)),
            KeyCode::PageDown => self.scroll_overlay(SCROLL_STEP as isize),
            KeyCode::Home => {
                if let Some(o) = self.overlays.last_mut() {
                    o.scroll = 0;
                }
            }
            _ => {}
        }
        Action::None
    }

    fn scroll_overlay(&mut self, delta: isize) {
        if let Some(o) = self.overlays.last_mut() {
            o.scroll = o.scroll.saturating_add_signed(delta);
        }
    }
}
