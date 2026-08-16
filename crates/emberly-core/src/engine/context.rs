//! Context economy: the compaction cycle (FR-4), the conversation
//! window, and the token accounting that decides when to compact (FR-3).
//!
//! Part of the `Engine` inherent impl; the engine is the sole owner of the
//! state these methods touch.

use super::*;

impl Engine {
    /// Queue a compaction for the next clean boundary (Tech Spec §7). Manual
    /// outranks Auto — a user `/compact` is never downgraded to `auto` (FR-4).
    pub(super) fn request_compaction(&mut self, trigger: CompactTrigger) {
        if matches!(self.context.pending, Some(CompactTrigger::Manual)) {
            return;
        }
        self.context.pending = Some(trigger);
    }

    /// Compaction at a clean boundary (Tech Spec §7). Replaces the middle
    /// of the conversation — everything after the pinned original task and
    /// before the last `context.keep_recent_turns` turns — with a model-written summary,
    /// keeping the session usable when context grows. The pinned content
    /// (system prompt, original task) is never compacted; the JSONL log is
    /// untouched (the compaction is recorded as one event, replayed on resume).
    /// `trigger` records whether the user or the FR-4 threshold initiated it.
    pub(super) async fn compact(&mut self, trigger: CompactTrigger) {
        // Pinned = the original task (the system prompt lives outside the
        // conversation). Keep the tail verbatim; summarize the middle. The cut
        // point is snapped to a turn boundary via `group_turn_starts` — the
        // same primitive `windowed_messages` uses — so a turn with several
        // tool calls is never split between the summary and the kept tail
        // (a `Role::Tool` message left without its preceding `Role::Assistant`
        // tool_calls message is an invalid request to every provider).
        let pinned = usize::from(!self.history.messages.is_empty());
        let len = self.history.messages.len();
        let from = pinned;
        let turns = group_turn_starts(&self.history.messages[from..]);
        let keep = self.context.config.keep_recent_turns.min(turns.len());
        let elided = turns.len().saturating_sub(keep);
        let to = if elided == 0 {
            from
        } else if elided < turns.len() {
            from + turns[elided]
        } else {
            len
        };
        if to <= from {
            self.emit(UiEvent::CompactionStatus {
                message: "nothing to compact yet".into(),
            })
            .await;
            return;
        }

        self.emit(UiEvent::CompactionStatus {
            message: "compacting the conversation…".into(),
        })
        .await;

        let summary = match self.summarize(&self.history.messages[from..to]).await {
            Ok(text) if !text.is_empty() => text,
            // Failure fallback (Tech Spec §7): drop the middle behind a
            // placeholder with a visible warning — a full context never yields a
            // stuck session. Recorded as a compaction so resume stays consistent.
            other => {
                let why = match other {
                    Ok(_) => "the summary came back empty".to_string(),
                    Err(error) => error.to_string(),
                };
                self.emit(UiEvent::CompactionStatus {
                    message: format!("summarization failed ({why}); truncated older context"),
                })
                .await;
                format!("[older context was truncated — summarization failed: {why}]")
            }
        };

        // Rebuild: [original task][summary as user message][recent verbatim].
        // Turn map is rebuilt in parallel: pinned turns keep their numbers, the
        // summary gets the next monotonic number, and the tail retains its
        // original numbers (FR-3: compaction retires numbers, never shifts them).
        let mut rebuilt = Vec::with_capacity(2 + keep);
        let mut rebuilt_turns = Vec::with_capacity(2 + keep);
        rebuilt.extend(self.history.messages[..from].iter().cloned());
        rebuilt_turns.extend(self.history.turn_map[..from].iter().copied());
        let summary_turn = self.history.next_turn;
        rebuilt.push(Message::user_text(summary.clone()));
        rebuilt_turns.push(summary_turn);
        self.history.next_turn += 1;
        rebuilt.extend(self.history.messages[to..].iter().cloned());
        rebuilt_turns.extend(self.history.turn_map[to..].iter().copied());
        self.history.messages = rebuilt;
        self.history.turn_map = rebuilt_turns;
        self.context.compacted = true;

        self.write_transcript(TranscriptEvent::Compaction {
            summary,
            replaced_from: u32::try_from(from).unwrap_or(u32::MAX),
            replaced_to: u32::try_from(to).unwrap_or(u32::MAX),
            trigger,
        });
        let turns_compacted = elided;
        self.emit(UiEvent::CompactionStatus {
            message: match trigger {
                CompactTrigger::Manual => {
                    format!("Compacted {turns_compacted} turns into a summary.")
                }
                CompactTrigger::Auto => format!(
                    "Context was near full — compacted {turns_compacted} turns to keep going."
                ),
            },
        })
        .await;
        self.emit_context_usage().await;
    }

