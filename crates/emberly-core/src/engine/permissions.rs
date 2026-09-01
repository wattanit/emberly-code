//! The permission seam (Requirements §6): consulting the rule
//! engine, prompting, recording the answer, and persisting a grant.
//!
//! Part of the `Engine` inherent impl; the engine is the sole owner of the
//! state these methods touch.

use super::*;

impl Engine {
    /// Resolve a user's answer to a pending prompt: record it, apply any grant
    /// (session or persisted), and reply to the blocked tool. Unknown ids are
    /// ignored (a stray or already-answered prompt).
    pub(super) async fn answer_permission(
        &mut self,
        id: PermissionId,
        decision: crate::types::PermissionDecision,
        pending: &mut Vec<PendingAsk>,
    ) {
        let Some(pos) = pending.iter().position(|p| p.id == id) else {
            return;
        };
        let PendingAsk { request, reply, .. } = pending.swap_remove(pos);

        // A grant widens future asks. HC-4: outside-root actions can never be
        // turned into a rule (only ever allowed once), so a session/project
        // grant is applied only for in-root actions.
        if decision.is_allow() && !request.outside_root {
            match decision {
                crate::types::PermissionDecision::AllowForSession => {
                    self.grant_for_session(&request);
                }
                crate::types::PermissionDecision::AlwaysAllowInProject => {
                    self.grant_for_session(&request);
                    self.persist_project_grant(&request).await;
                }
                _ => {}
            }
        }

        let executed = decision.is_allow().then(|| request.summary.clone());
        self.write_transcript(TranscriptEvent::PermissionDecision {
            id,
            decision,
            executed,
        });
        let outcome = if decision.is_allow() {
            PermissionOutcome::Allow
        } else {
            PermissionOutcome::Deny
        };
        let _ = reply.send(outcome);
    }

    /// Change the auto-accept mode (Requirements §6.4, §6.7). Auto tiers are
    /// gated on active confinement by [`Mode::resolve`]; a refused escalation
    /// leaves the mode unchanged and explains why (the type system, not this
    /// method, guarantees an auto mode is never entered while degraded).
    pub(super) async fn set_mode(&mut self, requested: Mode) {
        match Mode::resolve(requested, &self.safety.sandbox) {
            Ok(mode) => {
                if mode == self.safety.mode {
                    return;
                }
                self.safety.mode = mode;
                self.write_transcript(TranscriptEvent::ModeChange { mode });
                self.emit(UiEvent::ModeChanged { mode }).await;
            }
            Err(unavailable) => {
                self.emit(UiEvent::Notice {
                    message: format!(
                        "auto-accept modes need OS confinement — {} (staying in {})",
                        unavailable.reason,
                        mode_label(self.safety.mode),
                    ),
                })
                .await;
            }
        }
    }

    /// Add an in-memory session grant from an approved request (Requirements
    /// §6.6): a bash command prefix, or a per-tool allow for file writes/edits.
    fn grant_for_session(&mut self, request: &PermissionRequest) {
        self.safety.rules.add_session_grant(grant_rule(request));
    }

    /// Persist an approved request as a project rule in `.agents/permissions.toml`
    /// and show the user the exact line written (Requirements §6.6). Best-effort:
    /// a write failure degrades to a session-only grant with a notice, never a
    /// crash (HC-7).
    async fn persist_project_grant(&mut self, request: &PermissionRequest) {
        let path = self.project_root.join(".agents").join("permissions.toml");
        let Some(block) = grant_rule(request).to_toml_block() else {
            // The rule cannot be written as a block that reads back. Appending
            // it anyway would leave a permissions.toml that fails to parse, and
            // a malformed file is ignored whole on load — every project rule
            // gone, `deny`s included. Degrade instead.
            self.emit(UiEvent::Notice {
                message: "couldn't express that as a permissions.toml rule; \
                          allowed for this session only"
                    .to_string(),
            })
            .await;
            return;
        };
        match append_rule_block(&path, &block) {
            Ok(()) => {
                self.emit(UiEvent::Notice {
                    message: format!("saved to .agents/permissions.toml:\n{}", block.trim_end()),
                })
                .await;
            }
            Err(error) => {
                self.emit(UiEvent::Notice {
                    message: format!(
                        "couldn't write .agents/permissions.toml ({error}); allowed for this session only"
                    ),
                })
                .await;
            }
        }
    }

    /// Consult the rule engine for a tool's ask (Requirements §6): `Allow` runs
    /// silently (no prompt, no UI churn), `Deny` auto-denies with the reason,
    /// and `Ask` raises the prompt as before. Every request and decision is
    /// recorded, including the auto ones (HC-7).
    pub(super) async fn on_permission_ask(
        &mut self,
        ask: PermissionAsk,
        pending: &mut Vec<PendingAsk>,
    ) {
        let request = ask.request;
        let outcome = self
            .safety
            .rules
            .evaluate(&make_query(&request), self.safety.mode);
        let id = self.take_permission_id();
        let rendering = build_rendering(&request, outcome.reason.clone(), ask.on_behalf_of.clone());

        match outcome.decision {
            Decision::Allow => {
                self.write_transcript(TranscriptEvent::PermissionRequest { id, rendering });
                self.write_transcript(TranscriptEvent::PermissionDecision {
                    id,
                    decision: crate::types::PermissionDecision::AllowOnce,
                    executed: Some(request.summary.clone()),
                });
                let _ = ask.reply.send(PermissionOutcome::Allow);
            }
            Decision::Deny => {
                self.write_transcript(TranscriptEvent::PermissionRequest { id, rendering });
                self.write_transcript(TranscriptEvent::PermissionDecision {
                    id,
                    decision: crate::types::PermissionDecision::Deny,
                    executed: None,
                });
                self.emit(UiEvent::Notice {
                    message: format!("auto-denied {}: {}", request.tool, outcome.reason),
                })
                .await;
                let _ = ask.reply.send(PermissionOutcome::Deny);
            }
            Decision::Ask => {
                self.write_transcript(TranscriptEvent::PermissionRequest {
                    id,
                    rendering: rendering.clone(),
                });
                self.emit(UiEvent::PermissionRequest { id, rendering })
                    .await;
                pending.push(PendingAsk {
                    id,
                    reply: ask.reply,
                    request,
                });
            }
        }
    }
}
