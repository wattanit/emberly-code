//! Agent-loop integration tests (Phase 1, groups 6 + 7). A `FakeProvider`
//! scripts the model side; the test plays the frontend — sending `UserInput`,
//! answering `PermissionRequest`s, and observing `UiEvent`s. Exercises the
//! gate round trip, tool-result feedback, denial-as-data (HC-6), `FileModified`,
//! and cancellation.
//!
//! No `.unwrap()`/`.expect()`: setup `panic!`s with context; the frontend
//! reads events and asserts on them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use emberly_core::{
    channel, AskAnswer, CaptureSink, Command, Engine, EngineConfig, FileTranscript, Mode,
    PermissionDecision, RetryPolicy, RuleEngine, RuleSource, SandboxStatus, SessionId,
    TranscriptEvent, TranscriptSink, UiEvent,
};
use emberly_providers::{
    ContentBlock, Effort, FakeProvider, Message, ModelInfo, Pricing, Provider, ProviderError, Role,
    ScriptOutcome, ScriptedResponse, StopReason, StreamEvent, TokenUsage,
};
use emberly_tools::{default_registry, TruncateConfig};
use tokio::sync::mpsc;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_project() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("emberly-engine-{}-{}", std::process::id(), n));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        panic!("failed to create temp project dir: {e}");
    }
    dir
}

struct Harness {
    commands_tx: mpsc::Sender<Command>,
    events_rx: mpsc::Receiver<UiEvent>,
}

fn start(scripts: Vec<ScriptedResponse>, root: PathBuf) -> Harness {
    start_with_provider(Arc::new(FakeProvider::new(scripts)), root)
}

fn make_config(
    provider: Arc<dyn Provider>,
    root: PathBuf,
    transcript: Box<dyn TranscriptSink>,
) -> EngineConfig {
    EngineConfig {
        provider,
        tools: default_registry(),
        project_root: root,
        model: "fake-1".into(),
        system: None,
        // Off by default here so existing tests see byte-identical requests;
        // the T-9 tests flip this field on the returned config explicitly.
        tool_explanations: false,
        trust_granted: false,
        truncate: TruncateConfig::default(),
        // Fast retries so retry tests don't wait on real backoff.
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        },
        session_id: SessionId::new(),
        sessions_dir: std::env::temp_dir(),
        active_session_path: std::sync::Arc::new(std::sync::RwLock::new(std::path::PathBuf::new())),
        provider_label: "fake".into(),
        sandbox: SandboxStatus::Unavailable {
            reason: "test".into(),
        },
        // Degraded (allowlist suspended) → every bash asks, matching the Phase 1
        // gate behavior these tests were written against.
        rules: RuleEngine::new(Vec::new(), false),
        // Tests run bash plainly even when reporting a confined status, so the
        // self-exec shim never re-executes the test binary.
        sandbox_spawn: Some(std::sync::Arc::new(emberly_tools::PlainSandbox)),
        config_provenance: Vec::new(),
        transcript,
        initial_conversation: Vec::new(),
        resuming: false,
        summary_prompt: None,
        provider_factory: None,
        config_reloader: None,
    }
}

fn start_with_provider(provider: Arc<dyn Provider>, root: PathBuf) -> Harness {
    spawn(make_config(provider, root, EngineConfig::no_transcript()))
}

/// Start a session with an in-memory transcript sink the test can inspect.
fn start_capturing(scripts: Vec<ScriptedResponse>, root: PathBuf) -> (Harness, CaptureSink) {
    let sink = CaptureSink::new();
    let config = make_config(
        Arc::new(FakeProvider::new(scripts)),
        root,
        Box::new(sink.clone()),
    );
    (spawn(config), sink)
}

/// Start a session that writes its durable transcript to `path` on disk (the
/// real [`FileTranscript`] sink, per-event fsync) — the resume round-trip setup.
fn start_with_file_transcript(
    scripts: Vec<ScriptedResponse>,
    root: PathBuf,
    path: &Path,
) -> Harness {
    let sink = match FileTranscript::open(path) {
        Ok(sink) => sink,
        Err(error) => panic!("open transcript {}: {error}", path.display()),
    };
    let config = make_config(Arc::new(FakeProvider::new(scripts)), root, Box::new(sink));
    spawn(config)
}

fn spawn(config: EngineConfig) -> Harness {
    let (engine_ports, frontend) = channel();
    let (engine, asks_rx, user_asks_rx) = Engine::new(config, engine_ports.events_tx);
    tokio::spawn(engine.run(engine_ports.commands_rx, asks_rx, user_asks_rx));
    Harness {
        commands_tx: frontend.commands_tx,
        events_rx: frontend.events_rx,
    }
}

impl Harness {
    async fn send(&self, command: Command) {
        let _ = self.commands_tx.send(command).await;
    }

    /// Collect events until the stream goes idle, auto-answering permission
    /// prompts with `answer` (if any).
    async fn collect(&mut self, answer: Option<PermissionDecision>) -> Vec<UiEvent> {
        let mut events = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(250), self.events_rx.recv()).await
        {
            if let UiEvent::PermissionRequest { id, .. } = &event {
                if let Some(decision) = answer {
                    self.send(Command::PermissionAnswer { id: *id, decision })
                        .await;
                }
            }
            events.push(event);
        }
        events
    }
}

/// Spawn a session with active confinement and a given rule engine — the setup
/// for exercising rule-driven allow/deny and the auto modes (which are gated on
/// confinement).
fn spawn_confined(scripts: Vec<ScriptedResponse>, root: PathBuf, rules: RuleEngine) -> Harness {
    let mut config = make_config(
        Arc::new(FakeProvider::new(scripts)),
        root,
        EngineConfig::no_transcript(),
    );
    config.sandbox = SandboxStatus::Confined {
        backend: "test".into(),
    };
    config.rules = rules;
    spawn(config)
}

fn prompt_count(events: &[UiEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, UiEvent::PermissionRequest { .. }))
        .count()
}

fn deltas(events: &[UiEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            UiEvent::AssistantDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn has_tool_finished(events: &[UiEvent], ok: bool) -> bool {
    events
        .iter()
        .any(|e| matches!(e, UiEvent::ToolFinished { ok: o, .. } if *o == ok))
}

#[tokio::test]
async fn text_only_turn_streams_and_reports_usage() {
    let mut h = start(vec![ScriptedResponse::text("hello สวัสดี")], temp_project());
    h.send(Command::UserInput { text: "hi".into() }).await;
    let events = h.collect(None).await;

    assert_eq!(deltas(&events), "hello สวัสดี");
    assert!(events.iter().any(|e| matches!(e, UiEvent::AssistantDone)));
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::ContextUsage { .. })));
}

