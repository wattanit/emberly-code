//! The multi-agent subsystem (FR-9, T-18–T-21, Tech Spec §8.4).
//!
//! A subagent is a **nested `Engine`**, not a second implementation: its own
//! conversation, context window, compaction, loop guardrail, and completion
//! gate are the exact same code as any top-level session's. What is
//! deliberately *not* reused per subagent is the permission/ask-user state —
//! a subagent's `ToolCtx` gets [`SubagentPermissionGate`]/[`SubagentAskUserGate`]
//! (Tech Spec §8.4), proxying every request to *this* (the root) engine's own
//! channels instead of a fresh `RuleEngine`, so a subagent's tool calls are
//! decided by the exact same rule state as the primary agent's and can never
//! drift from it (Requirements FR-9 honesty clause).
//!
//! **Why the handlers below never `.await` a subagent's own turn directly:**
//! this module's entry point, [`Engine::on_subagent_ask`], runs from inside
//! [`turn::run_one_tool_call`](super::turn)'s own `tokio::select!` loop — the
//! *same* loop that watches `chans.asks` for permission asks a spawned
//! subagent's tool calls will proxy back in. If a handler here held `&mut
//! self` while awaiting a subagent's turn to finish, it would starve that
//! very select loop and the subagent's own proxied permission ask would never
//! be serviced — a real, if narrow, deadlock (broken only by the spawn
//! timeout, which defeats live interaction for that whole window). Every
//! handler here therefore does only *fast*, non-blocking work (build a
//! config, spawn tasks, send into a channel with spare buffer capacity) and
//! hands the actual waiting to a detached `tokio::spawn`ed task that holds no
//! reference to `self` at all.
//!
//! **Known v0.5 scope cut (honesty clause, tracked in
//! `docs/version-0-5/PHASE2_TODO.md`):** `list_agents` reports every alive
//! subagent as [`SubagentStatus::Running`] — a live, round-tripped status
//! (awaiting permission / done / timed out / error) is designed for in the
//! type but not wired up this phase.
//!
//! Idle reap (`AgentsConfig::idle_timeout_secs`) and `[agents]` config-file
//! reading are both implemented: [`Engine::reap_idle_subagents`] runs off a
//! periodic tick in the idle loop (`IDLE_REAP_CHECK_INTERVAL`, `engine::mod`)
//! rather than the per-turn select this module's other handlers use — it has
//! to fire even when the primary agent is doing nothing at all, which a
//! `TurnChannels`-scoped select never sees.

use tokio::sync::oneshot;

use super::*;

/// One turn to run on a subagent's own nested engine (the message a
/// `spawn_agents`/`message_agent` call wants it to process next), and the
/// reply once it finishes, times out, or errors.
struct SubagentTurnRequest {
    text: String,
    reply: oneshot::Sender<SubagentTurnOutcome>,
}

/// How a subagent's single turn concluded, as its driver task reports it
/// back — the shared internal shape both `SubagentSpawnOutcome` and
/// `SubagentMessageOutcome` are built from at their respective call sites.
enum SubagentTurnOutcome {
    Answered(String),
    Errored(String),
}

/// One currently alive subagent (Tech Spec §8.4). Dropping this (via
/// `end_agent` or the whole session ending) drops `turn_tx`, which closes the
/// per-subagent driver's loop, which drops its `commands_tx`, which closes
/// the subagent engine's own `commands_rx` and ends its `run()` (writing its
/// own `session_end`) — a full, cascading, ordinary-Rust-drop shutdown with
/// no explicit "kill" message needed anywhere.
pub(super) struct SubagentInstance {
    name: String,
    turn_tx: mpsc::Sender<SubagentTurnRequest>,
    /// Last time the primary agent interacted with this subagent (spawned
    /// it, or sent it a further prompt) — what `idle_timeout_secs` measures
    /// against (Tech Spec §8.4).
    last_activity: tokio::time::Instant,
    /// Where this subagent's own nested transcript lives (Tech Spec §3.2,
    /// §8.4) — the inspector (`Command::InspectAgent`) reads it directly
    /// rather than adding a second, live-streamed channel; the transcript is
    /// already the durable, per-event-fsynced record of everything the
    /// subagent did.
    transcript_path: PathBuf,
}

