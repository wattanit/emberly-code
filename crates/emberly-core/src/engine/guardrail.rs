//! The two things that can stop a turn claiming it is done: the
//! loop-breaking guardrail (S-5) and the completion gate (S-6).
//!
//! Part of the `Engine` inherent impl, split out of one 2,700-line
//! block; the engine is still the single owner of this state.

use super::*;

impl Engine {
    /// Fold the just-completed tool-call turn into the loop-guardrail state and
    /// decide whether to halt (S-5, Tech Spec §7). Returns the halt reason when
    /// the loop is re-treading without progress, else `None`. Consumes the
    /// turn's observation.
    pub(super) fn evaluate_loop(&mut self) -> Option<String> {
        if !self.loop_config.enabled {
            return None;
        }
        let obs = std::mem::take(&mut self.turn_obs);
        // A turn with no tool calls can't loop; treat it as progress-neutral.
        if obs.calls.is_empty() {
            return None;
        }
        // Order-independent tool signature.
        let mut parts: Vec<String> = obs
            .calls
            .iter()
            .map(|(name, args)| format!("{name}\u{0}{args}"))
            .collect();
        parts.sort();
        let tool_sig = stable_hash(&parts.join("\n"));
        let result_hash = stable_hash(&obs.result_content);

        // Progress = a newly-modified file OR a not-seen-before result.
        let new_file = obs.files.iter().any(|f| !self.loop_seen_files.contains(f));
        let new_result = !self.loop_seen_results.contains(&result_hash);
        for f in obs.files {
            self.loop_seen_files.insert(f);
        }
        self.loop_seen_results.insert(result_hash);

        if new_file || new_result {
            self.reset_loop_window();
            self.loop_last_sig = Some(tool_sig);
            return None;
        }

        // No progress this turn.
        self.loop_no_progress_streak += 1;
        if self.loop_last_sig == Some(tool_sig) {
            self.loop_same_sig_streak += 1;
        } else {
            self.loop_same_sig_streak = 1;
        }
        self.loop_last_sig = Some(tool_sig);

        let cfg = self.loop_config;
        if self.loop_same_sig_streak >= cfg.repeat_window {
            Some("the last few steps repeated without progress".into())
        } else if self.loop_no_progress_streak >= cfg.max_no_progress_turns {
            Some("several steps in a row made no progress".into())
        } else {
            None
        }
    }

    /// Reset the no-progress counters (S-5). Called on genuine progress and when
    /// the user chooses to resume, so "keep going" doesn't instantly re-trip.
    pub(super) fn reset_loop_window(&mut self) {
        self.loop_same_sig_streak = 0;
        self.loop_no_progress_streak = 0;
        self.loop_last_sig = None;
    }

    /// Surface the halt (harness voice) and park until the user decides
    /// (S-5, Design §8.5): resume / stop / steer. Cancel or a departed frontend
    /// resolves to stop — the guardrail never quietly resumes. Records one
    /// `loop_halt` transcript event with the chosen resolution.
    pub(super) async fn await_loop_resolution(
        &mut self,
        commands_rx: &mut mpsc::Receiver<Command>,
        reason: String,
    ) -> LoopResolution {
        self.emit(UiEvent::LoopHalted {
            reason: reason.clone(),
        })
        .await;
        let resolution = loop {
            match commands_rx.recv().await {
                Some(Command::ResolveLoop { resolution }) => break resolution,
                // Esc/Ctrl-C while halted = stop here.
                Some(Command::Cancel) => break LoopResolution::Stop,
                // A mode toggle applies immediately; keep waiting for a decision.
                Some(Command::SetMode { mode }) => self.set_mode(mode).await,
                // Queue a compaction for after we resume (if we do).
                Some(Command::Compact) => self.request_compaction(CompactTrigger::Manual),
                // Strays (permission/ask answers with no pending prompt): ignore.
                Some(_) => {}
                // Frontend gone: stop, fail-safe (never spin unattended).
                None => break LoopResolution::Stop,
            }
        };
        self.write_transcript(TranscriptEvent::LoopHalt {
            reason,
            resolution: Some(resolution_label(&resolution)),
        });
        resolution
    }