#[tokio::test]
async fn tool_call_allowed_runs_and_feeds_result_back() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call(
            "c1",
            "write_file",
            r#"{"path":"out.txt","content":"hello\n"}"#,
        ),
        ScriptedResponse::text("done"),
    ];
    let mut h = start(scripts, root.clone());
    h.send(Command::UserInput {
        text: "write it".into(),
    })
    .await;
    let events = h.collect(Some(PermissionDecision::AllowOnce)).await;

    // The file was created.
    assert_eq!(
        std::fs::read_to_string(root.join("out.txt")).unwrap_or_default(),
        "hello\n"
    );
    // A prompt was raised, the tool succeeded, and the change was surfaced.
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::PermissionRequest { .. })));
    assert!(has_tool_finished(&events, true));
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::FileModified { path, adds, dels } if path == "out.txt" && *adds == 1 && *dels == 0
    )));
    // The model got the tool result and produced a closing message.
    assert_eq!(deltas(&events), "done");
}

#[tokio::test]
async fn transcript_records_the_durable_session() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "write_file", r#"{"path":"out.txt","content":"hi\n"}"#),
        ScriptedResponse::text("done"),
    ];
    let (mut h, sink) = start_capturing(scripts, root);
    h.send(Command::UserInput {
        text: "write it".into(),
    })
    .await;
    let _ = h.collect(Some(PermissionDecision::AllowOnce)).await;

    let events: Vec<TranscriptEvent> = sink.records().into_iter().map(|r| r.event).collect();
    // Opens with session_start.
    assert!(matches!(
        events.first(),
        Some(TranscriptEvent::SessionStart { .. })
    ));
    // First user message is pinned as the original task, and titles the session.
    assert!(events.iter().any(|e| matches!(
        e,
        TranscriptEvent::UserMessage {
            original_task: true,
            ..
        }
    )));
    assert!(events
        .iter()
        .any(|e| matches!(e, TranscriptEvent::SessionTitle { .. })));
    // The tool call, its permission round trip, and its result are all recorded.
    assert!(events
        .iter()
        .any(|e| matches!(e, TranscriptEvent::ToolCall { tool, .. } if tool == "write_file")));
    assert!(events
        .iter()
        .any(|e| matches!(e, TranscriptEvent::PermissionRequest { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        TranscriptEvent::PermissionDecision {
            decision: PermissionDecision::AllowOnce,
            ..
        }
    )));
    assert!(events
        .iter()
        .any(|e| matches!(e, TranscriptEvent::ToolResult { ok: true, .. })));
    // The closing assistant message is stored complete, not as deltas.
    assert!(events
        .iter()
        .any(|e| matches!(e, TranscriptEvent::AssistantMessage { text, .. } if text == "done")));

    // Closing the command channel ends the session cleanly.
    drop(h);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(matches!(
        sink.records().last().map(|r| r.event.clone()),
        Some(TranscriptEvent::SessionEnd { .. })
    ));
}

#[tokio::test]
async fn file_sink_session_resumes_to_an_identical_view() {
    // Group 10 round-trip (A-2, §3.3): run a scripted session through the real
    // on-disk FileTranscript sink, then resume from the written file and assert
    // the rebuilt conversation view is exactly what the session produced.
    let root = temp_project();
    let path = root.join("session.jsonl");
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "write_file", r#"{"path":"out.txt","content":"hi\n"}"#),
        ScriptedResponse::text("all done"),
    ];
    let mut h = start_with_file_transcript(scripts, root.clone(), &path);
    h.send(Command::UserInput {
        text: "write it".into(),
    })
    .await;
    let _ = h.collect(Some(PermissionDecision::AllowOnce)).await;
    // Closing the command channel makes the engine write a clean session_end.
    drop(h);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Resume: read the transcript back the way `emberly resume` does.
    let loaded = match emberly_core::resume::read_records(&path) {
        Ok(loaded) => loaded,
        Err(error) => panic!("read transcript: {error}"),
    };
    assert!(
        loaded.warnings.is_empty(),
        "a freshly-written transcript reads clean: {:?}",
        loaded.warnings
    );
    assert!(
        !emberly_core::resume::interrupted(&loaded.records),
        "the session ended cleanly, so resume is not offered"
    );

    let view = emberly_core::resume::rebuild_conversation(&loaded.records);
    // user · assistant(tool_use write_file) · tool_result · assistant(closing).
    assert_eq!(view.len(), 4, "four model-visible turns");
    assert_eq!(view[0], Message::user_text("write it"));
    assert_eq!(view[1].role, Role::Assistant);
    assert!(matches!(
        view[1].content.last(),
        Some(ContentBlock::ToolUse { name, .. }) if name == "write_file"
    ));
    assert_eq!(view[2].role, Role::Tool);
    assert!(matches!(
        view[2].content.first(),
        Some(ContentBlock::ToolResult { is_error, .. }) if !*is_error
    ));
    assert_eq!(view[3], Message::assistant_text("all done"));
}

#[tokio::test]
async fn compact_summarizes_the_middle_and_records_the_event() {
    // Four text turns build 8 messages; the fifth scripted response is consumed
    // by the summarization call that `/compact` makes.
    let scripts = vec![
        ScriptedResponse::text("r1"),
        ScriptedResponse::text("r2"),
        ScriptedResponse::text("r3"),
        ScriptedResponse::text("r4"),
        ScriptedResponse::text("SUMMARY OF THE MIDDLE"),
    ];
    let (mut h, sink) = start_capturing(scripts, temp_project());
    for i in 0..4 {
        h.send(Command::UserInput {
            text: format!("msg {i}"),
        })
        .await;
        let _ = h.collect(None).await;
    }

    h.send(Command::Compact).await;
    let events = h.collect(None).await;

    // The UI is told compaction happened.
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::CompactionStatus { message } if message.contains("compacted")
    )));
    // The transcript records the compaction with the model's summary.
    assert!(sink.records().iter().any(|r| matches!(
        &r.event,
        TranscriptEvent::Compaction { summary, .. } if summary == "SUMMARY OF THE MIDDLE"
    )));
}

#[tokio::test]
async fn denied_tool_feeds_failure_and_model_continues() {
    // HC-6 / §6.6: a denial is data the model reacts to, not a dead end.
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
        ScriptedResponse::text("understood, skipping"),
    ];
    let mut h = start(scripts, root.clone());
    h.send(Command::UserInput {
        text: "write it".into(),
    })
    .await;
    let events = h.collect(Some(PermissionDecision::Deny)).await;

    assert!(
        !root.join("out.txt").exists(),
        "denied write must not touch disk"
    );
    assert!(
        has_tool_finished(&events, false),
        "denied tool finishes as failure"
    );
    assert_eq!(deltas(&events), "understood, skipping");
}

