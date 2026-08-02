//! In-session reconfiguration (C-5, C-6): switching model or
//! effort, and reloading config from disk.
//!
//! Part of the `Engine` inherent impl, split out of one 2,700-line
//! block; the engine is still the single owner of this state.

use super::*;

impl Engine {
    /// Switch the active provider/model for subsequent turns (C-6). Runs at
    /// this clean boundary (idle) and never rewrites prior turns; a failure to
    /// build (unknown profile, missing key) is a harness-world error and the
    /// current model stays active.
    pub(super) async fn switch_model(&mut self, profile: String, model: Option<String>) {
        let Some(factory) = self.provider.factory.clone() else {
            self.emit(UiEvent::Notice {
                message: "switching models is not available in this session".into(),
            })
            .await;
            return;
        };
        let model = model.unwrap_or_else(|| self.provider.model.clone());
        match factory.build(&profile, &model) {
            Ok(choice) => {
                if choice.profile == self.provider.label && choice.model == self.provider.model {
                    return; // no-op: already active
                }
                self.provider.client = choice.provider;
                self.provider.label = choice.profile.clone();
                self.provider.model = choice.model.clone();
                self.write_transcript(TranscriptEvent::ModelSwitch {
                    provider: choice.profile.clone(),
                    model: choice.model.clone(),
                });
                self.emit(UiEvent::ModelChanged {
                    provider: choice.profile.clone(),
                    model: choice.model.clone(),
                })
                .await;
                self.emit(UiEvent::Notice {
                    message: format!("switched to {} / {}", choice.profile, choice.model),
                })
                .await;
                // Re-seed the reasoning effort to the new model's default — its
                // available levels and default differ per model (P-9). Always
                // re-emit so the sidebar/picker track the new model's levels
                // even when the default happens to match.
                self.provider.effort = self.provider.client.model_info().default_effort;
                self.emit_effort().await;
            }
            Err(why) => {
                self.emit(UiEvent::HarnessError {
                    what: format!("could not switch to '{profile}'"),
                    why,
                    next: format!(
                        "staying on {} / {}",
                        self.provider.label, self.provider.model
                    ),
                })
                .await;
            }
        }
    }

    /// Set the session reasoning effort for subsequent turns (C-6/P-9). Logged
    /// to the transcript (HC-7) and announced — never silent. A model with no
    /// effort control still accepts the setting; the adapter drops it at the
    /// wire (P-9), so setting it is never an error.
    pub(super) async fn set_effort(&mut self, effort: Effort) {
        // The engine is the authority on what a model supports (P-9): decline
        // (calmly, never an error) when the model has no control or the level
        // isn't offered, so no frontend can announce a change that won't happen.
        let levels = self.provider.client.model_info().effort_levels;
        if levels.is_empty() {
            self.emit(UiEvent::Notice {
                message: "this model has no reasoning-effort control".into(),
            })
            .await;
            return;
        }
        if !levels.contains(&effort) {
            self.emit(UiEvent::Notice {
                message: format!("this model does not offer '{effort}' reasoning effort"),
            })
            .await;
            return;
        }
        if self.provider.effort == Some(effort) {
            return; // no-op: already active
        }
        self.provider.effort = Some(effort);
        self.write_transcript(TranscriptEvent::EffortChange { effort });
        self.emit_effort().await;
        self.emit(UiEvent::Notice {
            message: format!("reasoning effort set to {effort}"),
        })
        .await;
    }

    /// Emit the current effort and the active model's available levels (P-9), so
    /// the sidebar and the effort picker stay in sync with the model.
    pub(super) async fn emit_effort(&self) {
        self.emit(UiEvent::EffortChanged {
            effort: self.provider.effort,
            available: self.provider.client.model_info().effort_levels,
        })
        .await;
    }