    /// Summarize a slice of the conversation with the current provider using the
    /// purpose-built prompt (Tech Spec §7). Drains the stream collecting text;
    /// tool calls are not offered.
    async fn summarize(&self, messages: &[Message]) -> Result<String, ProviderError> {
        let prompt = self
            .context
            .summary_prompt
            .as_deref()
            .unwrap_or_else(|| crate::prompts::compact());
        let request = CompletionRequest {
            model: self.provider.model.clone(),
            system: Some(prompt.to_string()),
            messages: vec![Message::user_text(render_for_summary(messages))],
            tools: Vec::new(),
            max_output_tokens: Some(self.provider.client.model_info().max_output_tokens),
            temperature: None,
            // Summarization is a fixed internal task; it does not carry the
            // session's reasoning effort.
            effort: None,
        };
        let mut stream = self.provider.client.stream_completion(request).await?;
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            if let StreamEvent::TextDelta { text: delta } = item? {
                text.push_str(&delta);
            }
        }
        Ok(text.trim().to_string())
    }

    /// Validate and stage an image attachment for the prompt currently being
    /// composed (FR-10, Design §4.14), issued at idle before the message is
    /// sent. `path` is whatever the user named directly via `/attach` — a
    /// **user**-initiated read, not an agent-initiated one, so no project-root
    /// confinement or permission prompt applies (mirrors `emberly export`'s
    /// output path, FR-12). The cap (`image.max_attachments`) is enforced
    /// against the engine's own staging list before reading the file, so a
    /// call over the cap costs no I/O.
    pub(super) async fn attach_image(&mut self, path: String) {
        if self.pending_attachments.len() >= self.image_max_attachments {
            self.emit(UiEvent::AttachFailed {
                path,
                reason: format!(
                    "too many attachments (max {}) — send or clear pending attachments first",
                    self.image_max_attachments
                ),
            })
            .await;
            return;
        }

        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => {
                self.emit(UiEvent::AttachFailed {
                    path,
                    reason: format!("cannot read: {e}"),
                })
                .await;
                return;
            }
        };

        let encoded = match encode_image_bytes(&bytes, self.image_max_bytes) {
            Ok(e) => e,
            Err(reason) => {
                self.emit(UiEvent::AttachFailed { path, reason }).await;
                return;
            }
        };

        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());

        self.pending_attachments.push(AttachedImage {
            name: name.clone(),
            media_type: encoded.content.media_type.clone(),
            data: encoded.content.data,
            width: encoded.width,
            height: encoded.height,
            format_label: encoded.format_label.to_string(),
        });

        self.emit(UiEvent::ImageAttached {
            path,
            name,
            media_type: encoded.content.media_type,
            width: encoded.width,
            height: encoded.height,
            format_label: encoded.format_label.to_string(),
            pending_count: self.pending_attachments.len(),
        })
        .await;
    }

    /// Build the outgoing user message, folding in any staged attachments
    /// (FR-10, Design §4.14). A user-attached image is the identical
    /// `ContentBlock::Image` a model-read image already produces (T-12) —
    /// told apart structurally by which message carries it, never by a flag
    /// (Tech Spec §4.1). On a model with no vision, the image is not sent;
    /// instead a plain text note says so, per the same honesty clause P-11
    /// already states for `read_image` (HC-6 — never a silent drop).
    pub(super) fn build_user_message(
        &self,
        text: String,
        attachments: Vec<AttachedImage>,
    ) -> Message {
        if attachments.is_empty() {
            return Message::user_text(text);
        }
        let mut content = vec![ContentBlock::Text { text }];
        if self.provider.client.model_info().vision {
            for image in attachments {
                content.push(ContentBlock::Image {
                    media_type: image.media_type,
                    data: image.data,
                });
            }
        } else {
            for image in attachments {
                content.push(ContentBlock::Text {
                    text: format!(
                        "[Image '{}' was not sent — the active model has no vision support. \
                         Switch models (/model) or describe the image.]",
                        image.name
                    ),
                });
            }
        }
        Message {
            role: Role::User,
            content,
        }
    }

    /// Push a message to the conversation and update the turn map (FR-3 turn
    /// numbering). `Role::User` messages start a new turn; assistant/tool
    /// messages inherit the current turn number.
    pub(super) fn push_conversation_message(&mut self, msg: Message) {
        let turn = if self.history.messages.is_empty() || msg.role == Role::User {
            let t = self.history.next_turn;
            self.history.next_turn += 1;
            t
        } else {
            *self.history.turn_map.last().unwrap_or(&0)
        };
        self.history.messages.push(msg);
        self.history.turn_map.push(turn);
    }

    /// The number of messages at the front of the conversation that are always
    /// sent and never windowed away (FR-3, Tech Spec §7): the original task
    /// (position 0) plus the compaction summary (position 1) when one is active.
    fn pinned_count(&self) -> usize {
        if self.history.messages.is_empty() {
            return 0;
        }
        // The original task is always pinned at position 0.
        let mut n = 1;
        // After compaction (live or resumed), the summary at position 1 is
        // also pinned (Tech Spec §7: windowing never drops the summary).
        if self.context.compacted {
            n += 1;
        }
        n.min(self.history.messages.len())
    }

    /// The windowed view of the conversation for sending to the provider
    /// (FR-3, Tech Spec §7). A **pure view** — never mutates
    /// `self.history.messages` (HC-7). Keeps the pinned prefix and the last
    /// `context.window_turns` turns; replaces older turns with one synthetic
    /// elision marker. Turns are grouped at clean boundaries: every
    /// `ContentBlock::ToolUse` keeps its matching `Message::tool_result`, so
    /// the sent list stays provider-valid.
    pub(super) fn windowed_messages(&self) -> Vec<Message> {
        let pinned = self.pinned_count();
        let total = self.history.messages.len();
        if total <= pinned {
            return self.history.messages.clone();
        }

        let turns = group_turn_starts(&self.history.messages[pinned..]);
        let elided = turns.len().saturating_sub(self.context.config.window_turns);
        if elided == 0 {
            return self.history.messages.clone();
        }

        // Index into self.history.messages where the first kept turn begins.
        let keep_from = if elided < turns.len() {
            pinned + turns[elided]
        } else {
            // window_turns = 0: elide every non-pinned turn.
            total
        };

        // The turn range being elided, using stable monotonic turn numbers
        // (FR-3, Tech Spec §7/§16) so the marker names a range `recall` can
        // take.
        let first_turn_msg = pinned + turns[0];
        let last_turn_msg = if elided < turns.len() {
            pinned + turns[elided] - 1
        } else {
            total - 1
        };
        let first_turn = self.history.turn_map[first_turn_msg];
        let last_turn = self.history.turn_map[last_turn_msg];

        let mut result = Vec::with_capacity(pinned + 1 + total.saturating_sub(keep_from));
        result.extend(self.history.messages[..pinned].iter().cloned());
        result.push(Message::user_text(format!(
            "[turns {first_turn}–{last_turn} elided from context \
             — still in the session transcript; use recall to retrieve them]"
        )));
        result.extend(self.history.messages[keep_from..].iter().cloned());
        result
    }

    /// Emit context usage and, when pricing is configured, the running cost
    /// estimate (Requirements §8.4, P-6; Design §3.1). Uses the provider's
    /// authoritative prompt-token count once available, falling back to the
    /// provider's own `count_tokens` heuristic before the first `Usage` (issue
    /// #12 made that script-weighted, not chars/4). Also checks the FR-4
    /// automatic-compaction threshold and queues a compaction when crossed.
    pub(super) async fn emit_context_usage(&mut self) {
        let info = self.provider.client.model_info();
        let reserve = OUTPUT_RESERVE.min(u64::from(info.max_output_tokens));
        let budget = u64::from(info.context_window)
            .saturating_sub(reserve)
            .max(1);
        let tokens = self
            .context
            .tokens_authoritative
            .unwrap_or_else(|| self.context_tokens());
        let pct = ((tokens.saturating_mul(100)) / budget).min(100);
        self.emit(UiEvent::ContextUsage {
            pct: u8::try_from(pct).unwrap_or(100),
            tokens,
        })
        .await;

        // Automatic compaction threshold check (FR-4, Tech Spec §7). The latch
        // prevents thrash: after an auto-compaction fires the latch disarms and
        // stays disarmed until usage drops below the threshold, then re-crosses
        // it. The compaction itself drops usage well below the line, so
        // re-arming is natural.
        if self.context.config.auto_compact {
            let pct_f = f64::from(u8::try_from(pct).unwrap_or(100));
            if pct_f >= self.context.config.auto_compact_threshold * 100.0 {
                if self.context.auto_armed {
                    self.request_compaction(CompactTrigger::Auto);
                    self.context.auto_armed = false;
                }
            } else {
                // Usage below threshold: re-arm the latch.
                self.context.auto_armed = true;
            }
        }

        // Cumulative session tokens — always available (independent of pricing).
        self.emit(UiEvent::SessionUsage {
            usage: self.session.usage,
        })
        .await;

        // Cost is only knowable with a pricing table (always labeled "est.").
        if info.pricing.is_some() {
            self.emit(UiEvent::CostEstimate {
                usage: self.session.usage,
                usd: self.session.cost_usd,
            })
            .await;
        }
    }

    fn context_tokens(&self) -> u64 {
        let count = |s: &str| self.provider.client.count_tokens(s).tokens;
        let mut total = self.provider.system.as_deref().map(count).unwrap_or(0); // Count the windowed sent view (FR-3, Design §8.6), not the full
                                                                                 // in-memory conversation — so usage reflects what the provider
                                                                                 // actually receives. The elision marker is included because it rides
                                                                                 // in the sent messages.
        let messages = self.windowed_messages();
        for message in &messages {
            for block in &message.content {
                total = total.saturating_add(match block {
                    ContentBlock::Text { text } => count(text),
                    ContentBlock::ToolUse { name, input, .. } => {
                        count(name).saturating_add(count(&input.to_string()))
                    }
                    ContentBlock::ToolResult { content, .. } => count(content),
                    // Replayed reasoning is sent back on the wire, so it counts
                    // toward the context budget (P-10).
                    ContentBlock::Reasoning { text, .. } => count(text),
                    // An image's token cost is not chars/4 of its base64. Until
                    // an authoritative provider-reported usage arrives (P-6),
                    // estimate a fixed per-image cost rather than inflating the
                    // budget with raw base64 length (initial; tune with use,
                    // Requirements §13).
                    ContentBlock::Image { .. } => IMAGE_TOKEN_ESTIMATE,
                    // A document's token cost is not chars/4 of its base64
                    // either — same reasoning as the image estimate above,
                    // refined by provider-reported usage where available
                    // (P-12).
                    ContentBlock::Document { .. } => DOCUMENT_TOKEN_ESTIMATE,
                });
            }
        }
        total
    }
}