#[tokio::test]
async fn unknown_tool_is_a_recoverable_failure() {
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "no_such_tool", "{}"),
        ScriptedResponse::text("recovered"),
    ];
    let mut h = start(scripts, temp_project());
    h.send(Command::UserInput { text: "go".into() }).await;
    let events = h.collect(Some(PermissionDecision::AllowOnce)).await;

    assert!(has_tool_finished(&events, false));
    assert_eq!(deltas(&events), "recovered");
}

/// The Phase 1 exit criterion (IMPLEMENTATION_PLAN.md): a scripted session
/// reads a file, proposes an edit, prompts for permission, runs a bash
/// command, and terminates — all end-to-end through the engine.
#[tokio::test]
async fn full_workflow_read_edit_permission_bash() {
    let root = temp_project();
    if let Err(e) = std::fs::write(root.join("f.txt"), "hello\nworld\n") {
        panic!("seed failed: {e}");
    }

    let scripts = vec![
        ScriptedResponse::tool_call("c1", "read_file", r#"{"path":"f.txt"}"#),
        ScriptedResponse::tool_call(
            "c2",
            "edit_file",
            r#"{"path":"f.txt","old_string":"world","new_string":"emberly"}"#,
        ),
        ScriptedResponse::tool_call("c3", "bash", r#"{"command":"echo done"}"#),
        ScriptedResponse::text("workflow complete"),
    ];
    let mut h = start(scripts, root.clone());
    h.send(Command::UserInput {
        text: "do the workflow".into(),
    })
    .await;
    let events = h.collect(Some(PermissionDecision::AllowOnce)).await;

    // The edit landed and bash ran; the model produced its closing message.
    assert_eq!(
        std::fs::read_to_string(root.join("f.txt")).unwrap_or_default(),
        "hello\nemberly\n"
    );
    assert_eq!(deltas(&events), "workflow complete");

    // The in-root read did NOT prompt; the edit and bash did (HC-4/§6.2).
    let prompts = events
        .iter()
        .filter(|e| matches!(e, UiEvent::PermissionRequest { .. }))
        .count();
    assert_eq!(prompts, 2, "edit + bash prompt; in-root read does not");

    // Every tool finished successfully, and the edit surfaced a file change.
    assert!(
        !has_tool_finished(&events, false),
        "no tool failures expected"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::FileModified { path, .. } if path == "f.txt"
    )));
}

#[tokio::test]
async fn retries_retryable_pre_stream_failure_then_succeeds() {
    // First completion fails to start with a retryable connect error; the
    // engine retries and the second attempt streams a reply (S-3).
    let scripts = vec![
        ScriptedResponse::connect_error(ProviderError::Connect("reset".into())),
        ScriptedResponse::text("recovered after retry"),
    ];
    let mut h = start(scripts, temp_project());
    h.send(Command::UserInput { text: "go".into() }).await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(e, UiEvent::Retrying { .. })),
        "the retry must be surfaced, never silent"
    );
    assert_eq!(deltas(&events), "recovered after retry");
}

#[tokio::test]
async fn non_retryable_failure_is_not_retried() {
    let scripts = vec![ScriptedResponse::connect_error(ProviderError::Auth)];
    let mut h = start(scripts, temp_project());
    h.send(Command::UserInput { text: "go".into() }).await;
    let events = h.collect(None).await;

    assert!(!events.iter().any(|e| matches!(e, UiEvent::Retrying { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::HarnessError { .. })));
}

#[tokio::test]
async fn mid_stream_drop_retries_the_whole_turn() {
    // The first turn streams partial text then the connection drops; the
    // engine retries the whole turn (partial not stitched, Tech Spec §4.3).
    let scripts = vec![
        ScriptedResponse::drop_after(vec![StreamEvent::TextDelta {
            text: "partial…".into(),
        }]),
        ScriptedResponse::text("recovered"),
    ];
    let mut h = start(scripts, temp_project());
    h.send(Command::UserInput { text: "go".into() }).await;
    let events = h.collect(None).await;

    assert!(events.iter().any(|e| matches!(e, UiEvent::Retrying { .. })));
    // The partial was shown live; the recovered turn also streamed.
    assert!(deltas(&events).contains("recovered"));
}

#[tokio::test]
async fn cost_and_context_use_authoritative_usage() {
    // A priced model + a scripted Usage event → the engine reports the exact
    // context size and a cost estimate (P-6).
    let info = ModelInfo {
        model: "m".into(),
        context_window: 1_000,
        max_output_tokens: 100,
        pricing: Some(Pricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        }),
        effort_levels: Vec::new(),
        default_effort: None,
    };
    let response = ScriptedResponse {
        events: vec![
            StreamEvent::TextDelta { text: "hi".into() },
            StreamEvent::Usage {
                usage: TokenUsage {
                    input: 400,
                    output: 500,
                },
            },
        ],
        outcome: ScriptOutcome::Done(StopReason::EndTurn),
    };
    let provider: Arc<dyn Provider> = Arc::new(FakeProvider::new([response]).with_model_info(info));

    let mut h = start_with_provider(provider, temp_project());
    h.send(Command::UserInput { text: "hi".into() }).await;
    let events = h.collect(None).await;

    // Context uses the authoritative prompt-token count (400), not an estimate.
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::ContextUsage { tokens: 400, .. })));
    // Cost = 400/1e6*3 + 500/1e6*15 = 0.0012 + 0.0075 = 0.0087.
    let cost = events.iter().find_map(|e| match e {
        UiEvent::CostEstimate { usd, usage } if usage.input == 400 && usage.output == 500 => {
            Some(*usd)
        }
        _ => None,
    });
    match cost {
        Some(usd) => assert!(
            (usd - 0.0087).abs() < 1e-9,
            "unexpected cost estimate: {usd}"
        ),
        None => panic!("expected a CostEstimate event with the accumulated usage"),
    }
}