/// The multi-agent subsystem's own state (FR-9, Tech Spec §8.4): every
/// currently alive subagent, keyed by the id the model addresses it by, plus
/// the resource-bound tunables.
pub(super) struct AgentState {
    pub(super) config: AgentsConfig,
    pub(super) instances: std::collections::HashMap<String, SubagentInstance>,
    pub(super) next_seq: u64,
}

/// The four tool names a subagent's own registry never contains (the
/// structural depth bound, Requirements §2.2) — a subagent cannot itself
/// spawn a subagent because the tool simply is not registered for it, not
/// because of a runtime depth counter that could be gotten wrong.
const MULTI_AGENT_TOOL_NAMES: [&str; 4] =
    ["spawn_agents", "message_agent", "list_agents", "end_agent"];

/// Build the `ToolRegistry` a subagent runs with: never a superset of the
/// parent's own (Requirements FR-9) and never the four multi-agent tools
/// themselves. `requested` names a subset to further restrict to; `None`
/// means "everything the parent has, minus the four multi-agent tools,"
/// where the exclusion is silent (the ordinary default depth bound, not an
/// error). An *explicit* request naming a multi-agent tool, or any name
/// absent from the *parent's own* registry, is a structured spawn-time
/// failure (HC-6) — asking for one by name gets a clear reason, never a
/// silently smaller registry and never a silent grant of a capability the
/// session itself lacks.
fn build_subagent_tools(
    parent: &ToolRegistry,
    requested: Option<&[String]>,
) -> Result<ToolRegistry, String> {
    let mut registry = ToolRegistry::new();
    match requested {
        Some(names) => {
            for name in names {
                if MULTI_AGENT_TOOL_NAMES.contains(&name.as_str()) {
                    return Err(format!(
                        "'{name}' cannot be given to a subagent — a subagent may not itself \
                         spawn, message, list, or end subagents"
                    ));
                }
                match parent.get(name) {
                    Some(tool) => {
                        registry.register(tool);
                    }
                    None => return Err(format!("tool '{name}' is not available in this session")),
                }
            }
        }
        None => {
            for name in parent.names() {
                if MULTI_AGENT_TOOL_NAMES.contains(&name.as_str()) {
                    continue;
                }
                if let Some(tool) = parent.get(&name) {
                    registry.register(tool);
                }
            }
        }
    }
    Ok(registry)
}

/// A per-subagent driver: the "frontend" that drives one subagent's nested
/// `Engine` (A-1/A-2's promise that a fake/headless frontend can drive the
/// engine, put to a genuine production use). Owns the subagent's `events_rx`
/// for its whole life — no handoff, no race over who reads it. Loops on
/// `turn_rx`: each request sends `Command::UserInput` into the subagent
/// engine, drains its events accumulating assistant text until `TurnEnded`,
/// reports the turn's token/cost delta back to the root (`subagent_tx`,
/// Requirements FR-9 — "cost is never hidden"), and replies. Exits (ending
/// the chain, see [`SubagentInstance`]'s doc) when `turn_rx` closes.
async fn run_subagent_driver(
    commands_tx: mpsc::Sender<Command>,
    mut events_rx: mpsc::Receiver<UiEvent>,
    mut turn_rx: mpsc::Receiver<SubagentTurnRequest>,
    subagent_tx: mpsc::Sender<SubagentAsk>,
) {
    // The subagent's own `Engine` reports its *cumulative* session usage/cost
    // after every completion (`emit_context_usage`); tracking the last-seen
    // total here and reporting only the delta after each turn is what lets
    // the root simply *add* what it receives, with no risk of double-
    // counting a turn's usage across multiple `ReportUsage` messages.
    let mut last_usage = TokenUsage::default();
    let mut last_cost = 0.0_f64;
    while let Some(SubagentTurnRequest { text, reply }) = turn_rx.recv().await {
        if commands_tx.send(Command::UserInput { text }).await.is_err() {
            let _ = reply.send(SubagentTurnOutcome::Errored(
                "the subagent's engine is gone".into(),
            ));
            continue;
        }
        let mut answer = String::new();
        let mut errored = false;
        let mut turn_usage = last_usage;
        let mut turn_cost = last_cost;
        loop {
            match events_rx.recv().await {
                Some(UiEvent::AssistantDelta { text }) => answer.push_str(&text),
                Some(UiEvent::TurnEnded) => break,
                Some(UiEvent::HarnessError { what, why, .. }) => {
                    errored = true;
                    answer = format!("{what}: {why}");
                }
                Some(UiEvent::SessionUsage { usage }) => turn_usage = usage,
                Some(UiEvent::CostEstimate { usd, .. }) => turn_cost = usd,
                // Tool activity, context usage, and everything else the
                // subagent's own turn emits is not surfaced to the caller
                // (Design §4.13 — no raw concurrent streaming); it is
                // recorded in the subagent's own transcript instead.
                Some(_) => {}
                None => {
                    errored = true;
                    if answer.is_empty() {
                        answer = "the subagent's engine ended unexpectedly".into();
                    }
                    break;
                }
            }
        }
        let usage_delta = TokenUsage {
            input: turn_usage.input.saturating_sub(last_usage.input),
            output: turn_usage.output.saturating_sub(last_usage.output),
        };
        let cost_delta = (turn_cost - last_cost).max(0.0);
        last_usage = turn_usage;
        last_cost = turn_cost;
        if usage_delta.input > 0 || usage_delta.output > 0 || cost_delta > 0.0 {
            let _ = subagent_tx
                .send(SubagentAsk::ReportUsage {
                    usage: usage_delta,
                    cost_usd: cost_delta,
                })
                .await;
        }
        let outcome = if errored {
            SubagentTurnOutcome::Errored(answer)
        } else {
            SubagentTurnOutcome::Answered(answer)
        };
        let _ = reply.send(outcome);
    }
}