    /// Run one registered completion check (S-6, Tech Spec §7, §6.1). Spawns
    /// through the identical confined path as the `bash` tool
    /// (`sandbox().bash_invocation()`, env scrubbed to the same allowlist,
    /// its own process group, the same default timeout) but **never** raises a
    /// `PermissionRequest` — the deliberate divergence from `bash` (S-6 honesty
    /// clause: contained, not re-prompted, because registering the check in
    /// trusted config is the authorization).
    async fn run_completion_check(&self, check: &CompletionCheck) -> CheckResult {
        let invocation = self
            .sandbox_spawn
            .bash_invocation(&check.command, &self.project_root);
        let mut command = tokio::process::Command::new(&invocation.program);
        command.args(&invocation.args);
        command.current_dir(&self.project_root);
        command.env_clear();
        for key in emberly_tools::DEFAULT_ENV_ALLOWLIST {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        for (key, value) in &invocation.extra_env {
            command.env(key, value);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        let timeout = std::time::Duration::from_secs(emberly_tools::DEFAULT_TIMEOUT_SECS);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return CheckResult {
                    name: check.name.clone(),
                    passed: false,
                    reason: format!("failed to start check command: {e}"),
                }
            }
        };
        let mut group_kill = CompletionCheckKillGuard::arm(child.id());

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                group_kill.disarm();
                let exit_code = output.status.code().unwrap_or(-1);
                let passed = output.status.code() == Some(check.expect_exit);
                let reason = if passed {
                    format!("exit {exit_code}")
                } else {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let mut combined = format!("exit code: {exit_code}\n");
                    if !stdout.is_empty() {
                        combined.push_str("--- stdout ---\n");
                        combined.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        combined.push_str("--- stderr ---\n");
                        combined.push_str(&stderr);
                    }
                    // Same reduction the tool pipeline applies (§5.3): salient
                    // reduction first, then the size backstop — so a noisy
                    // failing check does not flood the re-opened turn.
                    let reduced = reduce_output("bash", &combined);
                    truncate_output(&reduced.content, &self.truncate).content
                };
                CheckResult {
                    name: check.name.clone(),
                    passed,
                    reason,
                }
            }
            Ok(Err(e)) => CheckResult {
                name: check.name.clone(),
                passed: false,
                reason: format!("check command error: {e}"),
            },
            // Timeout: leave the guard armed — its drop SIGKILLs the whole
            // process group, exactly as a `bash` tool-call timeout does (S-4).
            Err(_elapsed) => CheckResult {
                name: check.name.clone(),
                passed: false,
                reason: format!(
                    "check timed out after {}s and was killed",
                    timeout.as_secs()
                ),
            },
        }
    }

    /// Evaluate the completion gate at a completion attempt — a turn ending
    /// with no tool calls (S-6, Tech Spec §7). The mirror of `evaluate_loop`,
    /// hooking the branch S-5 deliberately ignores (`:1548`). **Inert when no
    /// checks are registered:** returns `Terminate` immediately, zero
    /// behavior change (S-6). Every check that runs is recorded as a
    /// `completion_check` transcript event, win or lose (HC-7).
    pub(super) async fn evaluate_completion_gate(
        &mut self,
        commands_rx: &mut mpsc::Receiver<Command>,
    ) -> CompletionGateOutcome {
        if self.completion_checks.is_empty() {
            return CompletionGateOutcome::Terminate;
        }
        let checks = self.completion_checks.clone();
        let mut all = Vec::with_capacity(checks.len());
        let mut failing = Vec::new();
        for check in &checks {
            let result = self.run_completion_check(check).await;
            self.write_transcript(TranscriptEvent::CompletionCheck {
                name: result.name.clone(),
                passed: result.passed,
                reason: result.reason.clone(),
            });
            if !result.passed {
                failing.push(result.clone());
            }
            all.push(result);
        }
        // Sidebar gate status (Design §8.7, §3.1): the last result per check,
        // win or lose. Only emitted when checks are registered (this branch
        // is unreachable otherwise), so an inert gate never shows the line.
        self.emit(UiEvent::CompletionStatus { checks: all }).await;
        if failing.is_empty() {
            self.completion_attempts = 0;
            return CompletionGateOutcome::Terminate;
        }
        self.completion_attempts += 1;
        // Agent-world content (Design §8.7): the failing checks return to the
        // model as ordinary tool-result-shaped content — the harness does not
        // editorialize; the model reacts and fixes like any tool failure.
        self.push_conversation_message(Message::user_text(render_gate_failure(&failing)));
        self.emit_context_usage().await;

        if self.completion_attempts < self.completion_config.max_attempts {
            return CompletionGateOutcome::ReOpen;
        }

        // S-6: the bounded number of failed attempts is reached — hand
        // control to the user (resume / steer / stop / finish), exactly as
        // S-5 does at its own bound. Never spin on.
        match self
            .await_completion_resolution(commands_rx, failing, self.completion_attempts)
            .await
        {
            GateResolution::Resume => {
                self.completion_attempts = 0;
                CompletionGateOutcome::ReOpen
            }
            GateResolution::Steer(text) => {
                self.completion_attempts = 0;
                self.record_user_message(&text);
                self.push_conversation_message(Message::user_text(text));
                self.emit_context_usage().await;
                CompletionGateOutcome::ReOpen
            }
            // `Stop` ends the turn with the gate still unsatisfied; `Finish`
            // ends it as the user's explicit override (Design §8.7) — both
            // just terminate the turn here, the transcript event written by
            // `await_completion_resolution` is what distinguishes them.
            GateResolution::Stop | GateResolution::Finish => CompletionGateOutcome::Terminate,
        }
    }

    /// Surface the completion-gate halt (harness voice) and park until the
    /// user decides (S-6, Design §8.7): resume / stop / steer / finish.
    /// Cancel or a departed frontend resolves to stop — the gate never
    /// quietly resumes or grants the override on its own. Records one
    /// `completion_gate_halt` transcript event with the chosen resolution;
    /// `override_finish` is `true` only for `Finish` (Design §8.7 — never
    /// presented as though the checks passed).
    async fn await_completion_resolution(
        &mut self,
        commands_rx: &mut mpsc::Receiver<Command>,
        failing: Vec<CheckResult>,
        attempts: usize,
    ) -> GateResolution {
        self.emit(UiEvent::CompletionGateHalted {
            failing: failing.clone(),
            attempts,
        })
        .await;
        let resolution = loop {
            match commands_rx.recv().await {
                Some(Command::ResolveCompletionGate { resolution }) => break resolution,
                // Esc/Ctrl-C while halted = stop here.
                Some(Command::Cancel) => break GateResolution::Stop,
                // A mode toggle applies immediately; keep waiting for a decision.
                Some(Command::SetMode { mode }) => self.set_mode(mode).await,
                // Queue a compaction for after we resume (if we do).
                Some(Command::Compact) => self.request_compaction(CompactTrigger::Manual),
                // Strays (permission/ask answers with no pending prompt): ignore.
                Some(_) => {}
                // Frontend gone: stop, fail-safe (never spin unattended).
                None => break GateResolution::Stop,
            }
        };
        let override_finish = matches!(resolution, GateResolution::Finish);
        self.write_transcript(TranscriptEvent::CompletionGateHalt {
            failing,
            attempts,
            resolution: Some(gate_resolution_label(&resolution)),
            override_finish,
        });
        resolution
    }
}