#[tokio::test]
async fn usage_chunk_after_done_still_counts() {
    // Regression: OpenAI-compatible servers (Ollama, real OpenAI with
    // include_usage) send the `usage` chunk *after* the finish_reason chunk —
    // i.e. `Usage` arrives after `Done`. The loop must keep draining past
    // `Done` or the session token counter (and cost) stay stuck at 0.
    let info = ModelInfo {
        model: "m".into(),
        context_window: 1_000,
        max_output_tokens: 100,
        pricing: Some(Pricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        }),
        effort_levels: Vec::new(),
        default_effort: None,
    };
    // `drop_after` appends no terminal event, so this is exactly the wire
    // order: content delta → finish_reason (Done) → usage chunk → EOF.
    let response = ScriptedResponse::drop_after(vec![
        StreamEvent::TextDelta { text: "hi".into() },
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
        StreamEvent::Usage {
            usage: TokenUsage {
                input: 400,
                output: 500,
            },
        },
    ]);
    let provider: Arc<dyn Provider> = Arc::new(FakeProvider::new([response]).with_model_info(info));

    let mut h = start_with_provider(provider, temp_project());
    h.send(Command::UserInput { text: "hi".into() }).await;
    let events = h.collect(None).await;

    // The pricing-independent token counter must reflect the post-Done usage.
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::SessionUsage { usage } if usage.input == 400 && usage.output == 500
        )),
        "a usage chunk arriving after Done must still update the session token counter"
    );
    // And the cost estimate, which is derived from the same usage.
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::CostEstimate { usage, .. } if usage.input == 400 && usage.output == 500
        )),
        "cost must reflect usage that arrived after Done"
    );
}

#[tokio::test]
async fn no_cost_estimate_without_pricing() {
    // The default fake model has no pricing → no CostEstimate emitted.
    let mut h = start(vec![ScriptedResponse::text("hello")], temp_project());
    h.send(Command::UserInput { text: "hi".into() }).await;
    let events = h.collect(None).await;
    assert!(!events
        .iter()
        .any(|e| matches!(e, UiEvent::CostEstimate { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::ContextUsage { .. })));
}

#[tokio::test]
async fn cancel_during_bash_stops_promptly() {
    let scripts = vec![ScriptedResponse::tool_call(
        "c1",
        "bash",
        r#"{"command":"sleep 5"}"#,
    )];
    let mut h = start(scripts, temp_project());
    h.send(Command::UserInput { text: "run".into() }).await;

    let started = Instant::now();
    let mut events = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(500), h.events_rx.recv()).await
    {
        match &event {
            UiEvent::PermissionRequest { id, .. } => {
                h.send(Command::PermissionAnswer {
                    id: *id,
                    decision: PermissionDecision::AllowOnce,
                })
                .await;
            }
            UiEvent::ToolStarted { .. } => {
                h.send(Command::Cancel).await;
            }
            _ => {}
        }
        events.push(event);
    }

    assert!(
        started.elapsed() < Duration::from_secs(3),
        "cancel should kill the sleep well before its 5s timeout"
    );
    assert!(
        has_tool_finished(&events, false),
        "canceled tool finishes as failure"
    );
}

/// `/new` mid-session: the current transcript is closed cleanly and a fresh one
/// begins, with the shared session-path handle following the switch (HC-3).
#[tokio::test]
async fn new_session_ends_current_and_starts_fresh() {
    use std::sync::RwLock;

    let root = temp_project();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let sessions_dir =
        std::env::temp_dir().join(format!("emberly-switch-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&sessions_dir);

    let first_id = SessionId::new();
    let first_path = sessions_dir.join(format!("{first_id}.jsonl"));
    let shared = Arc::new(RwLock::new(first_path.clone()));
    let transcript = match emberly_core::FileTranscript::create(&sessions_dir, first_id) {
        Ok(file) => Box::new(file) as Box<dyn TranscriptSink>,
        Err(e) => panic!("create first transcript: {e}"),
    };

    let mut config = make_config(
        Arc::new(FakeProvider::new(vec![ScriptedResponse::text(
            "first-turn",
        )])),
        root,
        transcript,
    );
    config.session_id = first_id;
    config.sessions_dir = sessions_dir.clone();
    config.active_session_path = shared.clone();

    let mut h = spawn(config);
    h.send(Command::UserInput {
        text: "hello".into(),
    })
    .await;
    let _ = h.collect(None).await;

    // Switch to a brand-new session.
    let second_id = SessionId::new();
    h.send(Command::NewSession {
        session_id: second_id,
    })
    .await;
    let _ = h.collect(None).await;

    // Close the engine so the new session's session_end is written.
    drop(h);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The shared handle now names the new session (panic/exit path follows it).
    let current = match shared.read() {
        Ok(guard) => guard.clone(),
        Err(e) => panic!("read shared path: {e}"),
    };
    let second_path = sessions_dir.join(format!("{second_id}.jsonl"));
    assert_eq!(current, second_path, "shared path follows the switch");

    // The first transcript ended cleanly (its last record is session_end).
    let first = match emberly_core::resume::read_records(&first_path) {
        Ok(loaded) => loaded.records,
        Err(e) => panic!("read first: {e}"),
    };
    assert!(
        matches!(
            first.last().map(|r| &r.event),
            Some(TranscriptEvent::SessionEnd { .. })
        ),
        "first session ended cleanly on switch"
    );
    assert!(!emberly_core::resume::interrupted(&first));

    // The second transcript is its own session: a session_start with the new id.
    let second = match emberly_core::resume::read_records(&second_path) {
        Ok(loaded) => loaded.records,
        Err(e) => panic!("read second: {e}"),
    };
    assert_eq!(emberly_core::resume::session_id(&second), Some(second_id));
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

// ---- Phase 2: rule engine + modes wired through the gate ------------------

#[tokio::test]
async fn allowlisted_bash_runs_without_a_prompt_when_confined() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "bash", r#"{"command":"echo hi"}"#),
        ScriptedResponse::text("done"),
    ];
    let mut h = spawn_confined(scripts, root, RuleEngine::new(Vec::new(), true));
    h.send(Command::UserInput { text: "run".into() }).await;
    // No answer supplied: an allowlisted command must not raise a prompt.
    let events = h.collect(None).await;

    assert_eq!(prompt_count(&events), 0, "echo is on the allowlist");
    assert!(has_tool_finished(&events, true));
    assert_eq!(deltas(&events), "done");
}

#[tokio::test]
async fn offlist_bash_still_prompts_when_confined() {
    let root = temp_project();
    // `true` is off the allowlist but always exits 0, so we can assert success.
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "bash", r#"{"command":"true"}"#),
        ScriptedResponse::text("done"),
    ];
    let mut h = spawn_confined(scripts, root, RuleEngine::new(Vec::new(), true));
    h.send(Command::UserInput { text: "run".into() }).await;
    let events = h.collect(Some(PermissionDecision::AllowOnce)).await;

    assert_eq!(prompt_count(&events), 1, "`true` is off the allowlist");
    assert!(has_tool_finished(&events, true));
}