    /// Re-read config + prompts from disk and apply the live pieces to the
    /// running session (C-5): the system/compact prompts and the provider
    /// profile set (so a newly-added profile is switchable and shows in the
    /// picker). The active provider/model is left as-is — use `/model` to
    /// switch. Restart-only changes are named, not applied. Applies to
    /// subsequent turns; never rewrites prior turns or the transcript.
    pub(super) async fn reload_config(&mut self) {
        let Some(reloader) = self.config_reloader.clone() else {
            self.emit(UiEvent::Notice {
                message: "config reload is not available in this session".into(),
            })
            .await;
            return;
        };
        let reloaded = match reloader.reload() {
            Ok(reloaded) => reloaded,
            Err(why) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not reload config".into(),
                    why,
                    next: "keeping the current config".into(),
                })
                .await;
                return;
            }
        };

        let mut changed = Vec::new();
        if reloaded.system != self.provider.system {
            self.provider.system = reloaded.system;
            changed.push("system prompt");
        }
        if reloaded.summary_prompt != self.context.summary_prompt {
            self.context.summary_prompt = reloaded.summary_prompt;
            changed.push("compact prompt");
        }
        let old_profiles = self
            .provider
            .factory
            .as_ref()
            .map(|factory| factory.profiles())
            .unwrap_or_default();
        let profile_names_changed = reloaded.profiles != old_profiles;
        if profile_names_changed {
            self.emit(UiEvent::ProfilesChanged {
                profiles: reloaded.profiles.clone(),
            })
            .await;
        }
        // A profile's content (base_url, auth, model metadata) can change
        // without its name changing (e.g. filling in a built-in placeholder
        // profile like `openai`) — `provider_config_changed` catches that so
        // the notice isn't silent about it (C-5).
        if profile_names_changed || reloaded.provider_config_changed {
            changed.push("provider profiles");
        }
        self.provider.factory = Some(reloaded.provider_factory);

        // The config-configured default `provider =` / `model =` selection
        // can change independently of the profile set above (e.g. the user
        // just points an already-configured profile as the default). Never
        // auto-switches the active session (C-6) — surfaced as a concrete
        // `/model` command instead.
        let mut model_hint = None;
        if reloaded.configured_provider != self.provider.configured_provider
            || reloaded.configured_model != self.provider.configured_model
        {
            self.provider.configured_provider = reloaded.configured_provider;
            self.provider.configured_model = reloaded.configured_model;
            changed.push("provider selection");
            if let (Some(provider), Some(model)) = (
                &self.provider.configured_provider,
                &self.provider.configured_model,
            ) {
                model_hint = Some(format!("run /model {provider} {model} to switch"));
            }
        }

        if reloaded.tool_explanations != self.tool_explanations {
            self.tool_explanations = reloaded.tool_explanations;
            changed.push("tool explanations");
        }
        if reloaded.loop_config != self.guardrail.config {
            self.guardrail.config = reloaded.loop_config;
            changed.push("loop guardrail");
        }
        if reloaded.completion_config != self.completion.config {
            self.completion.config = reloaded.completion_config;
            changed.push("completion gate");
        }
        if reloaded.completion_checks != self.completion.checks {
            self.completion.checks = reloaded.completion_checks;
            changed.push("completion checks");
        }
        if reloaded.truncate != self.truncate {
            self.truncate = reloaded.truncate;
            changed.push("truncation");
        }
        if reloaded.context != self.context.config {
            self.context.config = reloaded.context;
            changed.push("context window");
        }
        if reloaded.image_max_bytes != self.image_max_bytes {
            self.image_max_bytes = reloaded.image_max_bytes;
            changed.push("image size limit");
        }
        if reloaded.document_max_bytes != self.document_max_bytes {
            self.document_max_bytes = reloaded.document_max_bytes;
            changed.push("document size limit");
        }
        if reloaded.memory != self.memory.config {
            self.memory.config = reloaded.memory;
            self.memory.store = build_memory_store(
                self.memory.config.enabled,
                self.memory.user_dir.as_ref(),
                self.memory.project_dir.clone(),
            );
            self.refresh_memory_indexes();
            changed.push("memory");
        }
        if reloaded.skills != self.skills.config {
            self.skills.config = reloaded.skills;
            self.skills.catalog = build_skill_catalog(
                self.skills.config.enabled,
                self.skills.user_dir.as_ref(),
                self.skills.project_dir.clone(),
            );
            self.refresh_skill_catalog();
            changed.push("skills");
        }
        let mut old_tool_names = self.tools.names();
        old_tool_names.sort();
        self.tools = reloaded.tools;
        let mut new_tool_names = self.tools.names();
        new_tool_names.sort();
        if new_tool_names != old_tool_names {
            changed.push("tools");
        }
        if reloaded.rule_specs != self.safety.rule_specs {
            self.safety.rules.reload_config_rules(
                reloaded.rule_specs.clone(),
                self.safety.sandbox.bash_allowlist_active(),
            );
            self.safety.rule_specs = reloaded.rule_specs;
            changed.push("permission rules");
        }

        let mut message = if changed.is_empty() {
            "reloaded config — no live changes".to_string()
        } else {
            format!("reloaded: {}", changed.join(", "))
        };
        if let Some(hint) = model_hint {
            message.push_str("; ");
            message.push_str(&hint);
        } else if changed.contains(&"provider profiles") {
            message.push_str("; run /model to use it");
        }
        for note in &reloaded.restart_notes {
            message.push_str("; ");
            message.push_str(note);
        }
        for warning in &reloaded.warnings {
            message.push_str("; ");
            message.push_str(warning);
        }
        self.emit(UiEvent::Notice { message }).await;
    }
}
