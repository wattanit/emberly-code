//! Applying [`UiEvent`]s to the view model — the engine-to-frontend
//! half of the channel boundary (A-1).
//!
//! Part of the `App` inherent impl.

use super::*;

impl App {
    /// Seed the conversation timeline from a resumed transcript (Tech Spec
    /// §3.3), so the restored session shows its history rather than a blank
    /// pane. Maps durable records to display items; non-conversation records
    /// (session_start/title/permission/end) are skipped.
    pub fn seed_history(&mut self, records: &[TranscriptRecord]) {
        for record in records {
            match &record.event {
                TranscriptEvent::UserMessage { text, images, .. } => {
                    self.timeline.items.push(ConvItem::User(text.clone()));
                    for image in images {
                        self.timeline.items.push(ConvItem::Attachment {
                            name: image.name.clone(),
                            width: image.width,
                            height: image.height,
                            format_label: image.format_label.clone(),
                        });
                    }
                }
                TranscriptEvent::AssistantMessage { text, reasoning } => {
                    // Replay a recorded reasoning trail (collapsed) unless the
                    // view hides it (P-10, Design §4.4).
                    if let Some(reasoning) = reasoning {
                        if self.timeline.reasoning != ReasoningView::Hidden {
                            self.timeline.items.push(ConvItem::Reasoning {
                                text: reasoning.clone(),
                                expanded: self.timeline.reasoning == ReasoningView::Expanded,
                            });
                        }
                    }
                    if !text.is_empty() {
                        self.timeline.items.push(ConvItem::Assistant(text.clone()));
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
                    self.timeline.items.push(ConvItem::Tool {
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
                    self.timeline
                        .items
                        .push(ConvItem::Notice(format!("compacted — {summary}")));
                }
                _ => {}
            }
        }
        // Resumed content scrolls off the top; start pinned to the latest.
        self.timeline.scroll = 0;
    }

    /// Fold one engine event into the view-model. Pure over `self` — no I/O — so
    /// a canned event sequence can be asserted against the resulting state.
    pub fn apply_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::AssistantDelta { text } => {
                if self.timeline.streaming {
                    if let Some(ConvItem::Assistant(buf)) = self.timeline.items.last_mut() {
                        buf.push_str(&text);
                        return;
                    }
                }
                // The answer is starting: settle the just-streamed reasoning
                // trail to its collapsed line unless the view pins it open
                // (Design §4.4).
                if self.timeline.reasoning == ReasoningView::Collapsed {
                    if let Some(ConvItem::Reasoning { expanded, .. }) =
                        self.timeline.items.last_mut()
                    {
                        *expanded = false;
                    }
                }
                self.timeline.streaming = true;
                self.timeline.items.push(ConvItem::Assistant(text));
            }
            UiEvent::ReasoningDelta { text } => {
                // `hidden` is a view choice: skip the trail but the engine still
                // records the trace to the transcript (P-10, Design §4.4).
                if self.timeline.reasoning == ReasoningView::Hidden {
                    return;
                }
                if let Some(ConvItem::Reasoning { text: buf, .. }) = self.timeline.items.last_mut()
                {
                    buf.push_str(&text);
                } else {
                    // Stream in place while thinking; expanded until the answer
                    // begins (then settled), or always when the view pins it.
                    self.timeline.items.push(ConvItem::Reasoning {
                        text,
                        expanded: true,
                    });
                }
            }
            UiEvent::AssistantDone => self.timeline.streaming = false,
            UiEvent::TurnEnded => {
                self.anim.busy = false;
                self.timeline.streaming = false;
            }
            UiEvent::ToolStarted {
                call_id,
                tool,
                summary,
                explanation,
            } => {
                self.timeline.streaming = false;
                self.timeline.items.push(ConvItem::Tool {
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
                self.prompts.permission = Some((id, rendering));
                self.prompts.permission_scroll = 0; // start every prompt at the top
            }
            UiEvent::AskUserRequest {
                id,
                question,
                options,
            } => {
                self.timeline.streaming = false;
                self.prompts.ask = Some(AskPrompt::new(id, question, options));
            }
            UiEvent::LoopHalted { reason } => {
                self.timeline.streaming = false;
                self.prompts.loop_halt = Some(LoopHaltPrompt::new(reason));
            }
            UiEvent::CompletionGateHalted { failing, attempts } => {
                self.timeline.streaming = false;
                self.prompts.completion_gate = Some(CompletionGatePrompt::new(failing, attempts));
            }
            UiEvent::ContextUsage { pct, tokens } => {
                self.usage.context_pct = pct;
                self.usage.context_tokens = tokens;
            }
            UiEvent::CostEstimate { usd, .. } => {
                self.usage.cost_usd = usd;
                self.usage.cost_known = true;
            }
            UiEvent::SessionUsage { usage } => self.usage.tokens = usage,
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
            UiEvent::Notice { message } => self.timeline.items.push(ConvItem::Notice(message)),
            UiEvent::HarnessError { what, why, next } => {
                self.timeline
                    .items
                    .push(ConvItem::Notice(format!("error: {what} — {why}. {next}")));
            }
            UiEvent::Retrying {
                attempt,
                max_attempts,
                delay_ms,
                reason,
            } => {
                self.timeline.items.push(ConvItem::Notice(format!(
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
                self.files.last = Some(path.clone());
                let is_new = self.upsert_modified(path, adds, dels);
                // A newly-landed entry gets a brief settle highlight (Design §6.4).
                if is_new && self.anim.active {
                    self.anim.sidebar_settle = SETTLE_FRAMES;
                }
            }
            UiEvent::FileDiff { path, unified } => {
                // Show it inline when the edit executes (Design §4.2) …
                self.timeline.items.push(ConvItem::Diff {
                    unified: unified.clone(),
                });
                // … and keep the latest per file for the on-demand overlay.
                self.files.diffs.insert(path, unified);
            }
            UiEvent::CompactionStatus { message } => {
                self.timeline.items.push(ConvItem::Notice(message));
            }
            UiEvent::TaskListUpdated { items } => {
                self.tasks = items.clone();
                self.timeline.items.push(ConvItem::TaskList { items });
            }
            UiEvent::MemoryStatus { user, project } => {
                self.memory.user = user;
                self.memory.project = project;
            }
            UiEvent::CompletionStatus { checks } => {
                self.completion_status = checks;
            }
            UiEvent::SkillsAvailable { skills } => {
                self.skills = skills;
            }
            UiEvent::SubagentSpawned { id, name, .. } => {
                self.agents.push(AgentSummary {
                    id,
                    name,
                    ended: false,
                });
            }
            // Marked ended rather than removed, so the entry survives in the
            // `/agents` inspector catalog for the rest of the session (Design
            // §4.13); only the sidebar section filters it back out (§3.1).
            UiEvent::SubagentEnded { id, .. } => {
                if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
                    agent.ended = true;
                }
            }
            UiEvent::AgentActivity { id, name, text } => {
                self.apply_agent_activity(&id, &name, text);
            }
            // A staged attachment (FR-10, Design §4.14): mirror the engine's
            // own staging list so the compose area can confirm what's
            // pending; the chip itself only lands in the timeline once the
            // message is actually sent (`send_prompt`).
            UiEvent::ImageAttached {
                name,
                width,
                height,
                format_label,
                ..
            } => {
                self.pending_attachments.push(PendingAttachment {
                    name: name.clone(),
                    width,
                    height,
                    format_label: format_label.clone(),
                });
                self.timeline.items.push(ConvItem::Notice(format!(
                    "attached {name} · {width}×{height} · {format_label}"
                )));
            }
            UiEvent::AttachFailed { path, reason } => {
                self.timeline
                    .items
                    .push(ConvItem::Notice(format!("attach failed: {path}: {reason}")));
            }
            // `init`/`clean` voice: exactly what was written and where, plus
            // the one calm, non-blocking disclosure line (FR-12, Design §8.11).
            UiEvent::SessionExported { path } => {
                self.timeline
                    .items
                    .push(ConvItem::Notice(format!("exported to {path}")));
                self.timeline.items.push(ConvItem::Notice(
                    crate::strings::export::SENSITIVE_CONTENT_NOTE.to_string(),
                ));
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
        self.timeline
            .items
            .iter_mut()
            .rev()
            .find(|item| matches!(item, ConvItem::Tool { call_id: c, .. } if c == call_id))
    }

    /// Insert or update a modified-file entry; returns `true` if it was new.
    fn upsert_modified(&mut self, path: String, adds: u32, dels: u32) -> bool {
        if let Some(existing) = self.files.modified.iter_mut().find(|f| f.path == path) {
            existing.adds = adds;
            existing.dels = dels;
            false
        } else {
            self.files.modified.push(ModifiedFile { path, adds, dels });
            true
        }
    }
}