#[tokio::test]
async fn a_deny_rule_auto_denies_without_prompting() {
    let root = temp_project();
    let deny = emberly_core::parse_rules(
        "[[rule]]\ntool = \"bash\"\naction = \"deny\"\n",
        RuleSource::Project,
    )
    .unwrap_or_else(|e| panic!("parse deny rule: {e}"));
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "bash", r#"{"command":"rm -rf /"}"#),
        ScriptedResponse::text("understood"),
    ];
    let mut h = spawn_confined(scripts, root, RuleEngine::new(deny, true));
    h.send(Command::UserInput {
        text: "clean".into(),
    })
    .await;
    let events = h.collect(None).await;

    assert_eq!(prompt_count(&events), 0, "a deny rule never prompts");
    // Denial reaches the model as a structured failure (HC-6), not a crash.
    assert!(has_tool_finished(&events, false));
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::Notice { message } if message.contains("auto-denied")
    )));
    assert_eq!(deltas(&events), "understood");
}

#[tokio::test]
async fn auto_mode_is_refused_when_sandbox_is_degraded() {
    // Default harness runs the degraded (Unavailable) path.
    let mut h = start(vec![ScriptedResponse::text("ok")], temp_project());
    h.send(Command::SetMode { mode: Mode::Auto }).await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::Notice { message } if message.contains("auto-accept modes need OS confinement")
        )),
        "the refusal is explained"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::ModeChanged { .. })),
        "the mode never changes while degraded"
    );
}

#[tokio::test]
async fn auto_accept_edits_skips_the_write_prompt_when_confined() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "write_file", r#"{"path":"out.txt","content":"hi\n"}"#),
        ScriptedResponse::text("done"),
    ];
    let mut h = spawn_confined(scripts, root.clone(), RuleEngine::new(Vec::new(), true));
    h.send(Command::SetMode {
        mode: Mode::AutoAcceptEdits,
    })
    .await;
    h.send(Command::UserInput {
        text: "write it".into(),
    })
    .await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::ModeChanged {
                mode: Mode::AutoAcceptEdits
            }
        )),
        "the mode change is confirmed"
    );
    assert_eq!(prompt_count(&events), 0, "edits auto-allow in this mode");
    assert!(has_tool_finished(&events, true));
    assert_eq!(
        std::fs::read_to_string(root.join("out.txt")).unwrap_or_default(),
        "hi\n"
    );
}

#[tokio::test]
async fn auto_mode_runs_offlist_bash_without_prompting() {
    let root = temp_project();
    // `true` is off the allowlist; in Auto it must still run silently, because
    // the kernel sandbox (required for Auto) enforces the hard lines.
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "bash", r#"{"command":"true"}"#),
        ScriptedResponse::text("done"),
    ];
    let mut h = spawn_confined(scripts, root, RuleEngine::new(Vec::new(), true));
    h.send(Command::SetMode { mode: Mode::Auto }).await;
    h.send(Command::UserInput { text: "run".into() }).await;
    let events = h.collect(None).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::ModeChanged { mode: Mode::Auto })),
        "Auto is entered (confinement is active)"
    );
    assert_eq!(prompt_count(&events), 0, "Auto auto-runs off-list bash");
    assert!(has_tool_finished(&events, true));
    assert_eq!(deltas(&events), "done");
}

#[tokio::test]
async fn allow_for_session_covers_the_next_identical_command() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "bash", r#"{"command":"make build"}"#),
        ScriptedResponse::tool_call("c2", "bash", r#"{"command":"make build"}"#),
        ScriptedResponse::text("done"),
    ];
    let mut h = spawn_confined(scripts, root, RuleEngine::new(Vec::new(), true));
    h.send(Command::UserInput {
        text: "build".into(),
    })
    .await;
    // Auto-answer any prompt with a session grant; the second call should not
    // raise one because the grant already covers it.
    let events = h.collect(Some(PermissionDecision::AllowForSession)).await;

    assert_eq!(
        prompt_count(&events),
        1,
        "only the first `make build` prompts; the session grant covers the second"
    );
    assert_eq!(deltas(&events), "done");
}

/// A fake [`ProviderFactory`](emberly_core::ProviderFactory): every profile
/// except `"unknown"` builds successfully (C-6 switch tests).
struct FakeFactory;

impl emberly_core::ProviderFactory for FakeFactory {
    fn build(&self, profile: &str, model: &str) -> Result<emberly_core::ProviderChoice, String> {
        if profile == "unknown" {
            return Err("unknown provider profile 'unknown'".to_string());
        }
        Ok(emberly_core::ProviderChoice {
            provider: Arc::new(FakeProvider::new(Vec::new())),
            profile: profile.to_string(),
            model: model.to_string(),
        })
    }

    fn profiles(&self) -> Vec<String> {
        vec!["zai".to_string()]
    }
}

/// `SwitchModel` swaps the active provider/model, emits `ModelChanged` (never
/// silent — a `Notice` too), and records a `ModelSwitch` audit event (C-6).
#[tokio::test]
async fn switch_model_swaps_emits_and_records() {
    let sink = CaptureSink::new();
    let mut config = make_config(
        Arc::new(FakeProvider::new(Vec::new())),
        temp_project(),
        Box::new(sink.clone()),
    );
    config.provider_factory = Some(Arc::new(FakeFactory));
    let mut h = spawn(config);
    let _ = h.collect(None).await; // drain session_start / sandbox events

    h.send(Command::SwitchModel {
        profile: "zai".into(),
        model: Some("glm-4.6".into()),
    })
    .await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(e,
            UiEvent::ModelChanged { provider, model } if provider == "zai" && model == "glm-4.6")),
        "a ModelChanged is emitted"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Notice { message } if message.contains("switched to"))),
        "the switch is announced, never silent"
    );
    assert!(
        sink.records().iter().any(|r| matches!(&r.event,
            TranscriptEvent::ModelSwitch { provider, model } if provider == "zai" && model == "glm-4.6")),
        "a ModelSwitch audit record is written (HC-7)"
    );
}

/// An unknown profile is a harness-world error; the current model stays active.
#[tokio::test]
async fn switch_model_unknown_profile_errors_without_switching() {
    let mut config = make_config(
        Arc::new(FakeProvider::new(Vec::new())),
        temp_project(),
        EngineConfig::no_transcript(),
    );
    config.provider_factory = Some(Arc::new(FakeFactory));
    let mut h = spawn(config);
    let _ = h.collect(None).await;

    h.send(Command::SwitchModel {
        profile: "unknown".into(),
        model: None,
    })
    .await;
    let events = h.collect(None).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::HarnessError { .. })),
        "unknown profile surfaces a harness error"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, UiEvent::ModelChanged { .. })),
        "no ModelChanged on a failed switch"
    );
}

/// A fake [`ConfigReloader`](emberly_core::ConfigReloader): reports a changed
/// system prompt, a new profile set, and a restart-only change (C-5).
struct FakeReloader;

