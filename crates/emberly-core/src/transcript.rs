//! [`TranscriptRecord`] / [`TranscriptEvent`] — the durable, append-only
//! record (Requirements HC-7, §8.2; Tech Spec §3.2). One JSON object per line
//! in `.agents/sessions/<session-id>.jsonl`. The transcript is ground truth;
//! the in-context conversation is a derived view over it and never rewrites a
//! line.
//!
//! The record types are serializable in their own right (A-3), independent of
//! the writer: [`TranscriptSink`] and its [`FileTranscript`] implementation
//! (per-event fsync) append them, and resume/replay reads them back.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::id::{PermissionId, SessionId, ToolCallId};
use crate::types::{Effort, Mode, PermissionDecision, PermissionRendering, SandboxStatus};

/// Current transcript schema version. Present on every record from day one so
/// a reader can detect and warn on newer schemas rather than crash (Tech Spec
/// §3.3). v2 added `session_start.prompts_version`.
pub const SCHEMA_VERSION: u32 = 2;

/// One line of the transcript: the schema version, a timestamp, and the event
/// itself flattened alongside them, producing
/// `{"v":1,"ts":"…","type":"…", …}` (Tech Spec §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptRecord {
    /// Schema version (Tech Spec §3.2 `v`).
    pub v: u32,
    /// Event time, RFC 3339 UTC.
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    /// The event, flattened so its `type` tag and fields sit at the top level.
    #[serde(flatten)]
    pub event: TranscriptEvent,
}

impl TranscriptRecord {
    /// Wrap an event with the current schema version and a timestamp.
    #[must_use]
    pub fn new(ts: OffsetDateTime, event: TranscriptEvent) -> Self {
        Self {
            v: SCHEMA_VERSION,
            ts,
            event,
        }
    }
}

