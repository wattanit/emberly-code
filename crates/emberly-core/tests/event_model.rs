//! Round-trip serialization coverage for the event model (Phase 1, group 1).
//!
//! Every `UiEvent`, `Command`, and `TranscriptEvent` variant must survive a
//! JSON round-trip unchanged (A-3: serializable-from-day-one is what makes the
//! transcript (HC-7) and replay tests (A-2) cheap). These run in a separate
//! test crate, so they exercise only the public API.
//!
//! No `.unwrap()`/`.expect()` here either — helpers thread `?` and use
//! `assert_eq!`/`panic!` so the whole tree passes the Phase 1 lint gate.

use emberly_core::event::UiEvent;
use emberly_core::transcript::{TranscriptEvent, TranscriptRecord};
use emberly_core::types::{
    Mode, PermissionDecision, PermissionRendering, SandboxStatus, TokenUsage,
};
use emberly_core::{Command, PermissionId, SessionId, ToolCallId};
use serde::de::DeserializeOwned;
use serde::Serialize;
use time::OffsetDateTime;

/// Serialize, deserialize, and assert the value is unchanged.
fn assert_roundtrip<T>(value: &T) -> serde_json::Result<()>
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value)?;
    let back: T = serde_json::from_str(&json)?;
    assert_eq!(value, &back, "round-trip mismatch; json was: {json}");
    Ok(())
}

fn sample_rendering() -> PermissionRendering {
    PermissionRendering {
        tool: "bash".into(),
        summary: "run: cargo test".into(),
        detail: "cargo test --workspace".into(),
        affected_paths: vec!["/proj".into()],
        outside_root: false,
        reason: "matched rule: bash ask".into(),
    }
}

#[test]
fn ui_event_variants_roundtrip() -> serde_json::Result<()> {
    let events = vec![
        UiEvent::AssistantDelta {
            text: "สวัสดี".into(),
        }, // Thai content survives
        UiEvent::AssistantDone,
        UiEvent::ToolStarted {
            call_id: ToolCallId("call_1".into()),
            tool: "read_file".into(),
            summary: "read src/main.rs".into(),
        },
        UiEvent::ToolFinished {
            call_id: ToolCallId("call_1".into()),
            ok: true,
            summary: "42 lines".into(),
            preview: "line one\nline two".into(),
        },
        UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering: sample_rendering(),
        },
        UiEvent::ContextUsage {
            pct: 37,
            tokens: 12_000,
        },
        UiEvent::CostEstimate {
            usage: TokenUsage {
                input: 1000,
                output: 200,
            },
            usd: 0.0123,
        },
        UiEvent::SandboxStatus {
            status: SandboxStatus::Confined {
                backend: "landlock".into(),
            },
        },
        UiEvent::ModeChanged {
            mode: Mode::AutoAcceptEdits,
        },
        UiEvent::HarnessError {
            what: "provider unreachable".into(),
            why: "connection refused".into(),
            next: "retrying in 5s".into(),
        },
        UiEvent::SessionMeta {
            session_id: SessionId::new(),
            title: "wire the event model".into(),
            provider: "anthropic".into(),
            model: "claude-opus-4-8".into(),
            project_root: "~/emberly-code".into(),
        },
        UiEvent::FileModified {
            path: "src/lib.rs".into(),
            adds: 12,
            dels: 3,
        },
        UiEvent::CompactionStatus {
            message: "summarized 8 turns".into(),
        },
    ];
    for e in &events {
        assert_roundtrip(e)?;
    }
    Ok(())
}

#[test]
fn command_variants_roundtrip() -> serde_json::Result<()> {
    let commands = vec![
        Command::UserInput {
            text: "แก้ไขไฟล์นี้".into(),
        },
        Command::PermissionAnswer {
            id: PermissionId(7),
            decision: PermissionDecision::AllowOnce,
        },
        Command::SetMode { mode: Mode::Auto },
        Command::Compact,
        Command::Cancel,
    ];
    for c in &commands {
        assert_roundtrip(c)?;
    }
    Ok(())
}

#[test]
fn transcript_event_variants_roundtrip() -> serde_json::Result<()> {
    let ts = OffsetDateTime::UNIX_EPOCH;
    let events = vec![
        TranscriptEvent::SessionStart {
            session_id: SessionId::new(),
            provider: "openai".into(),
            model: "local/llama".into(),
            project_root: "/proj".into(),
            sandbox: SandboxStatus::Unavailable {
                reason: "no landlock".into(),
            },
            config_provenance: vec![],
        },
        TranscriptEvent::UserMessage {
            text: "task".into(),
            original_task: true,
        },
        TranscriptEvent::AssistantMessage {
            text: "on it".into(),
        },
        TranscriptEvent::ToolCall {
            call_id: ToolCallId("c1".into()),
            tool: "bash".into(),
            args: serde_json::json!({ "command": "ls" }),
        },
        TranscriptEvent::ToolResult {
            call_id: ToolCallId("c1".into()),
            ok: true,
            output: "a\nb".into(),
            truncated: false,
            full_output_ref: None,
        },
        TranscriptEvent::PermissionRequest {
            id: PermissionId(1),
            rendering: sample_rendering(),
        },
        TranscriptEvent::PermissionDecision {
            id: PermissionId(1),
            decision: PermissionDecision::Deny,
            executed: None,
        },
        TranscriptEvent::ModeChange { mode: Mode::Normal },
        TranscriptEvent::Compaction {
            summary: "…".into(),
            replaced_from: 0,
            replaced_to: 8,
        },
        TranscriptEvent::SessionTitle { title: "t".into() },
        TranscriptEvent::SessionEnd { reason: None },
        TranscriptEvent::AbnormalExit {
            reason: "panic in tui".into(),
        },
    ];
    for e in events {
        assert_roundtrip(&TranscriptRecord::new(ts, e))?;
    }
    Ok(())
}

/// The transcript wire shape is `{"v":1,"ts":"…","type":"…", …}` — the schema
/// version and timestamp sit alongside the flattened event tag (Tech Spec
/// §3.2).
#[test]
fn transcript_record_wire_shape() -> serde_json::Result<()> {
    let rec = TranscriptRecord::new(
        OffsetDateTime::UNIX_EPOCH,
        TranscriptEvent::AssistantMessage { text: "hi".into() },
    );
    let value: serde_json::Value = serde_json::to_value(&rec)?;
    assert_eq!(value["v"], serde_json::json!(1));
    assert_eq!(value["type"], serde_json::json!("assistant_message"));
    assert_eq!(value["text"], serde_json::json!("hi"));
    assert_eq!(value["ts"], serde_json::json!("1970-01-01T00:00:00Z"));
    Ok(())
}
