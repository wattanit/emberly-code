//! The guided provider/model setup wizard (C-7).
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    /// Keys for the guided setup wizard (Design §4.6): Enter advances after
    /// validating the current step, Esc steps back one screen (or dismisses
    /// from the first), plain typing goes to the active step's editor. The
    /// `Adapter` step is its own tiny inline up/down + Enter list — this
    /// prompt doesn't use the `overlays` stack at all, matching
    /// `LoopHaltPrompt`/`CompletionGatePrompt`.
    pub(super) fn on_provider_wizard_key(&mut self, key: KeyEvent) -> Action {
        let Some(step) = self.wizard.pending.as_ref().map(|w| w.step) else {
            return Action::None;
        };
        if step == WizardStep::Adapter {
            return self.on_provider_wizard_adapter_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.provider_wizard_step_back();
                Action::None
            }
            KeyCode::Enter => self.provider_wizard_advance(),
            KeyCode::Backspace => {
                let empty = self
                    .wizard
                    .pending
                    .as_ref()
                    .is_some_and(|w| w.editor.is_empty());
                if empty {
                    self.provider_wizard_step_back();
                } else if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.editor.backspace();
                }
                Action::None
            }
            KeyCode::Left => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.editor.left();
                }
                Action::None
            }
            KeyCode::Right => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.editor.right();
                }
                Action::None
            }
            KeyCode::Char(c) => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.editor.insert_char(c);
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn on_provider_wizard_adapter_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.provider_wizard_step_back();
                Action::None
            }
            KeyCode::Up => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.adapter_selected = wizard.adapter_selected.saturating_sub(1);
                }
                Action::None
            }
            KeyCode::Down => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.adapter_selected =
                        (wizard.adapter_selected + 1).min(WIZARD_ADAPTERS.len() - 1);
                }
                Action::None
            }
            KeyCode::Enter => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    let adapter = wizard.adapter();
                    wizard.step = WizardStep::Endpoint;
                    wizard.editor = LineEditor::new();
                    // A helpful starting point, not a forced value — the user
                    // can still overwrite it (Design §4.6).
                    if adapter == "openai" {
                        wizard.editor.insert_str("https://api.openai.com/v1");
                    }
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Step back one wizard screen, restoring that step's already-collected
    /// value into a fresh editor — so stepping back and forward again never
    /// loses anything typed. Esc from the first step dismisses the wizard.
    fn provider_wizard_step_back(&mut self) {
        let Some(prev) = self.wizard.pending.as_ref().and_then(|w| match w.step {
            WizardStep::Name => None,
            WizardStep::Adapter => Some(WizardStep::Name),
            WizardStep::Endpoint => Some(WizardStep::Adapter),
            WizardStep::ModelId => Some(WizardStep::Endpoint),
            WizardStep::ApiKey => Some(WizardStep::ModelId),
            WizardStep::Summary => Some(WizardStep::ApiKey),
        }) else {
            self.wizard.pending = None;
            return;
        };
        if let Some(wizard) = self.wizard.pending.as_mut() {
            wizard.error = None;
            wizard.step = prev;
            let seed = match prev {
                WizardStep::Name => wizard.name.clone(),
                WizardStep::Endpoint => wizard.endpoint.clone(),
                WizardStep::ModelId => wizard.model_id.clone(),
                WizardStep::ApiKey => wizard.api_key.clone(),
                WizardStep::Adapter | WizardStep::Summary => String::new(),
            };
            wizard.editor = LineEditor::new();
            wizard.editor.insert_str(&seed);
        }
    }

    /// Validate the current step and advance, or (on the `Summary` step)
    /// perform the write. Validation stays as light as raw editing gets
    /// (Tech Spec C-7): only a non-empty name that isn't already a profile,
    /// and a non-empty model id, are checked here.
    fn provider_wizard_advance(&mut self) -> Action {
        let Some(wizard) = self.wizard.pending.as_mut() else {
            return Action::None;
        };
        match wizard.step {
            WizardStep::Name => {
                let name = wizard.editor.text().trim().to_string();
                if name.is_empty() {
                    return Action::None;
                }
                if self.profiles.contains(&name) {
                    if let Some(wizard) = self.wizard.pending.as_mut() {
                        wizard.error = Some(format!("a profile named '{name}' already exists"));
                    }
                    return Action::None;
                }
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.name = name;
                    wizard.error = None;
                    wizard.step = WizardStep::Adapter;
                }
                Action::None
            }
            WizardStep::Endpoint => {
                wizard.endpoint = wizard.editor.text().trim().to_string();
                wizard.step = WizardStep::ModelId;
                wizard.editor = LineEditor::new();
                Action::None
            }
            WizardStep::ModelId => {
                let model_id = wizard.editor.text().trim().to_string();
                if model_id.is_empty() {
                    return Action::None;
                }
                wizard.model_id = model_id;
                wizard.step = WizardStep::ApiKey;
                wizard.editor = LineEditor::new();
                Action::None
            }
            WizardStep::ApiKey => {
                let key = wizard.editor.text().to_string();
                if key.is_empty() {
                    return Action::None;
                }
                wizard.api_key = key;
                wizard.step = WizardStep::Summary;
                wizard.editor = LineEditor::new();
                Action::None
            }
            WizardStep::Summary => self.provider_wizard_write(),
            // The Adapter step is handled entirely by
            // `on_provider_wizard_adapter_key` before this function is
            // reached (see `on_provider_wizard_key`).
            WizardStep::Adapter => Action::None,
        }
    }

    /// Write the completed profile (Requirements C-7) and, on success, fire
    /// the same `Command::ReloadConfig` a `/config` save would (C-5) — no
    /// bespoke apply path, no separate "wizard complete" voice (Design §4.6):
    /// the reload's own notice is the whole story.
    fn provider_wizard_write(&mut self) -> Action {
        let Some(wizard) = self.wizard.pending.as_ref() else {
            return Action::None;
        };
        let profile = NewProviderProfile {
            name: wizard.name.clone(),
            adapter: wizard.adapter().to_string(),
            base_url: (!wizard.endpoint.is_empty()).then(|| wizard.endpoint.clone()),
            model_id: wizard.model_id.clone(),
            api_key: wizard.api_key.clone(),
        };
        match self.wizard.writer.write_profile(profile) {
            Ok(()) => {
                if let Some(wizard) = self.wizard.pending.take() {
                    self.wizard
                        .created_models
                        .insert(wizard.name, wizard.model_id);
                }
                Action::Command(Command::ReloadConfig)
            }
            Err(message) => {
                if let Some(wizard) = self.wizard.pending.as_mut() {
                    wizard.error = Some(message);
                }
                Action::None
            }
        }
    }
}
