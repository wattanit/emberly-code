//! Slash commands and [`AppCommand`] dispatch — what the user asked
//! for, turned into engine `Command`s or local state changes.
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// Handle `/mode [name]`: no arg opens the picker; an arg sets the mode
    /// directly, validated against confinement (the auto tiers need an active
    /// sandbox — Requirements §6.4).
    fn mode_command(&mut self, arg: &str) -> Action {
        let arg = arg.trim();
        if arg.is_empty() {
            self.open_mode_picker();
            return Action::None;
        }
        match parse_mode(arg) {
            Some(Mode::Normal) => Action::Command(Command::SetMode { mode: Mode::Normal }),
            Some(requested) => {
                let auto_ok = self
                    .sandbox
                    .as_ref()
                    .is_some_and(SandboxStatus::allows_auto_modes);
                if auto_ok {
                    Action::Command(Command::SetMode { mode: requested })
                } else {
                    self.timeline.items.push(ConvItem::Notice(
                        "auto-accept modes are unavailable without OS confinement".into(),
                    ));
                    Action::None
                }
            }
            None => {
                self.timeline.items.push(ConvItem::Notice(format!(
                    "unknown mode '{arg}' — try: normal, auto-accept-edits, auto"
                )));
                Action::None
            }
        }
    }

    /// Handle `/effort [level]`: no arg opens the picker; an arg sets the level
    /// directly, validated against the model's declared levels (P-9).
    fn effort_command(&mut self, arg: &str) -> Action {
        let arg = arg.trim();
        if arg.is_empty() {
            self.open_effort_picker();
            return Action::None;
        }
        match Effort::parse(arg) {
            Some(level) if self.effort_levels.contains(&level) => {
                Action::Command(Command::SetEffort { effort: level })
            }
            Some(level) if self.effort_levels.is_empty() => {
                self.timeline.items.push(ConvItem::Notice(format!(
                    "this model has no reasoning-effort control (ignoring '{level}')"
                )));
                Action::None
            }
            Some(level) => {
                let offered = self
                    .effort_levels
                    .iter()
                    .map(Effort::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                self.timeline.items.push(ConvItem::Notice(format!(
                    "this model does not offer '{level}' — try: {offered}"
                )));
                Action::None
            }
            None => {
                self.timeline.items.push(ConvItem::Notice(format!(
                    "unknown effort '{arg}' — try low, medium, high, or max"
                )));
                Action::None
            }
        }
    }

    /// Toggle the most recent reasoning trail open/closed (the expand
    /// affordance, Design §4.4).
    pub(super) fn toggle_reasoning(&mut self) {
        for item in self.timeline.items.iter_mut().rev() {
            if let ConvItem::Reasoning { expanded, .. } = item {
                *expanded = !*expanded;
                return;
            }
        }
    }

    /// The project's `.agents/` directory, derived from the sessions dir
    /// (`<root>/.agents/sessions`).
    fn agents_dir(&self) -> PathBuf {
        self.sessions_dir
            .parent()
            .map_or_else(|| self.sessions_dir.clone(), Path::to_path_buf)
    }

    /// `/config` — edit the project `.agents/config.toml` in `$EDITOR` (C-5).
    /// Seeds it from the init template (same content `emberly init` writes) if
    /// the project has none yet (C-1/C-2); the write lands in the project tier.
    fn edit_config(&mut self) -> Action {
        match crate::edit::config_target(&self.agents_dir(), &self.config_template) {
            Ok((path, existed)) => {
                // Provenance before the edit (C-3): existing project value vs a
                // fresh override seeded from the defaults. Edits land here (C-1).
                self.timeline.items.push(ConvItem::Notice(if existed {
                    format!("editing your project config — {}", path.display())
                } else {
                    format!(
                        "no project config yet — created {} from the template; \
                         your edits override the defaults",
                        path.display()
                    )
                }));
                Action::EditFile(path)
            }
            Err(e) => {
                self.timeline
                    .items
                    .push(ConvItem::Notice(format!("could not prepare config: {e}")));
                Action::None
            }
        }
    }

    /// `/prompt [name]` — edit a prompt file (`system` | `compact`, default
    /// `system`) in `$EDITOR` (C-5). Seeds from the baked-in default (C-1) if
    /// the project has no override yet.
    fn edit_prompt(&mut self, name: &str) -> Action {
        match crate::edit::prompt_target(&self.agents_dir(), name.trim()) {
            Ok((path, existed)) => {
                let shown = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("prompt");
                // Provenance before the edit (C-3): an existing project override
                // vs a fresh copy of the baked-in default. Edits land here (C-1).
                self.timeline.items.push(ConvItem::Notice(if existed {
                    format!("editing your project '{shown}' prompt — {}", path.display())
                } else {
                    format!(
                        "no project '{shown}' prompt yet — created {} from the baked-in \
                         default; your edits override it",
                        path.display()
                    )
                }));
                Action::EditFile(path)
            }
            Err(msg) => {
                self.timeline.items.push(ConvItem::Notice(msg));
                Action::None
            }
        }
    }

    /// Report an `$EDITOR` handoff's outcome as a timeline notice (C-5).
    pub fn note_edit(&mut self, path: &Path, status: crate::edit::EditStatus) {
        use crate::edit::EditStatus;
        let message = match status {
            // A ReloadConfig follows (C-5), which reports what actually changed.
            EditStatus::Edited => format!("edited {}", path.display()),
            EditStatus::NoEditor => {
                "no editor configured — set $EDITOR or $VISUAL, then try again".to_string()
            }
            EditStatus::Failed(why) => format!("editor failed: {why}"),
        };
        self.timeline.items.push(ConvItem::Notice(message));
    }

    /// Run a typed `/name` command; unknown names surface a calm notice.
    pub(super) fn run_slash(&mut self, name: &str) -> Action {
        match commands::parse_slash(name) {
            // The four argument-taking commands (C-6/P-9). Each opens its picker
            // when the argument is absent; `run_command` is the no-argument path
            // the palette and keybindings take, so it agrees by construction.
            Slash::Command(spec, args) => match spec.cmd {
                AppCommand::Model => self.run_model_command(args),
                AppCommand::Prompt => self.edit_prompt(args),
                AppCommand::Effort => self.effort_command(args),
                AppCommand::CycleMode => self.mode_command(args),
                cmd => self.run_command(cmd),
            },
            Slash::Unknown(name) => {
                self.timeline.items.push(ConvItem::Notice(format!(
                    "unknown command: /{name} — Ctrl-P lists commands"
                )));
                Action::None
            }
        }
    }

    /// `/model <profile> [model]` — switch the active provider profile (and
    /// optionally the model) for subsequent turns (C-6). Gated at idle, like
    /// `/new`: a switch applies to the next turn.
    fn run_model_command(&mut self, args: &str) -> Action {
        let mut parts = args.split_whitespace();
        // No arguments → open the picker (browsing is fine even mid-turn).
        let Some(profile) = parts.next() else {
            self.open_model_picker();
            return Action::None;
        };
        if self.anim.busy {
            self.timeline.items.push(ConvItem::Notice(
                "finish or cancel the current turn before switching models".into(),
            ));
            return Action::None;
        }
        let model = parts.next().map(str::to_string);
        Action::Command(Command::SwitchModel {
            profile: profile.to_string(),
            model,
        })
    }

    /// Execute a command from the palette, a slash command, or a keybinding.
    /// One place maps each [`AppCommand`] to its effect.
    pub fn run_command(&mut self, cmd: AppCommand) -> Action {
        match cmd {
            AppCommand::Help => {
                self.open_text_overlay("commands", help_text());
                Action::None
            }
            AppCommand::View => {
                match self.last_assistant_text() {
                    Some(text) => self.open_text_overlay("message", text),
                    None => self.open_text_overlay("message", "(no assistant message yet)"),
                }
                Action::None
            }
            AppCommand::Diff => {
                self.open_last_diff();
                Action::None
            }
            AppCommand::Files => {
                self.open_text_overlay("modified files", self.files_text());
                Action::None
            }
            AppCommand::Session => {
                let rows = self.session_rows();
                self.push_overlay(Overlay {
                    title: "sessions".into(),
                    content: OverlayContent::Sessions { rows, selected: 0 },
                    scroll: 0,
                });
                Action::None
            }
            AppCommand::NewSession => {
                // A switch resets the conversation, so refuse mid-turn — the
                // frontend gates it here rather than dropping it in the engine.
                if self.anim.busy {
                    self.timeline.items.push(ConvItem::Notice(
                        "finish or cancel the current turn before starting a new session".into(),
                    ));
                    Action::None
                } else {
                    Action::NewSession
                }
            }
            AppCommand::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                Action::None
            }
            AppCommand::CycleMode => {
                // Cycle to the next tier; the engine is the single gate that
                // resolves it against confinement (Tech Spec §6.6) and emits a
                // ModeChanged on success or an explanatory Notice on refusal, so
                // the frontend just proposes the next tier.
                let next = match self.mode {
                    emberly_core::Mode::Normal => emberly_core::Mode::AutoAcceptEdits,
                    emberly_core::Mode::AutoAcceptEdits => emberly_core::Mode::Auto,
                    emberly_core::Mode::Auto => emberly_core::Mode::Normal,
                };
                Action::Command(Command::SetMode { mode: next })
            }
            AppCommand::Model => {
                self.open_model_picker();
                Action::None
            }
            AppCommand::Effort => {
                self.open_effort_picker();
                Action::None
            }
            AppCommand::Memory => self.open_memory_inspector(),
            AppCommand::Skills => self.open_skills_inspector(),
            AppCommand::Config => self.edit_config(),
            AppCommand::Prompt => self.edit_prompt("system"),
            AppCommand::Reload => Action::Command(Command::ReloadConfig),
            AppCommand::Compact => Action::Command(Command::Compact),
            AppCommand::Cancel => Action::Command(Command::Cancel),
            AppCommand::Quit => Action::Quit,
        }
    }
}
