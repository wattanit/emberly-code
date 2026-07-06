//! Agent-loop integration tests (Phase 1, groups 6 + 7). A `FakeProvider`
//! scripts the model side; the test plays the frontend — sending `UserInput`,
//! answering `PermissionRequest`s, and observing `UiEvent`s. Exercises the
//! gate round trip, tool-result feedback, denial-as-data (HC-6), `FileModified`,
//! and cancellation.
//!
//! No `.unwrap()`/`.expect()`: setup `panic!`s with context; the frontend
//! reads events and asserts on them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use emberly_core::{
    channel, CaptureSink, Command, Engine, EngineConfig, PermissionDecision, RetryPolicy,
    SandboxStatus, SessionId, TranscriptEvent, TranscriptSink, UiEvent,
};
use emberly_providers::{
    FakeProvider, ModelInfo, Pricing, Provider, ProviderError, ScriptOutcome, ScriptedResponse,
    StopReason, StreamEvent, TokenUsage,
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
        truncate: TruncateConfig::default(),
        // Fast retries so retry tests don't wait on real backoff.
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        },
        session_id: SessionId::new(),
        provider_label: "fake".into(),
        sandbox: SandboxStatus::Unavailable {
            reason: "test".into(),
        },
        config_provenance: Vec::new(),
        transcript,
        initial_conversation: Vec::new(),
        resuming: false,
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

fn spawn(config: EngineConfig) -> Harness {
    let (engine_ports, frontend) = channel();
    let (engine, asks_rx) = Engine::new(config, engine_ports.events_tx);
    tokio::spawn(engine.run(engine_ports.commands_rx, asks_rx));
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
        .any(|e| matches!(e, TranscriptEvent::AssistantMessage { text } if text == "done")));

    // Closing the command channel ends the session cleanly.
    drop(h);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(matches!(
        sink.records().last().map(|r| r.event.clone()),
        Some(TranscriptEvent::SessionEnd { .. })
    ));
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
