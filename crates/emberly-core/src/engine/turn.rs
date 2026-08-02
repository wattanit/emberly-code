//! One assistant turn: opening the completion stream, consuming it,
//! and running whatever tool calls it produced.
//!
//! Part of the `Engine` inherent impl, split out of one 2,700-line
//! block; the engine is still the single owner of this state.

use super::*;

impl Engine {
    /// Drive completions until the model stops without requesting tools, an
    /// error/drop occurs, or the user cancels.
    pub(super) async fn run_turn(&mut self, chans: &mut TurnChannels<'_>) {
        let mut drop_attempts = 0u32;
        loop {
            let stream = match self.open_stream_with_retry().await {
                Ok(stream) => stream,
                Err(error) => {
                    self.emit_provider_error(&error).await;
                    return;
                }
            };

            let (end, out) = self.consume_stream(stream, chans.commands).await;
            match end {
                StreamEnd::Done { tool_calls } => {
                    self.push_assistant_message(&out, &tool_calls);
                    self.emit_context_usage().await;
                    self.emit(UiEvent::AssistantDone).await;
                    if tool_calls.is_empty() {
                        // S-6: the model claims done. With no checks
                        // registered the gate is inert (zero behavior
                        // change); with checks, a failure re-opens the loop
                        // instead of letting the turn end here.
                        match self.evaluate_completion_gate(chans.commands).await {
                            CompletionGateOutcome::Terminate => return,
                            CompletionGateOutcome::ReOpen => continue,
                        }
                    }
                    if self.run_tool_calls(tool_calls, chans).await.is_canceled() {
                        return;
                    }
                    // S-5: before issuing the next provider call, check whether
                    // the loop is re-treading without progress. On a trip, hand
                    // control to the user (resume / stop / steer) — never spin on.
                    if let Some(reason) = self.evaluate_loop() {
                        match self.await_loop_resolution(chans.commands, reason).await {
                            LoopResolution::Resume => self.reset_loop_window(),
                            LoopResolution::Stop => return,
                            LoopResolution::Steer(text) => {
                                self.reset_loop_window();
                                self.record_user_message(&text);
                                self.push_conversation_message(Message::user_text(text));
                                self.emit_context_usage().await;
                            }
                        }
                    }
                    // Loop: send the tool results back for another completion.
                }
                StreamEnd::Interrupted => {
                    // Keep the partial text visible in the conversation.
                    self.push_assistant_message(&out, &[]);
                    self.emit_context_usage().await;
                    self.emit(UiEvent::AssistantDone).await;
                    return;
                }
                StreamEnd::Errored(error) => {
                    self.emit(UiEvent::AssistantDone).await;
                    self.emit_provider_error(&error).await;
                    return;
                }
                StreamEnd::Dropped => {
                    // The partial text was shown live; it is NOT stitched into
                    // the retry (partial turns are not stitched — Tech Spec
                    // §4.3). Retry the whole turn from the unchanged history.
                    drop_attempts += 1;
                    if self.retry.may_retry(drop_attempts) {
                        let delay = self.retry.delay_for(drop_attempts);
                        self.emit_retrying(
                            drop_attempts,
                            delay,
                            "the model response ended unexpectedly",
                        )
                        .await;
                        self.emit(UiEvent::AssistantDone).await;
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    self.emit(UiEvent::AssistantDone).await;
                    self.emit(UiEvent::HarnessError {
                        what: "the model response kept ending unexpectedly".into(),
                        why: "the provider stream closed before completing, repeatedly".into(),
                        next: "send your message again to retry".into(),
                    })
                    .await;
                    return;
                }
            }
        }
    }

    /// Open a completion stream, retrying retryable pre-stream failures with
    /// backoff (Tech Spec §4.3). Every retry is surfaced (Design §6.1).
    async fn open_stream_with_retry(&mut self) -> Result<CompletionStream, ProviderError> {
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let request = self.build_request();
            match self.provider.client.stream_completion(request).await {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    if !error.is_retryable() || !self.retry.may_retry(attempts) {
                        return Err(error);
                    }
                    let delay = error
                        .retry_after()
                        .unwrap_or_else(|| self.retry.delay_for(attempts));
                    self.emit_retrying(attempts, delay, &error.to_string())
                        .await;
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn emit_retrying(&self, attempt: u32, delay: std::time::Duration, reason: &str) {
        self.emit(UiEvent::Retrying {
            attempt,
            max_attempts: self.retry.max_attempts,
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            reason: reason.to_string(),
        })
        .await;
    }

    /// Read one completion stream to its end, emitting text deltas and
    /// accumulating tool calls. Cancellable at each await point.
    async fn consume_stream(
        &mut self,
        mut stream: CompletionStream,
        commands_rx: &mut mpsc::Receiver<Command>,
    ) -> (StreamEnd, TurnOutput) {
        let mut out = TurnOutput::default();
        let mut tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut saw_done = false;

        let end = loop {
            tokio::select! {
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        self.handle_stream_event(event, &mut out, &mut tool_calls, &mut saw_done)
                            .await;
                    }
                    Some(Err(error)) => break StreamEnd::Errored(error),
                    None => break if saw_done { StreamEnd::Done { tool_calls: std::mem::take(&mut tool_calls) } } else { StreamEnd::Dropped },
                },
                Some(command) = commands_rx.recv() => {
                    match command {
                        Command::Cancel => break StreamEnd::Interrupted,
                        // Queue a compaction for the clean boundary (Tech Spec §7).
                        Command::Compact => self.request_compaction(CompactTrigger::Manual),
                        // A mode toggle applies immediately, even mid-stream.
                        Command::SetMode { mode } => self.set_mode(mode).await,
                        // Ignore permission answers / other commands mid-stream.
                        _ => {}
                    }
                }
            }
        };

        // The caller commits the assistant message: on Done/Interrupted it is
        // kept; on a retryable Dropped it is discarded (not stitched, §4.3).
        (end, out)
    }

    /// Apply one stream event. Terminal events (`Done`) only set `saw_done`;
    /// the stream loop keeps draining until EOF so trailing chunks — notably
    /// the OpenAI-compatible `usage` chunk, which arrives after finish_reason —
    /// are still processed. The loop finalizes via its `None` (EOF) arm.
    async fn handle_stream_event(
        &mut self,
        event: StreamEvent,
        out: &mut TurnOutput,
        tool_calls: &mut Vec<PendingToolCall>,
        saw_done: &mut bool,
    ) {
        match event {
            StreamEvent::TextDelta { text: delta } => {
                out.text.push_str(&delta);
                self.emit(UiEvent::AssistantDelta { text: delta }).await;
            }
            StreamEvent::ReasoningDelta { text: delta } => {
                out.reasoning.push_str(&delta);
                self.emit(UiEvent::ReasoningDelta { text: delta }).await;
            }
            StreamEvent::ReasoningSignature {
                signature,
                redacted,
            } => {
                // Opaque replay token for the reasoning block (P-1); kept to
                // echo back on later tool-use turns.
                out.reasoning_signature = Some((signature, redacted));
            }
            StreamEvent::ToolCallStart { id, name } => {
                tool_calls.push(PendingToolCall {
                    id,
                    name,
                    args: String::new(),
                });
            }
            StreamEvent::ToolCallDelta { id, args_delta } => {
                if let Some(call) = tool_calls.iter_mut().find(|c| c.id == id) {
                    call.args.push_str(&args_delta);
                }
            }
            StreamEvent::ToolCallEnd { .. } => {}
            StreamEvent::Usage { usage } => {
                // Accumulate billed tokens and cost (each request's input is
                // billed), and record the prompt size as the current context.
                self.session.usage.input = self.session.usage.input.saturating_add(usage.input);
                self.session.usage.output = self.session.usage.output.saturating_add(usage.output);
                self.context.tokens_authoritative = Some(usage.input);
                if let Some(pricing) = self.provider.client.model_info().pricing {
                    self.session.cost_usd += pricing.estimate_usd(usage);
                }
            }
            StreamEvent::Done { stop_reason: _ } => {
                // Mark done but keep draining the stream: OpenAI-compatible
                // servers send the `usage` chunk *after* the finish_reason
                // chunk (real OpenAI does too). Breaking here would drop it and
                // leave session token counts at 0. The stream's `None` arm
                // finalizes with `StreamEnd::Done` once `saw_done` is set.
                *saw_done = true;
            }
            // `StreamEvent` is non-exhaustive; ignore variants added later.
            _ => {}
        }
    }

    /// Execute tool calls sequentially, appending each result to the
    /// conversation. Stops early on cancellation, backfilling canceled results
    /// so the conversation stays well-formed (every tool_use has a result).
    async fn run_tool_calls(
        &mut self,
        tool_calls: Vec<PendingToolCall>,
        chans: &mut TurnChannels<'_>,
    ) -> ToolCallResult {
        // Start a fresh loop-signature observation for this turn (S-5).
        self.guardrail.turn_obs = TurnObservation::default();
        let mut iter = tool_calls.into_iter();
        while let Some(call) = iter.next() {
            match self.run_one_tool_call(&call, chans).await {
                ToolCallResult::Completed(outcome) => {
                    self.ingest_tool_result(&call, *outcome).await
                }
                ToolCallResult::Canceled => {
                    self.push_canceled_result(&call).await;
                    for remaining in iter {
                        self.push_canceled_result(&remaining).await;
                    }
                    return ToolCallResult::Canceled;
                }
            }
        }
        ToolCallResult::Completed(Box::new(emberly_tools::ToolOutcome::success("", "")))
    }

    /// Run one tool call, driving its execution concurrently with permission
    /// asks and cancellation.
    async fn run_one_tool_call(
        &mut self,
        call: &PendingToolCall,
        chans: &mut TurnChannels<'_>,
    ) -> ToolCallResult {
        let args = serde_json::from_str(&call.args).unwrap_or(serde_json::Value::Null);

        // Record the (tool, normalized-args) for the loop signature (S-5),
        // including unknown-tool attempts (a loop can re-tread those too).
        if self.guardrail.config.enabled {
            self.guardrail
                .turn_obs
                .calls
                .push((call.name.clone(), normalize_args(&args)));
        }

        let Some(tool) = self.tools.get(&call.name) else {
            return ToolCallResult::Completed(Box::new(emberly_tools::ToolOutcome::failure(
                format!("unknown tool: {}", call.name),
                "unknown tool",
            )));
        };

        // Describe the invocation from its arguments (e.g. `run: cargo test`,
        // `read src/main.rs`) so the activity line says what is happening, not
        // just the tool name (Design §6.3).
        let summary = tool.describe(&args).unwrap_or_else(|| call.name.clone());
        // T-9: the model's caption for a non-obvious call rides in `args`
        // (§5.4); surface it to the frontend. Absent/empty → None (no caption).
        let explanation = explanation_from_args(&args);
        self.emit(UiEvent::ToolStarted {
            call_id: call.id.clone(),
            tool: call.name.clone(),
            summary,
            explanation,
        })
        .await;

        let ctx = self.make_ctx();
        let mut exec = Box::pin(tool.execute(args, &ctx));
        let mut pending: Vec<PendingAsk> = Vec::new();
        let mut pending_user: Vec<PendingUserAsk> = Vec::new();
        let mut commands_open = true;

        loop {
            tokio::select! {
                outcome = &mut exec => return ToolCallResult::Completed(Box::new(outcome)),
                Some(ask) = chans.asks.recv() => self.on_permission_ask(ask, &mut pending).await,
                Some(ask) = chans.user_asks.recv() => self.on_user_ask(ask, &mut pending_user).await,
                Some(recall) = chans.recall.recv() => self.on_recall(recall).await,
                Some(task) = chans.task.recv() => self.on_task_list_set(task).await,
                Some(mem) = chans.memory.recv() => self.on_memory_op(mem).await,
                Some(skill) = chans.skill.recv() => self.on_skill_invoke(skill).await,
                command = chans.commands.recv(), if commands_open => match command {
                    Some(Command::PermissionAnswer { id, decision }) => {
                        self.answer_permission(id, decision, &mut pending).await;
                    }
                    Some(Command::AskUserAnswer { id, answer }) => {
                        self.answer_user_ask(id, answer, &mut pending_user).await;
                    }
                    Some(Command::Cancel) => return ToolCallResult::Canceled,
                    // Queue a compaction for the clean boundary (Tech Spec §7).
                    Some(Command::Compact) => self.request_compaction(CompactTrigger::Manual),
                    // A mode toggle applies immediately to later asks this turn.
                    Some(Command::SetMode { mode }) => self.set_mode(mode).await,
                    Some(_) => {}
                    None => {
                        // No more input (frontend gone): resolve anything pending
                        // as the safe default (deny / decline) and stop watching
                        // commands, so we never hang on an answer that cannot
                        // arrive.
                        commands_open = false;
                        for p in pending.drain(..) {
                            let _ = p.reply.send(PermissionOutcome::Deny);
                        }
                        for p in pending_user.drain(..) {
                            self.record_ask(&p.question, &p.options, None);
                            let _ = p.reply.send(AskUserOutcome::Declined);
                        }
                    }
                },
            }
        }
    }

    /// Truncate a tool result at ingestion (§8.1), append it to the
    /// conversation, and emit UI events (finish, file change, usage).
    async fn ingest_tool_result(
        &mut self,
        call: &PendingToolCall,
        outcome: emberly_tools::ToolOutcome,
    ) {
        // Accumulate this result into the turn's loop signature (S-5): identical
        // repeated results are a no-progress signal.
        if self.guardrail.config.enabled {
            self.guardrail
                .turn_obs
                .result_content
                .push_str(&outcome.content);
        }
        // Salient reduction (FR-2) runs before the size backstop (§5.3), gated
        // on `truncate.reduce`. Both are deterministic, no-model-call transforms.
        let reduced = if self.truncate.reduce {
            reduce_output(&call.name, &outcome.content)
        } else {
            Reduction {
                content: outcome.content.clone(),
                reduced: false,
                withheld: String::new(),
            }
        };
        let truncation = truncate_output(&reduced.content, &self.truncate);

        // Durable record: the model-visible (possibly reduced + truncated)
        // output, plus a sidecar holding the full output when either layer
        // withheld content (Requirements §8.1/§8.5, HC-7).
        let withheld = reduced.reduced || truncation.truncated;
        let full_output_ref = if withheld {
            self.session.transcript.sidecar(&call.id, &outcome.content)
        } else {
            None
        };
        self.write_transcript(TranscriptEvent::ToolResult {
            call_id: call.id.clone(),
            ok: outcome.ok,
            output: truncation.content.clone(),
            truncated: withheld,
            full_output_ref,
        });

        self.push_conversation_message(Message::tool_result(
            call.id.clone(),
            truncation.content,
            !outcome.ok,
        ));

        // An image from `read_image` (P-11): append a `ContentBlock::Image` as
        // a synthetic user message so both adapters can carry it — Anthropic in
        // a user-role image block, OpenAI as an `image_url` part (whose tool
        // role cannot hold images, Tech Spec §4.2). The bytes are NOT in the
        // transcript (HC-7); the `tool_call` recorded the path, and the block
        // lives only in the live conversation (re-derived from disk on resume —
        // honestly absent if the file is gone).
        if let Some(image) = outcome.image {
            self.push_conversation_message(Message {
                role: Role::User,
                content: vec![ContentBlock::Image {
                    media_type: image.media_type,
                    data: image.data,
                }],
            });
        }

        // A document from `read_document` (P-12): append a
        // `ContentBlock::Document` the same way an image is appended — a
        // synthetic user message so both adapters can carry it (Tech Spec
        // §4.2). The bytes are NOT in the transcript (HC-7); the `tool_call`
        // recorded the path, and the block lives only in the live
        // conversation (re-derived from disk on resume).
        if let Some(document) = outcome.document {
            self.push_conversation_message(Message {
                role: Role::User,
                content: vec![ContentBlock::Document {
                    media_type: document.media_type,
                    data: document.data,
                }],
            });
        }

        self.emit(UiEvent::ToolFinished {
            call_id: call.id.clone(),
            ok: outcome.ok,
            summary: outcome.summary,
            preview: result_preview(&outcome.content),
            untrusted: outcome.untrusted,
        })
        .await;

        if let Some(change) = outcome.file_change {
            // A newly-modified file is the strongest progress signal (S-5).
            if self.guardrail.config.enabled {
                self.guardrail.turn_obs.files.push(change.path.clone());
            }
            self.emit(UiEvent::FileModified {
                path: change.path.clone(),
                adds: change.adds,
                dels: change.dels,
            })
            .await;
            if let Some(unified) = change.diff {
                self.emit(UiEvent::FileDiff {
                    path: change.path,
                    unified,
                })
                .await;
            }
        }
        self.emit_context_usage().await;
    }

    async fn push_canceled_result(&mut self, call: &PendingToolCall) {
        let canceled = "The user canceled before this tool ran.".to_string();
        // Keep the transcript well-formed: every tool_call has a tool_result.
        self.write_transcript(TranscriptEvent::ToolResult {
            call_id: call.id.clone(),
            ok: false,
            output: canceled.clone(),
            truncated: false,
            full_output_ref: None,
        });
        self.push_conversation_message(Message::tool_result(call.id.clone(), canceled, true));
        self.emit(UiEvent::ToolFinished {
            call_id: call.id.clone(),
            ok: false,
            summary: "canceled".into(),
            preview: String::new(),
            untrusted: false,
        })
        .await;
    }

    /// Build the assistant message for the turn: reasoning (P-10), then text,
    /// then any tool-use blocks. Records the complete assistant message — with
    /// reasoning as a distinct field, never merged into the answer — and each
    /// requested tool call to the transcript (Tech Spec §3.2, §4.7).
    fn push_assistant_message(&mut self, out: &TurnOutput, tool_calls: &[PendingToolCall]) {
        let text = out.text.as_str();
        let has_reasoning = !out.reasoning.is_empty() || out.reasoning_signature.is_some();
        if text.is_empty() && tool_calls.is_empty() && !has_reasoning {
            return;
        }
        if !text.is_empty() || has_reasoning {
            self.write_transcript(TranscriptEvent::AssistantMessage {
                text: text.to_string(),
                // Recorded whatever the view key: `hidden` is a view choice, not
                // a discard (P-10, Design §4.4).
                reasoning: (!out.reasoning.is_empty()).then(|| out.reasoning.clone()),
            });
        }
        // A reasoning-only turn (e.g. canceled before any text or tool call) has
        // nothing a provider's wire format can carry — every adapter maps
        // `ContentBlock::Reasoning` alone to a contentless assistant message,
        // which providers reject, and once committed that rejection repeats on
        // every retry (issue #13). The reasoning is still visible above, via the
        // transcript; it is just never replayed to the provider.
        if text.is_empty() && tool_calls.is_empty() {
            return;
        }
        let mut content = Vec::new();
        // Reasoning must precede text/tool_use so a provider that requires the
        // thinking block echoed back accepts the turn (Anthropic ordering).
        if has_reasoning {
            let (signature, redacted) = match &out.reasoning_signature {
                Some((sig, red)) => (Some(sig.clone()), *red),
                None => (None, false),
            };
            content.push(ContentBlock::Reasoning {
                // A redacted block has no replayable text; a normal one replays
                // the exact reasoning it streamed.
                text: if redacted {
                    String::new()
                } else {
                    out.reasoning.clone()
                },
                signature,
                redacted,
            });
        }
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
        for call in tool_calls {
            let input = serde_json::from_str(&call.args).unwrap_or(serde_json::Value::Null);
            self.write_transcript(TranscriptEvent::ToolCall {
                call_id: call.id.clone(),
                tool: call.name.clone(),
                args: input.clone(),
            });
            content.push(ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input,
            });
        }
        self.push_conversation_message(Message {
            role: Role::Assistant,
            content,
        });
    }