impl Engine {
    /// Dispatch one multi-agent lifecycle ask (T-18–T-21) to its handler.
    pub(super) async fn on_subagent_ask(&mut self, ask: SubagentAsk) {
        match ask {
            SubagentAsk::Spawn { req, reply } => self.spawn_subagents(req, reply).await,
            SubagentAsk::Message { req, reply } => self.message_subagent(req, reply).await,
            SubagentAsk::List { reply } => {
                let entries = self
                    .agents
                    .instances
                    .iter()
                    .map(|(id, inst)| SubagentListEntry {
                        id: id.clone(),
                        name: inst.name.clone(),
                        // v1 simplification (module docs): a live round-tripped
                        // status is designed for but not wired up this phase.
                        status: SubagentStatus::Running,
                    })
                    .collect();
                let _ = reply.send(Ok(entries));
            }
            SubagentAsk::End { id, reply } => {
                let outcome = if self.agents.instances.remove(&id).is_some() {
                    self.emit(UiEvent::SubagentEnded {
                        id,
                        reason: "ended".into(),
                    })
                    .await;
                    SubagentEndOutcome::Ended
                } else {
                    SubagentEndOutcome::NotFound
                };
                let _ = reply.send(Ok(outcome));
            }
            SubagentAsk::ReportUsage { usage, cost_usd } => {
                self.roll_up_subagent_usage(usage, cost_usd).await;
            }
        }
    }

    /// Fold a subagent's turn-delta token usage and cost into the session's
    /// own running total (Requirements FR-9 — delegated work is still the
    /// session's spend, never hidden), and re-announce both — the same
    /// events `emit_context_usage` sends after the primary agent's own
    /// turns, so the sidebar total is never stale after a delegation.
    async fn roll_up_subagent_usage(&mut self, usage: TokenUsage, cost_usd: f64) {
        self.session.usage.input = self.session.usage.input.saturating_add(usage.input);
        self.session.usage.output = self.session.usage.output.saturating_add(usage.output);
        self.session.cost_usd += cost_usd;
        // The updated total belongs in the next view-cache write (FR-5,
        // Tech Spec §3.2a) even though nothing was appended to the
        // transcript by this rollup itself.
        self.session.cache_dirty = true;
        self.emit(UiEvent::SessionUsage {
            usage: self.session.usage,
        })
        .await;
        if cost_usd > 0.0 {
            self.emit(UiEvent::CostEstimate {
                usage: self.session.usage,
                usd: self.session.cost_usd,
            })
            .await;
        }
    }