/// A durable transcript event. Internally tagged on `type` (snake_case).
///
/// Unlike [`UiEvent`](crate::event::UiEvent), assistant text is stored
/// *complete* (not as deltas): the transcript records what was said, not how
/// it streamed. `#[non_exhaustive]` for forward compatibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TranscriptEvent {
    /// Opens a session: identity and the environment it started in.
    SessionStart {
        session_id: SessionId,
        provider: String,
        model: String,
        project_root: String,
        sandbox: SandboxStatus,
        /// One entry per configuration piece whose source is not the baked-in
        /// default (Requirements C-3). Empty when everything is default.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        config_provenance: Vec<ConfigProvenance>,
        /// The default prompt-set version this session ran under
        /// ([`crate::prompts::VERSION`]). `default`s to 0 for pre-v2 transcripts.
        #[serde(default)]
        prompts_version: u32,
    },

    /// A user message. The first user message of a session is the original
    /// task and is pinned (never compacted) — flagged here so the view
    /// rebuilder can honor that (Requirements §8.3, Tech Spec §7).
    UserMessage {
        text: String,
        #[serde(default, skip_serializing_if = "is_false")]
        original_task: bool,
    },

    /// A complete assistant message (post-stream). `reasoning` holds the
    /// model's thinking trail as a distinct field, never merged into `text`
    /// (P-10, Tech Spec §4.7); recorded even when the view hides it. Additive —
    /// older readers warn-skip it, so no `SCHEMA_VERSION` bump.
    AssistantMessage {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },

    /// A tool invocation the model requested.
    ToolCall {
        call_id: ToolCallId,
        tool: String,
        args: serde_json::Value,
    },

    /// The result handed back to the model. Records whether the result was
    /// truncated at ingestion and, if so, where the full output lives
    /// (Requirements §8.1, Tech Spec §5.3). `truncated` means the recorded
    /// `output` is not the full output — whether by salient reduction (FR-2)
    /// or the size backstop (§8.1); `full_output_ref` has the whole thing.
    /// `ok` marks success vs a structured failure payload (HC-6).
    ToolResult {
        call_id: ToolCallId,
        ok: bool,
        output: String,
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        full_output_ref: Option<String>,
    },

    /// A permission prompt was raised (what was asked).
    PermissionRequest {
        id: PermissionId,
        rendering: PermissionRendering,
    },

    /// A permission prompt was resolved: what was asked, what the user
    /// answered, and what actually ran (Requirements §6.6).
    PermissionDecision {
        id: PermissionId,
        decision: PermissionDecision,
        /// The command/action that actually executed after the decision, if
        /// any (denials execute nothing).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        executed: Option<String>,
    },

    /// The workspace-trust decision for this session's root (FR-1, Tech Spec
    /// §3.2, §6.7). Written when trust was newly granted at startup; a declined
    /// root never starts a session, so only `trusted: true` reaches a transcript.
    /// Additive — older readers warn-skip it, so no `SCHEMA_VERSION` bump.
    TrustDecision { path: String, trusted: bool },

    /// The loop guardrail halted a non-progressing loop (S-5, Tech Spec §3.2,
    /// §7): the reason and the user's chosen resolution (`resume`/`stop`/`steer`,
    /// `None` until resolved). Additive — older readers warn-skip it, no
    /// `SCHEMA_VERSION` bump.
    LoopHalt {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<String>,
    },

    /// One completion-gate check evaluation (S-6, Tech Spec §3.2, §7): its
    /// name, pass/fail, and the reason (HC-7 — every evaluation is recorded,
    /// win or lose). Additive — older readers warn-skip it, no
    /// `SCHEMA_VERSION` bump.
    CompletionCheck {
        name: String,
        passed: bool,
        reason: String,
    },

    /// The completion gate halted after `max_attempts` failed completion
    /// attempts (S-6, Tech Spec §3.2, §7): the failing checks, the attempt
    /// count, and the user's chosen resolution (`resume`/`stop`/`steer`/
    /// `finish`, `None` until resolved). `override_finish` is `true` only when
    /// the user chose `finish` — the gate binds the model's claim of done,
    /// never the user's authority (Design §8.7). Additive — older readers
    /// warn-skip it, no `SCHEMA_VERSION` bump.
    CompletionGateHalt {
        failing: Vec<crate::types::CheckResult>,
        attempts: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        override_finish: bool,
    },

    /// The model asked the user a question and it was resolved (T-8, Tech Spec
    /// §3.2): the question, any options offered, and the user's answer — or
    /// `None` when they declined. Additive — older readers warn-skip it, so no
    /// `SCHEMA_VERSION` bump (like `ModelSwitch`/`EffortChange`).
    AskUser {
        question: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer: Option<String>,
    },

    /// The auto-accept mode changed (Requirements §6.4).
    ModeChange { mode: Mode },

    /// The active provider profile / model was switched in-session (C-6). An
    /// audit record (HC-7); the conversation view is unaffected.
    ModelSwitch { provider: String, model: String },

    /// The reasoning-effort level was changed in-session (C-6/P-9). An audit
    /// record (HC-7); the conversation view is unaffected. Additive — older
    /// readers warn-skip it, so no `SCHEMA_VERSION` bump (like `ModelSwitch`).
    EffortChange { effort: Effort },

    /// A `/compact` occurred: the summary text and the range of view turns it
    /// replaced (Requirements §8.3). The JSONL log itself is untouched.
    Compaction {
        summary: String,
        replaced_from: u32,
        replaced_to: u32,
        /// Whether the user or the FR-4 threshold initiated this compaction
        /// (FR-4, Tech Spec §3.2). Additive — older readers warn-skip it, no
        /// `SCHEMA_VERSION` bump; absent reads as `Manual`.
        #[serde(default)]
        trigger: CompactTrigger,
    },

    /// The session title was set or renamed (Requirements §8.2).
    SessionTitle { title: String },

    /// The model updated its task list (T-11, Tech Spec §3.2, HC-7). The full
    /// list is recorded on every update (replace, not merge). Additive — older
    /// readers warn-skip it, so no `SCHEMA_VERSION` bump (like `AskUser`/
    /// `EffortChange`).
    TaskList { items: Vec<emberly_tools::TaskItem> },

    /// Clean session end.
    SessionEnd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Written by the supervisor on an abnormal exit when possible
    /// (Requirements HC-3, S-2).
    AbnormalExit { reason: String },
}