impl emberly_core::ConfigReloader for FakeReloader {
    fn reload(&self) -> Result<emberly_core::ReloadedConfig, String> {
        Ok(emberly_core::ReloadedConfig {
            system: Some("new system prompt".to_string()),
            summary_prompt: None,
            provider_factory: Arc::new(FakeFactory),
            profiles: vec!["new".to_string(), "zai".to_string()],
            restart_notes: vec!["sandbox.require changed — restart to apply".to_string()],
        })
    }
}

/// `ReloadConfig` applies the live pieces and reports what changed + what needs
/// a restart; a changed profile set emits `ProfilesChanged` for the picker.
#[tokio::test]
async fn reload_config_applies_and_reports() {
    let mut config = make_config(
        Arc::new(FakeProvider::new(Vec::new())),
        temp_project(),
        EngineConfig::no_transcript(),
    );
    config.config_reloader = Some(Arc::new(FakeReloader));
    let mut h = spawn(config);
    let _ = h.collect(None).await;

    h.send(Command::ReloadConfig).await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(e,
            UiEvent::ProfilesChanged { profiles }
                if profiles == &vec!["new".to_string(), "zai".to_string()])),
        "the picker's profile set is refreshed"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Notice { message }
            if message.contains("reloaded")
                && message.contains("system prompt")
                && message.contains("restart"))),
        "the notice reports live changes and the restart-only one"
    );
}

/// Without a reloader (offline placeholder), `ReloadConfig` is a calm notice.
#[tokio::test]
async fn reload_config_without_reloader_is_a_notice() {
    let mut h = spawn(make_config(
        Arc::new(FakeProvider::new(Vec::new())),
        temp_project(),
        EngineConfig::no_transcript(),
    ));
    let _ = h.collect(None).await;

    h.send(Command::ReloadConfig).await;
    let events = h.collect(None).await;

    assert!(events.iter().any(|e| matches!(e,
        UiEvent::Notice { message } if message.contains("not available"))));
}

/// A factory whose built provider declares a different default effort, so a
/// `SwitchModel` re-seeds the session effort to the new model's default (P-9).
struct ReseedFactory;

impl emberly_core::ProviderFactory for ReseedFactory {
    fn build(&self, profile: &str, model: &str) -> Result<emberly_core::ProviderChoice, String> {
        let info = ModelInfo {
            model: model.to_string(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            pricing: None,
            effort_levels: Effort::ALL.to_vec(),
            default_effort: Some(Effort::High),
        };
        Ok(emberly_core::ProviderChoice {
            provider: Arc::new(FakeProvider::new(Vec::new()).with_model_info(info)),
            profile: profile.to_string(),
            model: model.to_string(),
        })
    }

    fn profiles(&self) -> Vec<String> {
        vec!["other".to_string()]
    }
}

/// `SetEffort` threads the level into the next turn's request (P-9), announces
/// the change (never silent), and records an `EffortChange` audit event (HC-7).
#[tokio::test]
async fn effort_threads_into_request_and_is_announced() {
    let fake = Arc::new(FakeProvider::new(vec![
        ScriptedResponse::text("one"),
        ScriptedResponse::text("two"),
    ]));
    let provider: Arc<dyn Provider> = fake.clone();
    let sink = CaptureSink::new();
    let config = make_config(provider, temp_project(), Box::new(sink.clone()));
    let mut h = spawn(config);

    // First turn carries the model's default effort (fake ⇒ Medium).
    h.send(Command::UserInput { text: "hi".into() }).await;
    let _ = h.collect(None).await;
    assert_eq!(fake.last_effort(), Some(Effort::Medium), "seeded default");

    // Switch effort at idle: announced via EffortChanged + a Notice.
    h.send(Command::SetEffort {
        effort: Effort::High,
    })
    .await;
    let events = h.collect(None).await;
    assert!(
        events.iter().any(|e| matches!(e,
            UiEvent::EffortChanged { effort: Some(l), .. } if *l == Effort::High)),
        "EffortChanged is emitted"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Notice { message } if message.contains("effort"))),
        "the change is announced, never silent"
    );

    // The next turn carries the new effort.
    h.send(Command::UserInput {
        text: "again".into(),
    })
    .await;
    let _ = h.collect(None).await;
    assert_eq!(
        fake.last_effort(),
        Some(Effort::High),
        "threaded into request"
    );

    assert!(
        sink.records().iter().any(|r| matches!(&r.event,
            TranscriptEvent::EffortChange { effort } if *effort == Effort::High)),
        "an EffortChange audit record is written (HC-7)"
    );
}

/// A model switch re-seeds the session effort to the new model's default and
/// announces it via `EffortChanged` (P-9).
#[tokio::test]
async fn switch_model_reseeds_effort_to_new_default() {
    let mut config = make_config(
        Arc::new(FakeProvider::new(Vec::new())), // default effort Medium
        temp_project(),
        EngineConfig::no_transcript(),
    );
    config.provider_factory = Some(Arc::new(ReseedFactory));
    let mut h = spawn(config);
    let _ = h.collect(None).await; // drain startup events

    h.send(Command::SwitchModel {
        profile: "other".into(),
        model: Some("m2".into()),
    })
    .await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(e,
            UiEvent::EffortChanged { effort: Some(l), .. } if *l == Effort::High)),
        "the switch re-seeds effort to the new model's default (Medium → High)"
    );
}

/// Reasoning arrives as a distinct `ReasoningDelta` UiEvent and is recorded in
/// the transcript as a distinct field — never merged into the answer (P-10).
#[tokio::test]
async fn reasoning_streams_distinctly_and_records_a_separate_field() {
    let response = ScriptedResponse {
        events: vec![
            StreamEvent::ReasoningDelta {
                text: "thinking…".into(),
            },
            StreamEvent::ReasoningSignature {
                signature: "sig".into(),
                redacted: false,
            },
            StreamEvent::TextDelta {
                text: "the answer".into(),
            },
        ],
        outcome: ScriptOutcome::Done(StopReason::EndTurn),
    };
    let (mut h, sink) = start_capturing(vec![response], temp_project());
    h.send(Command::UserInput { text: "hi".into() }).await;
    let events = h.collect(None).await;

    // The reasoning surfaces as its own event, distinct from the answer deltas.
    assert!(
        events.iter().any(|e| matches!(e,
            UiEvent::ReasoningDelta { text } if text == "thinking…")),
        "a ReasoningDelta UiEvent is emitted"
    );
    assert_eq!(deltas(&events), "the answer", "answer excludes reasoning");

    // The transcript records reasoning as a distinct field (recorded regardless
    // of any view choice — hidden is a view, not a discard).
    assert!(
        sink.records().iter().any(|r| matches!(&r.event,
            TranscriptEvent::AssistantMessage { text, reasoning }
                if text == "the answer" && reasoning.as_deref() == Some("thinking…"))),
        "AssistantMessage carries reasoning distinct from text"
    );
}

