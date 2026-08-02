//! Applying [`UiEvent`]s to the view model — the engine-to-frontend
//! half of the channel boundary (A-1).
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// Seed the conversation timeline from a resumed transcript (Tech Spec
    /// §3.3), so the restored session shows its history rather than a blank
    /// pane. Maps durable records to display items; non-conversation records
    /// (session_start/title/permission/end) are skipped.
    pub fn seed_history(&mut self, records: &[TranscriptRecord]) {
        for record in records {
            match &record.event {
                TranscriptEvent::UserMessage { text, .. } => {
                    self.conversation.push(ConvItem::User(text.clone()));
                }
                TranscriptEvent::AssistantMessage { text, reasoning } => {
                    // Replay a recorded reasoning trail (collapsed) unless the
                    // view hides it (P-10, Design §4.4).
                    if let Some(reasoning) = reasoning {
                        if self.reasoning_view != ReasoningView::Hidden {
                            self.conversation.push(ConvItem::Reasoning {
                                text: reasoning.clone(),
                                expanded: self.reasoning_view == ReasoningView::Expanded,
                            });
                        }
                    }
                    if !text.is_empty() {
                        self.conversation.push(ConvItem::Assistant(text.clone()));
                    }
                }
                TranscriptEvent::ToolCall {
                    call_id,
                    tool,
                    args,
                } => {
                    // Reconstruct a readable label from the recorded args.
                    let summary = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|c| format!("run: {c}"))
                        .or_else(|| {
                            args.get("path")
                                .and_then(|v| v.as_str())
                                .map(|p| format!("{tool} {p}"))
                        })
                        .unwrap_or_else(|| tool.clone());
                    // The explanation (T-9) rides in the recorded args, so a
                    // replayed session shows the same caption a live one did.
                    let explanation = args
                        .get("explanation")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    self.conversation.push(ConvItem::Tool {
                        call_id: call_id.clone(),
                        tool: tool.clone(),
                        summary,
                        explanation,
                        done: None,
                        result: None,
                        preview: None,
                        untrusted: false,
                    });
                }
                TranscriptEvent::ToolResult {
                    call_id,
                    ok,
                    output,
                    ..
                } => {
                    if let Some(ConvItem::Tool { done, preview, .. }) = self.find_tool_mut(call_id)
                    {
                        *done = Some(*ok);
                        *preview = Some(output.clone());
                    }
                }
                TranscriptEvent::Compaction { summary, .. } => {
                    self.conversation
                        .push(ConvItem::Notice(format!("compacted — {summary}")));
                }
                _ => {}
            }
        }
        // Resumed content scrolls off the top; start pinned to the latest.
        self.scroll = 0;
    }

    /// Fold one engine event into the view-model. Pure over `self` — no I/O — so
    /// a canned event sequence can be asserted against the resulting state.
    pub fn apply_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::AssistantDelta { text } => {
                if self.streaming {
                    if let Some(ConvItem::Assistant(buf)) = self.conversation.last_mut() {
                        buf.push_str(&text);
                        return;
                    }
                }
                // The answer is starting: settle the just-streamed reasoning
                // trail to its collapsed line unless the view pins it open
                // (Design §4.4).
                if self.reasoning_view == ReasoningView::Collapsed {
                    if let Some(ConvItem::Reasoning { expanded, .. }) = self.conversation.last_mut()
                    {
                        *expanded = false;
                    }
                }
                self.streaming = true;
                self.conversation.push(ConvItem::Assistant(text));
            }
            UiEvent::ReasoningDelta { text } => {
                // `hidden` is a view choice: skip the trail but the engine still
                // records the trace to the transcript (P-10, Design §4.4).
                if self.reasoning_view == ReasoningView::Hidden {
                    return;
                }
                if let Some(ConvItem::Reasoning { text: buf, .. }) = self.conversation.last_mut() {
                    buf.push_str(&text);
                } else {
                    // Stream in place while thinking; expanded until the answer
                    // begins (then settled), or always when the view pins it.
                    self.conversation.push(ConvItem::Reasoning {
                        text,
                        expanded: true,
                    });
                }
            }
            UiEvent::AssistantDone => self.streaming = false,
            UiEvent::TurnEnded => {
                self.busy = false;
                self.streaming = false;
            }
            UiEvent::ToolStarted {
                call_id,
                tool,
                summary,
                explanation,
            } => {
                self.streaming = false;
                self.conversation.push(ConvItem::Tool {
                    call_id,
                    tool,
                    summary,
                    explanation,
                    done: None,
                    result: None,
                    preview: None,
                    untrusted: false,
                });
            }
            UiEvent::ToolFinished {
                call_id,
                ok,
                summary,
                preview,
                untrusted,
            } => {
                if let Some(ConvItem::Tool {
                    done,
                    result,
                    preview: p,
                    untrusted: u,
                    ..
                }) = self.find_tool_mut(&call_id)
                {
                    *done = Some(ok);
                    // Keep the descriptive label; record the result status and
                    // a preview of the output separately.
                    *result = (!summary.is_empty()).then_some(summary);
                    *p = (!preview.is_empty()).then_some(preview);
                    *u = untrusted;
                }
            }
            UiEvent::PermissionRequest { id, rendering } => {
                self.pending_permission = Some((id, rendering));
                self.permission_scroll = 0; // start every prompt at the top
            }
            UiEvent::AskUserRequest {
                id,
                question,
                options,
            } => {
                self.streaming = false;
                self.pending_ask = Some(AskPrompt::new(id, question, options));
            }
            UiEvent::LoopHalted { reason } => {
                self.streaming = false;
                self.pending_loop_halt = Some(LoopHaltPrompt::new(reason));
            }
            UiEvent::CompletionGateHalted { failing, attempts } => {
                self.streaming = false;
                self.pending_completion_gate = Some(CompletionGatePrompt::new(failing, attempts));
            }
            UiEvent::ContextUsage { pct, tokens } => {
                self.context_pct = pct;
                self.context_tokens = tokens;
            }
            UiEvent::CostEstimate { usd, .. } => {
                self.cost_usd = usd;
                self.cost_known = true;
            }
            UiEvent::SessionUsage { usage } => self.session_usage = usage,
            UiEvent::SandboxStatus { status } => self.sandbox = Some(status),
            UiEvent::ModeChanged { mode } => self.mode = mode,
            UiEvent::ModelChanged { provider, model } => {
                // Sidebar reflects the new provider/model; the engine also emits
                // a Notice, so the switch is never silent (Design §3.1).
                self.session.provider = provider;
                self.session.model = model;
            }
            UiEvent::EffortChanged { effort, available } => {
                self.effort = effort;
                self.effort_levels = available;
            }
            UiEvent::ProfilesChanged { profiles } => {
                // A `/config` reload changed the provider set; refresh the
                // picker's list (C-5).
                self.profiles = profiles;
            }
            UiEvent::Notice { message } => self.conversation.push(ConvItem::Notice(message)),
            UiEvent::HarnessError { what, why, next } => {
                self.conversation
                    .push(ConvItem::Notice(format!("error: {what} — {why}. {next}")));
            }
            UiEvent::Retrying {
                attempt,
                max_attempts,
                delay_ms,
                reason,
            } => {
                self.conversation.push(ConvItem::Notice(format!(
                    "retrying ({attempt}/{max_attempts}) in {delay_ms}ms — {reason}"
                )));
            }
            UiEvent::SessionMeta {
                session_id,
                title,
                provider,
                model,
                project_root,
            } => {
                self.session = SessionInfo {
                    session_id,
                    title,
                    provider,
                    model,
                    project_root,
                };
            }
            UiEvent::FileModified { path, adds, dels } => {
                self.last_modified = Some(path.clone());
                let is_new = self.upsert_modified(path, adds, dels);
                // A newly-landed entry gets a brief settle highlight (Design §6.4).
                if is_new && self.motion {
                    self.sidebar_settle = SETTLE_FRAMES;
                }
            }
            UiEvent::FileDiff { path, unified } => {
                // Show it inline when the edit executes (Design §4.2) …
                self.conversation.push(ConvItem::Diff {
                    unified: unified.clone(),
                });
                // … and keep the latest per file for the on-demand overlay.
                self.latest_diffs.insert(path, unified);
            }
            UiEvent::CompactionStatus { message } => {
                self.conversation.push(ConvItem::Notice(message));
            }
            UiEvent::TaskListUpdated { items } => {
                self.tasks = items.clone();
                self.conversation.push(ConvItem::TaskList { items });
            }
            UiEvent::MemoryStatus { user, project } => {
                self.memory_user = user;
                self.memory_project = project;
            }
            UiEvent::CompletionStatus { checks } => {
                self.completion_status = checks;
            }
            UiEvent::SkillsAvailable { skills } => {
                self.skills = skills;
            }
            UiEvent::MemoryEntries { user, project } => {
                self.apply_memory_entries(user, project);
            }
            UiEvent::MemoryBody { scope, name, body } => {
                self.apply_memory_body(scope, &name, body);
            }
            UiEvent::SkillBody {
                name,
                origin,
                body,
                resources,
            } => {
                self.apply_skill_body(&name, origin, body, &resources);
            }
            // `#[non_exhaustive]`: unknown future events are ignored, not fatal.
            _ => {}
        }
    }

    fn find_tool_mut(&mut self, call_id: &ToolCallId) -> Option<&mut ConvItem> {
        self.conversation
            .iter_mut()
            .rev()
            .find(|item| matches!(item, ConvItem::Tool { call_id: c, .. } if c == call_id))
    }

    /// Insert or update a modified-file entry; returns `true` if it was new.
    fn upsert_modified(&mut self, path: String, adds: u32, dels: u32) -> bool {
        if let Some(existing) = self.modified_files.iter_mut().find(|f| f.path == path) {
            existing.adds = adds;
            existing.dels = dels;
            false
        } else {
            self.modified_files.push(ModifiedFile { path, adds, dels });
            true
        }
    }
}