/// Whether a compaction was started by the user or the automatic threshold
/// (FR-4, Tech Spec §3.2). Serialized as `trigger` on the `Compaction` event;
/// absent on older records reads as `Manual` (additive — no `SCHEMA_VERSION`
/// bump).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    #[default]
    Manual,
    Auto,
}

/// Records which configuration tier an active piece came from, for provenance
/// reporting (Requirements C-3, Tech Spec §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProvenance {
    /// What the piece is, e.g. `system_prompt`, `pricing.model-x`.
    pub piece: String,
    /// Where it came from, e.g. `project:.agents/config.toml`, `default`.
    pub source: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // signature required by serde's skip_serializing_if
fn is_false(b: &bool) -> bool {
    !*b
}

/// A destination for durable transcript records (HC-7). Implementations append
/// one record per call and own durability. Write failures are **swallowed** —
/// transcript I/O is a harness-world concern that must never crash the agent
/// (they are surfaced to the user by the supervisor path, Phase 5 group 2).
pub trait TranscriptSink: Send + Sync {
    /// Append one record, flushing it to durable storage before returning
    /// (per-event fsync so a crash loses almost nothing — Tech Spec §10, S-2).
    fn record(&mut self, record: &TranscriptRecord);

    /// Persist a tool's full, untruncated output to a sidecar and return the
    /// reference to store in `full_output_ref` (Requirements §8.1); `None` if
    /// unsupported or the write failed.
    fn sidecar(&mut self, call_id: &ToolCallId, content: &str) -> Option<String> {
        let _ = (call_id, content);
        None
    }
}

/// Discards everything — the default when no session file is configured, and
/// the sink used by tests that don't assert on the transcript.
pub struct NoopSink;

impl TranscriptSink for NoopSink {
    fn record(&mut self, _record: &TranscriptRecord) {}
}

/// Captures records in memory so tests can assert the exact durable sequence.
#[derive(Clone, Default)]
pub struct CaptureSink {
    records: Arc<Mutex<Vec<TranscriptRecord>>>,
}

impl CaptureSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of the records captured so far.
    #[must_use]
    pub fn records(&self) -> Vec<TranscriptRecord> {
        self.records.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

impl TranscriptSink for CaptureSink {
    fn record(&mut self, record: &TranscriptRecord) {
        if let Ok(mut v) = self.records.lock() {
            v.push(record.clone());
        }
    }
}

/// Append-only, per-event-fsynced JSONL transcript on disk (HC-7, Tech Spec
/// §3.2). One `TranscriptRecord` per line in `<dir>/<session>.jsonl`; truncated
/// tool outputs spill to `<dir>/<session>-outputs/`.
pub struct FileTranscript {
    file: File,
    outputs_dir: PathBuf,
    /// Once a write fails the file is marked unhealthy so we stop retrying every
    /// event (the failure is reported once by the supervisor path).
    healthy: bool,
}

impl FileTranscript {
    /// Create or open `<dir>/<session>.jsonl` for append, creating `<dir>` if
    /// needed. Nothing is written until the first [`record`](Self::record).
    pub fn create(dir: &Path, session: SessionId) -> std::io::Result<Self> {
        Self::open(&dir.join(format!("{session}.jsonl")))
    }

    /// Open an existing (or new) transcript file for append — the resume path
    /// (Tech Spec §3.3), which continues writing to the same session file.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session");
        let outputs_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{stem}-outputs"));
        Ok(Self {
            file,
            outputs_dir,
            healthy: true,
        })
    }

    fn append_line(&mut self, record: &TranscriptRecord) -> std::io::Result<()> {
        let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.file.sync_data()?; // per-event durability (S-2)
        Ok(())
    }
}