/// A turn with no reasoning leaves the transcript `reasoning` field `None`.
#[tokio::test]
async fn turn_without_reasoning_records_no_reasoning() {
    let (mut h, sink) = start_capturing(vec![ScriptedResponse::text("plain")], temp_project());
    h.send(Command::UserInput { text: "hi".into() }).await;
    let _ = h.collect(None).await;
    assert!(
        sink.records().iter().any(|r| matches!(&r.event,
            TranscriptEvent::AssistantMessage { text, reasoning }
                if text == "plain" && reasoning.is_none())),
        "no reasoning ⇒ reasoning field is None"
    );
}

/// End-to-end (P-9 + P-10): with an effort set, a turn that emits reasoning
/// sends the effort on the wire, streams the reasoning distinctly, and records
/// it as a separate transcript field — the Phase 3 exit criterion in one turn.
#[tokio::test]
async fn effort_and_reasoning_round_trip_in_one_turn() {
    let reasoning_turn = ScriptedResponse {
        events: vec![
            StreamEvent::ReasoningDelta {
                text: "weighing options".into(),
            },
            StreamEvent::ReasoningSignature {
                signature: "sig".into(),
                redacted: false,
            },
            StreamEvent::TextDelta {
                text: "final answer".into(),
            },
        ],
        outcome: ScriptOutcome::Done(StopReason::EndTurn),
    };
    let fake = Arc::new(FakeProvider::new(vec![reasoning_turn]));
    let provider: Arc<dyn Provider> = fake.clone();
    let sink = CaptureSink::new();
    let config = make_config(provider, temp_project(), Box::new(sink.clone()));
    let mut h = spawn(config);
    let _ = h.collect(None).await; // startup events (incl. initial EffortChanged)

    h.send(Command::SetEffort {
        effort: Effort::High,
    })
    .await;
    let _ = h.collect(None).await;

    h.send(Command::UserInput {
        text: "decide".into(),
    })
    .await;
    let events = h.collect(None).await;

    // P-9: the effort reached the wire.
    assert_eq!(
        fake.last_effort(),
        Some(Effort::High),
        "effort on the request"
    );
    // P-10: reasoning streamed distinctly from the answer.
    assert!(events.iter().any(|e| matches!(e,
        UiEvent::ReasoningDelta { text } if text == "weighing options")));
    assert_eq!(deltas(&events), "final answer");
    // P-10 + HC-7: recorded as a distinct transcript field.
    assert!(sink.records().iter().any(|r| matches!(&r.event,
        TranscriptEvent::AssistantMessage { text, reasoning }
            if text == "final answer" && reasoning.as_deref() == Some("weighing options"))));
}

/// Setting effort on a model with no reasoning control is a calm no-op notice,
/// never an error and never an announced change (P-9).
#[tokio::test]
async fn set_effort_on_a_model_without_a_control_declines_calmly() {
    // A fake with no declared effort levels.
    let info = ModelInfo {
        model: "plain".into(),
        context_window: 100,
        max_output_tokens: 100,
        pricing: None,
        effort_levels: Vec::new(),
        default_effort: None,
    };
    let provider: Arc<dyn Provider> = Arc::new(FakeProvider::new(Vec::new()).with_model_info(info));
    let sink = CaptureSink::new();
    let config = make_config(provider, temp_project(), Box::new(sink.clone()));
    let mut h = spawn(config);
    let _ = h.collect(None).await;

    h.send(Command::SetEffort {
        effort: Effort::High,
    })
    .await;
    let events = h.collect(None).await;

    assert!(
        events.iter().any(|e| matches!(e,
            UiEvent::Notice { message } if message.contains("no reasoning-effort control"))),
        "declined with a calm notice"
    );
    // No transcript EffortChange is written.
    assert!(
        !sink
            .records()
            .iter()
            .any(|r| matches!(&r.event, TranscriptEvent::EffortChange { .. })),
        "no audit record for a declined change"
    );
}

// --- T-9: tool-call explanation (Tech Spec §5.4) ------------------------------

/// Build a harness from a retained `FakeProvider` handle with the T-9 toggle
/// set, so a test can drive a turn and then inspect the request the engine sent.
fn spawn_keeping_provider(
    fake: Arc<FakeProvider>,
    root: PathBuf,
    tool_explanations: bool,
) -> Harness {
    let mut config = make_config(fake, root, EngineConfig::no_transcript());
    config.tool_explanations = tool_explanations;
    spawn(config)
}

#[tokio::test]
async fn explanations_on_inject_schema_property_and_prompt_instruction() {
    let fake = Arc::new(FakeProvider::new(vec![ScriptedResponse::text("hi")]));
    let mut h = spawn_keeping_provider(fake.clone(), temp_project(), true);
    h.send(Command::UserInput {
        text: "hello".into(),
    })
    .await;
    let _ = h.collect(None).await;

    let req = match fake.last_request() {
        Some(r) => r,
        None => panic!("no request captured"),
    };
    // Every advertised tool gained the optional `explanation` property.
    assert!(!req.tools.is_empty(), "built-ins are advertised");
    assert!(
        req.tools.iter().all(|t| t
            .input_schema
            .get("properties")
            .and_then(|p| p.get("explanation"))
            .is_some()),
        "explanation injected into every tool schema"
    );
    // The instruction is appended to the outgoing system prompt.
    let system = req.system.unwrap_or_default();
    assert!(
        system.contains("Tool-call explanations"),
        "prompt instructs the model to explain"
    );
}

#[tokio::test]
async fn explanations_off_omit_property_and_instruction() {
    let fake = Arc::new(FakeProvider::new(vec![ScriptedResponse::text("hi")]));
    let mut h = spawn_keeping_provider(fake.clone(), temp_project(), false);
    h.send(Command::UserInput {
        text: "hello".into(),
    })
    .await;
    let _ = h.collect(None).await;

    let req = match fake.last_request() {
        Some(r) => r,
        None => panic!("no request captured"),
    };
    assert!(
        req.tools.iter().all(|t| t
            .input_schema
            .get("properties")
            .and_then(|p| p.get("explanation"))
            .is_none()),
        "no explanation property when the feature is off — no tokens spent"
    );
    // The test harness starts from `system: None`, so off → still no system.
    assert!(
        req.system.unwrap_or_default().is_empty(),
        "no instruction appended when off"
    );
}