    fn build_request(&self) -> CompletionRequest {
        let tools = self
            .tools
            .specs()
            .into_iter()
            .map(|spec| {
                let mut input_schema = spec.input_schema;
                // T-9: inject the optional `explanation` property at the single
                // ToolSpec→provider point, so both wire adapters get it without
                // any per-adapter code (Tech Spec §5.4). Off → nothing added.
                if self.tool_explanations {
                    inject_explanation_property(&mut input_schema);
                }
                ToolSchema {
                    name: spec.name,
                    description: spec.description,
                    input_schema,
                }
            })
            .collect();
        CompletionRequest {
            model: self.provider.model.clone(),
            // T-9: append the explanation instruction only when the feature is
            // on, so with it off the model is never asked and no tokens are
            // spent. Kept out of the stored `self.provider.system` so a config reload
            // (which replaces it) stays orthogonal to this toggle.
            system: self.effective_system(),
            messages: self.windowed_messages(),
            tools,
            max_output_tokens: Some(self.provider.client.model_info().max_output_tokens),
            temperature: None,
            // The session's active reasoning effort (P-9). The adapter maps it
            // to the provider's control or drops it when unsupported.
            effort: self.provider.effort,
        }
    }

    /// The outgoing system prompt: the stored base (base prompt + project
    /// instructions) with the tool-call explanation instruction appended when
    /// enabled (T-9), and the task-list block appended when the list is non-empty
    /// and pinning is on (T-11, Tech Spec §7).
    fn effective_system(&self) -> Option<String> {
        let base = if self.tool_explanations {
            let instruction = crate::prompts::tool_explanation();
            match &self.provider.system {
                Some(b) => Some(format!("{b}\n\n{instruction}")),
                None => Some(instruction.to_string()),
            }
        } else {
            self.provider.system.clone()
        };

        let base = if self.context.config.pin_task_list && !self.task_list.is_empty() {
            let block = render_task_list_block(&self.task_list);
            match &base {
                Some(b) => Some(format!("{b}\n\n{block}")),
                None => Some(block),
            }
        } else {
            base
        };

        // Pin the memory index (FR-6, Tech Spec §7/§8.1). Only the one-line
        // index is standing context; entry bodies load via the `recall` op
        // (progressive disclosure). Project memory is absent on an untrusted
        // root (Design §4.9).
        let base = if self.memory.config.enabled {
            let block = render_memory_block(&self.memory.user_index, &self.memory.project_index);
            if !block.is_empty() {
                match &base {
                    Some(b) => Some(format!("{b}\n\n{block}")),
                    None => Some(block),
                }
            } else {
                base
            }
        } else {
            base
        };

        // Pin the skill catalog (FR-7, Tech Spec §7/§8.2). Only metadata is
        // standing context; bodies load via the `skill` tool (progressive
        // disclosure). Project skills are absent on an untrusted root.
        if self.skills.config.enabled && !self.skills.catalog_text.is_empty() {
            match &base {
                Some(b) => Some(format!("{b}\n\n{}", self.skills.catalog_text)),
                None => Some(self.skills.catalog_text.clone()),
            }
        } else {
            base
        }
    }

    fn make_ctx(&self) -> ToolCtx {
        ToolCtx::new(
            self.project_root.clone(),
            self.truncate,
            self.gates.permission.clone(),
            self.safety.spawn.clone(),
        )
        .with_ask_gate(self.gates.ask.clone())
        .with_recall_gate(self.gates.recall.clone())
        .with_task_list_gate(self.gates.task_list.clone())
        .with_memory_gate(self.gates.memory.clone())
        .with_skill_gate(self.gates.skill.clone())
        .with_scratch_gate(self.gates.scratch.clone())
        .with_vision(self.provider.client.model_info().vision)
        .with_image_max_bytes(self.image_max_bytes)
        .with_documents(self.provider.client.model_info().documents)
        .with_document_max_bytes(self.document_max_bytes)
    }
}