    /// End every subagent that has gone longer than `idle_timeout_secs`
    /// without a `message_agent` call (Tech Spec §8.4) — called from the
    /// idle loop's periodic tick (`IDLE_REAP_CHECK_INTERVAL`), so this fires
    /// even when the primary agent itself is doing nothing at all, not only
    /// during its own turns. Dropping a `SubagentInstance` cascades the same
    /// clean shutdown as an explicit `end_agent` (see its own doc comment).
    pub(super) async fn reap_idle_subagents(&mut self) {
        if self.agents.instances.is_empty() {
            return;
        }
        let timeout = std::time::Duration::from_secs(self.agents.config.idle_timeout_secs);
        let now = tokio::time::Instant::now();
        let expired: Vec<String> = self
            .agents
            .instances
            .iter()
            .filter(|(_, inst)| now.duration_since(inst.last_activity) >= timeout)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.agents.instances.remove(&id);
            self.emit(UiEvent::SubagentEnded {
                id,
                reason: "idle timeout".into(),
            })
            .await;
        }
    }

    /// Spawn one or more subagents and hand back their first-turn results
    /// once every one finishes, times out, or fails (T-18). Fast: builds
    /// configs and kicks off tasks, then hands the actual waiting to a
    /// detached task (see the module docs on why this never holds `&mut
    /// self` across a subagent's own turn).
    async fn spawn_subagents(
        &mut self,
        req: SubagentSpawnBatch,
        reply: oneshot::Sender<Result<Vec<SubagentSpawnResult>, SubagentError>>,
    ) {
        if !self.agents.config.enabled {
            let _ = reply.send(Err(SubagentError));
            return;
        }
        let capacity = self
            .agents
            .config
            .max_concurrent
            .saturating_sub(self.agents.instances.len());
        let mut waiters: Vec<(String, String, oneshot::Receiver<SubagentTurnOutcome>)> = Vec::new();
        let mut immediate: Vec<SubagentSpawnResult> = Vec::new();

        for (i, spec) in req.agents.into_iter().enumerate() {
            if i >= capacity {
                immediate.push(SubagentSpawnResult {
                    id: String::new(),
                    name: spec.name,
                    outcome: SubagentSpawnOutcome::Failed(format!(
                        "max_concurrent ({}) reached — this subagent was not spawned",
                        self.agents.config.max_concurrent
                    )),
                });
                continue;
            }
            let name = spec.name.clone();
            match self.spawn_one_subagent(spec).await {
                Ok((id, turn_reply_rx)) => waiters.push((id, name, turn_reply_rx)),
                Err(reason) => immediate.push(SubagentSpawnResult {
                    id: String::new(),
                    name,
                    outcome: SubagentSpawnOutcome::Failed(reason),
                }),
            }
        }

        let timeout = std::time::Duration::from_secs(self.agents.config.spawn_timeout_secs);
        // Every subagent's own engine + driver task is already spawned and
        // running independently by this point (the loop above); `join_all`
        // here is what makes waiting for them genuinely concurrent rather
        // than accidentally serialized — each subagent's own timeout runs on
        // its own clock, not stacked behind the ones before it (T-18).
        tokio::spawn(async move {
            let waited = futures::future::join_all(waiters.into_iter().map(
                |(id, name, turn_reply_rx)| async move {
                    let outcome = match tokio::time::timeout(timeout, turn_reply_rx).await {
                        Ok(Ok(SubagentTurnOutcome::Answered(text))) => {
                            SubagentSpawnOutcome::Answered(text)
                        }
                        Ok(Ok(SubagentTurnOutcome::Errored(reason))) => {
                            SubagentSpawnOutcome::Failed(reason)
                        }
                        Ok(Err(_)) => {
                            SubagentSpawnOutcome::Failed("the subagent's driver is gone".into())
                        }
                        Err(_) => SubagentSpawnOutcome::StillRunning,
                    };
                    SubagentSpawnResult { id, name, outcome }
                },
            ))
            .await;
            let mut results = immediate;
            results.extend(waited);
            let _ = reply.send(Ok(results));
        });
    }

    /// Send a further prompt to a specific, still-alive subagent (T-19).
    /// Fast for the same reason as `spawn_subagents`: the wait for the
    /// subagent's next turn happens in a detached task, never here.
    async fn message_subagent(
        &mut self,
        req: SubagentMessageRequest,
        reply: oneshot::Sender<Result<SubagentMessageOutcome, SubagentError>>,
    ) {
        let Some(instance) = self.agents.instances.get_mut(&req.id) else {
            let _ = reply.send(Ok(SubagentMessageOutcome::NotFound));
            return;
        };
        instance.last_activity = tokio::time::Instant::now();
        let turn_tx = instance.turn_tx.clone();
        let (turn_reply_tx, turn_reply_rx) = oneshot::channel();
        if turn_tx
            .send(SubagentTurnRequest {
                text: req.message,
                reply: turn_reply_tx,
            })
            .await
            .is_err()
        {
            let _ = reply.send(Ok(SubagentMessageOutcome::NotFound));
            return;
        }
        let timeout = std::time::Duration::from_secs(self.agents.config.spawn_timeout_secs);
        tokio::spawn(async move {
            let outcome = match tokio::time::timeout(timeout, turn_reply_rx).await {
                Ok(Ok(SubagentTurnOutcome::Answered(text))) => {
                    SubagentMessageOutcome::Replied(text)
                }
                Ok(Ok(SubagentTurnOutcome::Errored(reason))) => {
                    SubagentMessageOutcome::Failed(reason)
                }
                Ok(Err(_)) => {
                    SubagentMessageOutcome::Failed("the subagent's driver is gone".into())
                }
                Err(_) => SubagentMessageOutcome::Failed(
                    "the subagent is still running past the per-call timeout — try again later"
                        .into(),
                ),
            };
            let _ = reply.send(Ok(outcome));
        });
    }

    /// Construct and launch one subagent: its own nested `Engine`, its own
    /// transcript, its own driver task — then send its first turn request.
    /// Returns its new id and the oneshot the caller awaits for that first
    /// turn's outcome.
    async fn spawn_one_subagent(
        &mut self,
        spec: SubagentSpawnSpec,
    ) -> Result<(String, oneshot::Receiver<SubagentTurnOutcome>), String> {
        let tools = build_subagent_tools(&self.tools, spec.tools.as_deref())?;
        let (provider, provider_label, model) = self.resolve_subagent_provider(&spec)?;
        let id = self.next_agent_id();
        let session_id = SessionId::new();
        let system = self.compose_subagent_system_prompt(&spec.system_prompt);

        let (events_tx, events_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let event_profile = provider_label.clone();
        let event_model = model.clone();
        let config = self.build_subagent_engine_config(
            &spec,
            provider,
            provider_label,
            model,
            tools,
            system,
            session_id,
        );
        let (engine, asks_rx, user_asks_rx, recall_rx, task_rx, memory_rx, skill_rx, subagent_rx) =
            Engine::new(config, events_tx);
        let (commands_tx, commands_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        // `Engine::run` returns an explicitly boxed future (see its own doc
        // comment): the standard fix for the recursive async call graph a
        // subagent's own tool use creates (this very function spawning
        // *another* nested `Engine::run`).
        tokio::spawn(engine.run(
            commands_rx,
            asks_rx,
            user_asks_rx,
            recall_rx,
            task_rx,
            memory_rx,
            skill_rx,
            subagent_rx,
        ));

        let (turn_tx, turn_rx) = mpsc::channel(4);
        tokio::spawn(run_subagent_driver(
            commands_tx,
            events_rx,
            turn_rx,
            self.gates.subagent_tx.clone(),
        ));

        let (first_reply_tx, first_reply_rx) = oneshot::channel();
        // The channel is brand new with spare capacity, so this resolves
        // immediately regardless of whether the driver has started polling
        // yet — not a wait on the subagent's own progress.
        let _ = turn_tx
            .send(SubagentTurnRequest {
                text: spec.system_prompt.clone(),
                reply: first_reply_tx,
            })
            .await;

        self.agents.instances.insert(
            id.clone(),
            SubagentInstance {
                name: spec.name.clone(),
                turn_tx,
                last_activity: tokio::time::Instant::now(),
                transcript_path: self.subagent_transcript_path(session_id),
            },
        );
        self.emit(UiEvent::SubagentSpawned {
            id: id.clone(),
            name: spec.name,
            profile: event_profile,
            model: event_model,
        })
        .await;
        Ok((id, first_reply_rx))
    }

    fn next_agent_id(&mut self) -> String {
        self.agents.next_seq += 1;
        format!("agent-{}", self.agents.next_seq)
    }

    /// Resolve the provider a subagent runs under (P-8): an explicitly named,
    /// already-configured profile, or — the default — the parent's own
    /// active provider and model, at zero factory-call cost.
    fn resolve_subagent_provider(
        &self,
        spec: &SubagentSpawnSpec,
    ) -> Result<(Arc<dyn Provider>, String, String), String> {
        match &spec.profile {
            Some(profile) => {
                let factory = self.provider.factory.as_ref().ok_or_else(|| {
                    "no provider profiles are configured for this session".to_string()
                })?;
                let model = spec
                    .model
                    .clone()
                    .unwrap_or_else(|| self.provider.model.clone());
                let choice = factory.build(profile, &model)?;
                Ok((choice.provider, choice.profile, choice.model))
            }
            None => Ok((
                self.provider.client.clone(),
                self.provider.label.clone(),
                self.provider.model.clone(),
            )),
        }
    }

    /// Compose a subagent's system prompt (Requirements FR-9): the parent's
    /// own already-resolved system prompt (the harness's baked-in tool-use
    /// scaffold plus any project instructions) with the spawn call's
    /// model-authored persona/task layered underneath a short explanatory
    /// preamble — never a bare replacement of the scaffold.
    fn compose_subagent_system_prompt(&self, persona_task: &str) -> String {
        let preamble = "You are a subagent delegated by the primary agent for a specific task. \
            Stay focused on it and report your findings/results clearly when you are done.";
        match &self.provider.system {
            Some(base) => format!("{base}\n\n---\n{preamble}\n\n{persona_task}"),
            None => format!("{preamble}\n\n{persona_task}"),
        }
    }

    /// The directory a subagent's own nested transcript lives in (Tech Spec
    /// §3.2/§8.4): `.agents/sessions/<parent-session-id>/subagents/`.
    fn subagents_dir(&self) -> PathBuf {
        self.session
            .dir
            .join(self.session.id.to_string())
            .join("subagents")
    }

    /// The exact path a subagent's own transcript lives at, given its
    /// internal `SessionId` — shared by construction (`build_subagent_engine_config`)
    /// and lookup (`inspect_agent`) so the two can never disagree.
    fn subagent_transcript_path(&self, session_id: SessionId) -> PathBuf {
        self.subagents_dir().join(format!("{session_id}.jsonl"))
    }

    /// Derive a subagent's `EngineConfig` from this (the root) engine's own
    /// current state (Tech Spec §8.4): the same project root, sandbox
    /// confinement, image/document caps, tool-explanation setting, and
    /// loop/context/completion tunables; its own nested transcript; and, the
    /// load-bearing part, a permission/ask gate proxied to *this* engine
    /// rather than a fresh `RuleEngine`.
    #[allow(clippy::too_many_arguments)]
    fn build_subagent_engine_config(
        &self,
        spec: &SubagentSpawnSpec,
        provider: Arc<dyn Provider>,
        provider_label: String,
        model: String,
        tools: ToolRegistry,
        system: String,
        session_id: SessionId,
    ) -> EngineConfig {
        let subagents_dir = self.subagents_dir();
        let transcript: Box<dyn TranscriptSink> =
            match FileTranscript::create(&subagents_dir, session_id) {
                Ok(sink) => Box::new(sink),
                // Best-effort, matching HC-3's "never crash, degrade instead":
                // a subagent's own audit trail is valuable but its absence must
                // never block spawning the subagent itself.
                Err(_) => EngineConfig::no_transcript(),
            };
        EngineConfig {
            provider,
            tools,
            project_root: self.project_root.clone(),
            model,
            system: Some(system),
            tool_explanations: self.tool_explanations,
            trust_granted: false,
            loop_config: self.guardrail.config,
            completion_config: self.completion.config.clone(),
            // Deliberately empty: the session's own registered completion
            // checks (e.g. "cargo test must pass") gate the *primary* task's
            // claim of done, not an unrelated delegated subtask's first
            // natural stop. An inert gate behaves exactly as no gate (Tech
            // Spec §7).
            completion_checks: Vec::new(),
            context: self.context.config,
            truncate: self.truncate,
            retry: self.retry,
            session_id,
            sessions_dir: subagents_dir,
            active_session_path: Arc::new(RwLock::new(PathBuf::new())),
            provider_label,
            configured_provider: None,
            configured_model: None,
            sandbox: self.safety.sandbox.clone(),
            // Inert: tool permission is proxied to the root (below), never
            // locally evaluated — this value is never consulted.
            rules: RuleEngine::new(Vec::new(), false),
            rule_specs: Vec::new(),
            sandbox_spawn: Some(self.safety.spawn.clone()),
            config_provenance: Vec::new(),
            transcript,
            initial_conversation: Vec::new(),
            resuming: false,
            compacted: false,
            initial_cache: None,
            replayed: false,
            summary_prompt: None,
            // No in-session `/model` switching surface exists for a
            // subagent, so it needs no factory of its own.
            provider_factory: None,
            config_reloader: None,
            image_max_bytes: self.image_max_bytes,
            document_max_bytes: self.document_max_bytes,
            // Directory-shared, not Arc-shared: a subagent's own `MemoryStore`/
            // `SkillCatalog` instance is constructed from the same underlying
            // directories as the parent's (Tech Spec §8.4), so it reads and
            // writes the same durable memory and the same skill catalog on
            // disk — an implementation refinement over holding the literal
            // same in-process `Arc`, which would need its own override seam
            // for no correctness benefit (the files are the source of truth).
            memory: self.memory.config.clone(),
            user_memory_dir: self.memory.user_dir.clone(),
            project_memory_dir: self.memory.project_dir.clone(),
            skills: self.skills.config.clone(),
            user_skills_dir: self.skills.user_dir.clone(),
            project_skills_dir: self.skills.project_dir.clone(),
            agents: AgentsConfig::default(),
            external_permission_gate: Some(Arc::new(SubagentPermissionGate {
                tx: self.gates.permission_tx.clone(),
                label: spec.name.clone(),
            })),
            external_ask_gate: Some(Arc::new(SubagentAskUserGate {
                tx: self.gates.ask_tx.clone(),
            })),
        }
    }

    /// Fetch a subagent's own activity for the inspector (Design §4.13), in
    /// reply to `Command::InspectAgent`. Reads that subagent's own nested
    /// transcript directly (Tech Spec §3.2/§8.4) rather than adding a second,
    /// live-streamed channel — the transcript is already the durable,
    /// per-event-fsynced record of everything the subagent did, so this is a
    /// read-only snapshot as of the last flush, not a continuously live view.
    /// An unknown or already-ended id still replies, with a body saying so.
    pub(super) async fn inspect_agent(&mut self, id: String) {
        let Some(instance) = self.agents.instances.get(&id) else {
            self.emit(UiEvent::AgentActivity {
                name: id.clone(),
                id,
                text: "No alive subagent with this id. It may not exist, or it has already ended."
                    .into(),
            })
            .await;
            return;
        };
        let name = instance.name.clone();
        let path = instance.transcript_path.clone();
        let text = match crate::resume::read_records(&path) {
            Ok(loaded) => {
                format_agent_activity(&crate::resume::rebuild_conversation(&loaded.records))
            }
            // Best-effort, matching HC-3's "never crash, degrade instead": no
            // transcript yet (or unreadable) reads as "nothing to show," never
            // a harness error surfaced through this reply.
            Err(_) => "This subagent's activity is not available yet.".into(),
        };
        self.emit(UiEvent::AgentActivity { id, name, text }).await;
    }
}

/// Render a subagent's rebuilt conversation as plain, readable text for the
/// inspector overlay (Design §4.13) — a first cut: role-tagged lines, tool
/// calls/results named plainly. Not meant to be pretty, only informative;
/// the frontend shows it verbatim via the same read-only text-overlay path
/// `SkillBody` already uses.
fn format_agent_activity(messages: &[Message]) -> String {
    if messages.is_empty() {
        return "(no activity recorded yet)".to_string();
    }
    let mut out = String::new();
    for message in messages {
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    out.push_str(&format!("[{role}] {text}\n"));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    out.push_str(&format!("[{role}] tool_use: {name} {input}\n"));
                }
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let tag = if *is_error {
                        "tool error"
                    } else {
                        "tool result"
                    };
                    out.push_str(&format!("[{tag}] {content}\n"));
                }
                ContentBlock::Reasoning { text, .. } => {
                    out.push_str(&format!("[{role} reasoning] {text}\n"));
                }
                ContentBlock::Image { .. } => out.push_str(&format!("[{role}] (image)\n")),
                ContentBlock::Document { .. } => out.push_str(&format!("[{role}] (document)\n")),
            }
        }
    }
    out
}
