//! The four blocking prompts — permission (§6), ask_user (T-8), loop
//! halt (S-5) and completion gate (S-6) — and the answers sent back.
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// Keys while a permission prompt is open (Design §5). Scrolling reviews the
    /// full content; only `y`/`s` allow (deliberate); Enter/Esc/`d`/`n` deny
    /// (the safe default). Any other key is ignored — no accidental decision in
    /// either direction, and nothing auto-scrolls under the user.
    pub(super) fn on_permission_key(&mut self, id: PermissionId, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => {
                self.prompts.permission_scroll = self.prompts.permission_scroll.saturating_sub(1);
                Action::None
            }
            KeyCode::Down => {
                self.prompts.permission_scroll = self.prompts.permission_scroll.saturating_add(1);
                Action::None
            }
            KeyCode::PageUp => {
                self.prompts.permission_scroll =
                    self.prompts.permission_scroll.saturating_sub(SCROLL_STEP);
                Action::None
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.prompts.permission_scroll =
                    self.prompts.permission_scroll.saturating_add(SCROLL_STEP);
                Action::None
            }
            KeyCode::Home => {
                self.prompts.permission_scroll = 0;
                Action::None
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.decide(id, PermissionDecision::AllowOnce)
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.decide(id, PermissionDecision::AllowForSession)
            }
            KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Char('d')
            | KeyCode::Char('D')
            | KeyCode::Char('n')
            | KeyCode::Char('N') => self.decide(id, PermissionDecision::Deny),
            // Everything else: ignored. Decisions are deliberate.
            _ => Action::None,
        }
    }

    fn decide(&mut self, id: PermissionId, decision: PermissionDecision) -> Action {
        self.prompts.permission = None;
        self.prompts.permission_scroll = 0;
        Action::Command(Command::PermissionAnswer { id, decision })
    }

    /// Keys while a question prompt is open (T-8, Design §5.1). Typing edits the
    /// free-text answer; ↑/↓ move the option selection; Enter submits the typed
    /// text if any, else the highlighted option, else **nothing** — Enter never
    /// auto-answers. Esc declines (a real answer). No key silently decides.
    pub(super) fn on_ask_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(prompt) = self.prompts.ask.as_mut() else {
            return Action::None;
        };
        match key.code {
            // Dismiss = an explicit decline returned to the model (Design §5.1).
            KeyCode::Esc => self.answer_ask(AskAnswer::Declined),
            KeyCode::Up => {
                prompt.selected = match prompt.selected {
                    None | Some(0) => None,
                    Some(i) => Some(i - 1),
                };
                Action::None
            }
            KeyCode::Down if !prompt.options.is_empty() => {
                let last = prompt.options.len() - 1;
                prompt.selected = Some(prompt.selected.map_or(0, |i| (i + 1).min(last)));
                Action::None
            }
            KeyCode::Enter => {
                let text = prompt.editor.text().trim().to_string();
                if !text.is_empty() {
                    return self.answer_ask(AskAnswer::Answered(text));
                }
                if let Some(option) = prompt.selected.and_then(|i| prompt.options.get(i)) {
                    let answer = AskAnswer::Answered(option.clone());
                    return self.answer_ask(answer);
                }
                // Nothing typed, nothing chosen: ignored (no unsafe default).
                Action::None
            }
            // A literal newline in the free-text answer (multi-line), matching
            // the main input's Ctrl+J affordance.
            KeyCode::Char('j') if ctrl => {
                prompt.editor.newline();
                Action::None
            }
            KeyCode::Backspace => {
                prompt.editor.backspace();
                Action::None
            }
            KeyCode::Char(c) if !ctrl => {
                prompt.editor.insert_char(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn answer_ask(&mut self, answer: AskAnswer) -> Action {
        let Some(prompt) = self.prompts.ask.take() else {
            return Action::None;
        };
        Action::Command(Command::AskUserAnswer {
            id: prompt.id,
            answer,
        })
    }

    /// Keys while the loop-halt surface is open (S-5, Design §8.5). The menu:
    /// `g` keep going, `s` stop, `t`/Enter say something. In the steer field:
    /// type a message, Enter sends it, Esc goes back to the menu. Esc on the menu
    /// stops (the conservative choice — the loop halted to avoid wasted spend).
    pub(super) fn on_loop_halt_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(prompt) = self.prompts.loop_halt.as_mut() else {
            return Action::None;
        };
        if prompt.steering {
            return match key.code {
                KeyCode::Esc => {
                    prompt.steering = false;
                    prompt.editor.clear();
                    Action::None
                }
                KeyCode::Enter => {
                    let text = prompt.editor.text().trim().to_string();
                    if text.is_empty() {
                        Action::None
                    } else {
                        self.resolve_loop(LoopResolution::Steer(text))
                    }
                }
                KeyCode::Char('j') if ctrl => {
                    prompt.editor.newline();
                    Action::None
                }
                KeyCode::Backspace => {
                    prompt.editor.backspace();
                    Action::None
                }
                KeyCode::Char(c) if !ctrl => {
                    prompt.editor.insert_char(c);
                    Action::None
                }
                _ => Action::None,
            };
        }
        match key.code {
            KeyCode::Char('g') | KeyCode::Char('G') => self.resolve_loop(LoopResolution::Resume),
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Esc => {
                self.resolve_loop(LoopResolution::Stop)
            }
            KeyCode::Char('t') | KeyCode::Char('T') | KeyCode::Enter => {
                prompt.steering = true;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn resolve_loop(&mut self, resolution: LoopResolution) -> Action {
        self.prompts.loop_halt = None;
        Action::Command(Command::ResolveLoop { resolution })
    }

    /// Keys while the completion-gate halt surface is open (S-6, Design
    /// §8.7). The menu: `g` keep going, `s` stop, `t`/Enter say something,
    /// **`f` finish anyway** (the explicit override). In the steer field: type
    /// a message, Enter sends it, Esc goes back to the menu. Esc on the menu
    /// stops (the conservative choice, mirroring the loop-halt surface).
    pub(super) fn on_completion_gate_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(prompt) = self.prompts.completion_gate.as_mut() else {
            return Action::None;
        };
        if prompt.steering {
            return match key.code {
                KeyCode::Esc => {
                    prompt.steering = false;
                    prompt.editor.clear();
                    Action::None
                }
                KeyCode::Enter => {
                    let text = prompt.editor.text().trim().to_string();
                    if text.is_empty() {
                        Action::None
                    } else {
                        self.resolve_completion_gate(GateResolution::Steer(text))
                    }
                }
                KeyCode::Char('j') if ctrl => {
                    prompt.editor.newline();
                    Action::None
                }
                KeyCode::Backspace => {
                    prompt.editor.backspace();
                    Action::None
                }
                KeyCode::Char(c) if !ctrl => {
                    prompt.editor.insert_char(c);
                    Action::None
                }
                _ => Action::None,
            };
        }
        match key.code {
            KeyCode::Char('g') | KeyCode::Char('G') => {
                self.resolve_completion_gate(GateResolution::Resume)
            }
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Esc => {
                self.resolve_completion_gate(GateResolution::Stop)
            }
            KeyCode::Char('t') | KeyCode::Char('T') | KeyCode::Enter => {
                prompt.steering = true;
                Action::None
            }
            // Finish anyway — the user's explicit override (Design §8.7),
            // never presented as though the checks passed.
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.resolve_completion_gate(GateResolution::Finish)
            }
            _ => Action::None,
        }
    }

    fn resolve_completion_gate(&mut self, resolution: GateResolution) -> Action {
        self.prompts.completion_gate = None;
        Action::Command(Command::ResolveCompletionGate { resolution })
    }
}