#[tokio::test]
async fn tool_started_surfaces_the_models_explanation() {
    let root = temp_project();
    // A non-obvious call the model captioned, then an obvious one it did not.
    let scripts = vec![
        ScriptedResponse::tool_call(
            "c1",
            "read_file",
            r#"{"path":"a.txt","explanation":"peek at the config"}"#,
        ),
        ScriptedResponse::tool_call("c2", "read_file", r#"{"path":"b.txt"}"#),
        ScriptedResponse::text("done"),
    ];
    let mut h = start(scripts, root);
    h.send(Command::UserInput {
        text: "look".into(),
    })
    .await;
    let events = h.collect(None).await;

    let explanations: Vec<Option<String>> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::ToolStarted { explanation, .. } => Some(explanation.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        explanations,
        vec![Some("peek at the config".to_string()), None],
        "captioned call carries the explanation; the obvious one carries none"
    );
}

// --- T-8: ask_user round trip (Tech Spec §5.2) --------------------------------

/// Drive a turn to completion, answering the first `AskUserRequest` with
/// `answer`. Returns the collected events (the caller asserts on them and on
/// the transcript).
async fn drive_answering_ask(h: &mut Harness, answer: AskAnswer) -> Vec<UiEvent> {
    let mut events = Vec::new();
    let mut answered: Option<AskAnswer> = Some(answer);
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(500), h.events_rx.recv()).await
    {
        if let UiEvent::AskUserRequest { id, .. } = &event {
            if let Some(answer) = answered.take() {
                h.send(Command::AskUserAnswer { id: *id, answer }).await;
            }
        }
        events.push(event);
    }
    events
}

#[tokio::test]
async fn ask_user_blocks_then_resumes_with_the_answer() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call(
            "c1",
            "ask_user",
            r#"{"question":"which environment?","options":["dev","prod"]}"#,
        ),
        ScriptedResponse::text("deploying to dev"),
    ];
    let (mut h, sink) = start_capturing(scripts, root);
    h.send(Command::UserInput {
        text: "deploy".into(),
    })
    .await;
    let events = drive_answering_ask(&mut h, AskAnswer::Answered("dev".into())).await;

    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::AskUserRequest { question, options, .. }
            if question == "which environment?"
                && options == &["dev".to_string(), "prod".to_string()]
    )));
    assert_eq!(deltas(&events), "deploying to dev");
    assert!(
        sink.records().iter().any(|r| matches!(
            &r.event,
            TranscriptEvent::ToolResult { output, ok, .. }
                if *ok && output.contains("The user answered: dev")
        )),
        "answer returned to the model as tool-result data"
    );
    assert!(sink.records().iter().any(|r| matches!(
        &r.event,
        TranscriptEvent::AskUser { question, answer: Some(a), .. }
            if question == "which environment?" && a == "dev"
    )));
}

#[tokio::test]
async fn ask_user_dismissed_returns_a_structured_decline() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call("c1", "ask_user", r#"{"question":"proceed?"}"#),
        ScriptedResponse::text("stopping, as you didn't say"),
    ];
    let (mut h, sink) = start_capturing(scripts, root);
    h.send(Command::UserInput { text: "go".into() }).await;
    let events = drive_answering_ask(&mut h, AskAnswer::Declined).await;

    assert_eq!(deltas(&events), "stopping, as you didn't say");
    assert!(
        sink.records().iter().any(|r| matches!(
            &r.event,
            TranscriptEvent::ToolResult { output, ok, .. }
                if *ok && output.contains("declined to answer")
        )),
        "decline returned to the model as data"
    );
    assert!(sink.records().iter().any(|r| matches!(
        &r.event,
        TranscriptEvent::AskUser { answer: None, question, .. } if question == "proceed?"
    )));
}

/// The plan's combined "Done when" in one offline session (Tech Spec §14.5):
/// with explanations on, a captioned call surfaces its explanation, an
/// `ask_user` call blocks and resumes with the answer, and the request the
/// engine sent carried both the schema property and the prompt instruction.
#[tokio::test]
async fn explanation_and_ask_user_together() {
    let root = temp_project();
    let scripts = vec![
        ScriptedResponse::tool_call(
            "c1",
            "read_file",
            r#"{"path":"cfg.txt","explanation":"peek at the config"}"#,
        ),
        ScriptedResponse::tool_call(
            "c2",
            "ask_user",
            r#"{"question":"continue?","options":["yes","no"]}"#,
        ),
        ScriptedResponse::text("done"),
    ];
    let fake = Arc::new(FakeProvider::new(scripts));
    let mut h = spawn_keeping_provider(fake.clone(), root, true);
    h.send(Command::UserInput { text: "go".into() }).await;
    let events = drive_answering_ask(&mut h, AskAnswer::Answered("yes".into())).await;

    // T-9: the captioned call surfaced its explanation.
    assert!(events.iter().any(|e| matches!(
        e,
        UiEvent::ToolStarted { explanation: Some(x), .. } if x == "peek at the config"
    )));
    // T-8: the question was asked and the loop resumed with the answer.
    assert!(events
        .iter()
        .any(|e| matches!(e, UiEvent::AskUserRequest { .. })));
    assert_eq!(deltas(&events), "done");

    // T-9: the request advertised the schema property and the instruction.
    let req = match fake.last_request() {
        Some(r) => r,
        None => panic!("no request captured"),
    };
    assert!(req.tools.iter().all(|t| t
        .input_schema
        .get("properties")
        .and_then(|p| p.get("explanation"))
        .is_some()));
    assert!(req
        .system
        .unwrap_or_default()
        .contains("Tool-call explanations"));
}

// --- FR-1: trust decision recorded at session start ---------------------------

#[tokio::test]
async fn newly_granted_trust_is_recorded_once() {
    let root = temp_project();
    let sink = CaptureSink::new();
    let mut config = make_config(
        Arc::new(FakeProvider::new(vec![ScriptedResponse::text("hi")])),
        root,
        Box::new(sink.clone()),
    );
    config.trust_granted = true;
    let mut h = spawn(config);
    h.send(Command::UserInput { text: "go".into() }).await;
    let _ = h.collect(None).await;

    let trust_records = sink
        .records()
        .iter()
        .filter(|r| {
            matches!(
                &r.event,
                TranscriptEvent::TrustDecision { trusted: true, .. }
            )
        })
        .count();
    assert_eq!(trust_records, 1, "trust decision recorded exactly once");
}

#[tokio::test]
async fn already_trusted_session_records_no_trust_decision() {
    let root = temp_project();
    // Default config has trust_granted = false (already-trusted / silent).
    let (mut h, sink) = start_capturing(vec![ScriptedResponse::text("hi")], root);
    h.send(Command::UserInput { text: "go".into() }).await;
    let _ = h.collect(None).await;
    assert!(
        !sink
            .records()
            .iter()
            .any(|r| matches!(&r.event, TranscriptEvent::TrustDecision { .. })),
        "no trust record when trust was not newly granted"
    );
}