impl TranscriptSink for FileTranscript {
    fn record(&mut self, record: &TranscriptRecord) {
        if !self.healthy {
            return;
        }
        if self.append_line(record).is_err() {
            self.healthy = false;
        }
    }

    fn sidecar(&mut self, call_id: &ToolCallId, content: &str) -> Option<String> {
        if fs::create_dir_all(&self.outputs_dir).is_err() {
            return None;
        }
        let path = self
            .outputs_dir
            .join(format!("{}.txt", sanitize(&call_id.0)));
        if fs::write(&path, content).is_err() {
            return None;
        }
        Some(path.display().to_string())
    }
}

/// Append a single record to an existing transcript file by reopening it —
/// the crash-path counterpart to [`FileTranscript`], used by the supervisor's
/// panic hook where the engine (and its live sink) is unreachable. Best-effort
/// and standalone: it needs only the path. Safe because every prior event was
/// already fsynced, so we only ever add one trailing line.
pub fn append_abnormal_exit(path: &Path, reason: &str) {
    let record = TranscriptRecord::new(
        OffsetDateTime::now_utc(),
        TranscriptEvent::AbnormalExit {
            reason: reason.to_string(),
        },
    );
    let _ = append_record(path, &record);
}

fn append_record(path: &Path, record: &TranscriptRecord) -> std::io::Result<()> {
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    let mut file = OpenOptions::new().append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_data()?;
    Ok(())
}

/// Keep sidecar filenames to a safe alphabet (tool-call ids are provider-issued
/// strings like `toolu_1` / `call_1`, but never trust them as path fragments).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_transcript_round_trips() {
        let dir = std::env::temp_dir().join(format!("emberly-tx-{}", std::process::id()));
        let session = SessionId::new();
        let mut sink = match FileTranscript::create(&dir, session) {
            Ok(s) => s,
            Err(e) => panic!("create: {e}"),
        };
        let rec = TranscriptRecord::new(
            OffsetDateTime::UNIX_EPOCH,
            TranscriptEvent::UserMessage {
                text: "hello".into(),
                original_task: true,
            },
        );
        sink.record(&rec);

        let path = dir.join(format!("{session}.jsonl"));
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let parsed: TranscriptRecord = match serde_json::from_str(contents.trim()) {
            Ok(r) => r,
            Err(e) => panic!("parse: {e}"),
        };
        assert_eq!(parsed, rec);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_abnormal_exit_adds_a_trailing_line() {
        let dir = std::env::temp_dir().join(format!("emberly-tx-ax-{}", std::process::id()));
        let session = SessionId::new();
        {
            let mut sink = match FileTranscript::create(&dir, session) {
                Ok(s) => s,
                Err(e) => panic!("create: {e}"),
            };
            sink.record(&TranscriptRecord::new(
                OffsetDateTime::UNIX_EPOCH,
                TranscriptEvent::SessionEnd { reason: None },
            ));
        } // drop the live sink; the panic-path append reopens the file

        let path = dir.join(format!("{session}.jsonl"));
        append_abnormal_exit(&path, "boom");

        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let last = contents.lines().last().unwrap_or_default();
        let record: TranscriptRecord = match serde_json::from_str(last) {
            Ok(r) => r,
            Err(e) => panic!("parse: {e}"),
        };
        assert!(matches!(
            record.event,
            TranscriptEvent::AbnormalExit { reason } if reason == "boom"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidecar_writes_full_output_and_returns_ref() {
        let dir = std::env::temp_dir().join(format!("emberly-tx-sc-{}", std::process::id()));
        let session = SessionId::new();
        let mut sink = match FileTranscript::create(&dir, session) {
            Ok(s) => s,
            Err(e) => panic!("create: {e}"),
        };
        let reference = sink.sidecar(&ToolCallId::new("call_1"), "full output");
        let reference = reference.unwrap_or_default();
        assert!(reference.ends_with("call_1.txt"));
        assert_eq!(
            std::fs::read_to_string(&reference).unwrap_or_default(),
            "full output"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
