//! Resume: reading a transcript back and rebuilding the conversation view
//! (Requirements §8.2, Tech Spec §3.3). The transcript is ground truth; the
//! in-context conversation is a *derived view* over it. Reading is lenient —
//! a line with an unknown event `type` or a newer schema is **warned and
//! skipped, never a crash** (Tech Spec §3.3) — so a transcript written by a
//! future version still resumes as much as it can.

use std::path::{Path, PathBuf};

use emberly_providers::{ContentBlock, Message, Role};

use crate::id::SessionId;
use crate::transcript::{TranscriptEvent, TranscriptRecord, SCHEMA_VERSION};
use crate::view_cache::{view_cache_path, ViewCache, VIEW_CACHE_VERSION};

/// The result of reading a transcript file.
pub struct Loaded {
    pub records: Vec<TranscriptRecord>,
    /// One human-readable line per skipped record (unknown type / newer schema).
    pub warnings: Vec<String>,
}

/// Read a transcript file line by line, tolerating malformed or future records.
pub fn read_records(path: &Path) -> std::io::Result<Loaded> {
    let text = std::fs::read_to_string(path)?;
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TranscriptRecord>(line) {
            Ok(record) => records.push(record),
            Err(error) => warnings.push(describe_skip(n + 1, line, &error)),
        }
    }
    Ok(Loaded { records, warnings })
}

/// A skipped line: report a newer schema distinctly from a parse error so the
/// message teaches rather than alarms (Tech Spec §3.3).
fn describe_skip(line_no: usize, line: &str, error: &serde_json::Error) -> String {
    let version = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("v").and_then(serde_json::Value::as_u64));
    match version {
        Some(v) if v > u64::from(SCHEMA_VERSION) => {
            format!("line {line_no}: skipped a record from a newer transcript schema (v{v})")
        }
        _ => format!("line {line_no}: skipped an unreadable record ({error})"),
    }
}

/// Rebuild the engine conversation from transcript records (Tech Spec §3.3).
///
/// Assistant text and the tool calls that immediately follow it are folded
/// into one assistant message; tool results become tool messages; a
/// `compaction` splices out its `[replaced_from..replaced_to]` middle and
/// inserts the summary (as a user message), keeping the pinned head and recent
/// tail — the same view `/compact` produced live (Requirements §8.3).
/// Non-conversation records (session_start, permission, title, end) are skipped.
#[must_use]
pub fn rebuild_conversation(records: &[TranscriptRecord]) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::new();
    let mut i = 0;
    while i < records.len() {
        match &records[i].event {
            TranscriptEvent::UserMessage { text, .. } => {
                messages.push(Message::user_text(text.clone()));
            }
            TranscriptEvent::AssistantMessage { text, .. } => {
                // Reasoning is deliberately not reconstructed: the opaque
                // signature is not persisted, and historical thinking blocks may
                // be stripped (only the live turn needs replay — P-10/§4.7). A
                // tool-only-with-thinking turn has empty text and no text block.
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(ContentBlock::Text { text: text.clone() });
                }
                i += gather_tool_calls(&records[i + 1..], &mut content);
                messages.push(Message {
                    role: Role::Assistant,
                    content,
                });
            }
            TranscriptEvent::ToolCall { .. } => {
                // A tool-only turn (assistant produced no text).
                let mut content = Vec::new();
                // Re-read this record plus any following tool calls.
                i += gather_tool_calls(&records[i..], &mut content).saturating_sub(1);
                messages.push(Message {
                    role: Role::Assistant,
                    content,
                });
            }
            TranscriptEvent::ToolResult {
                call_id,
                output,
                ok,
                ..
            } => {
                messages.push(Message::tool_result(call_id.clone(), output.clone(), !*ok));
            }
            TranscriptEvent::Compaction {
                summary,
                replaced_from,
                replaced_to,
                trigger: _,
            } => {
                // Splice out `[from..to]` (the summarized middle) and insert the
                // summary, keeping the pinned head and the recent tail — the
                // same view `/compact` produced live (Tech Spec §7).
                let from = (*replaced_from as usize).min(messages.len());
                let to = (*replaced_to as usize).min(messages.len());
                if from <= to {
                    let tail = messages.split_off(to);
                    messages.truncate(from);
                    messages.push(Message::user_text(summary.clone()));
                    messages.extend(tail);
                }
            }
            // session_start / title / permission / mode / end / abnormal_exit
            // are not part of the model-visible conversation.
            _ => {}
        }
        i += 1;
    }
    messages
}

/// Append `tool_use` blocks for a run of leading `ToolCall` records to
/// `content`; returns how many records were consumed.
fn gather_tool_calls(records: &[TranscriptRecord], content: &mut Vec<ContentBlock>) -> usize {
    let mut consumed = 0;
    for record in records {
        if let TranscriptEvent::ToolCall {
            call_id,
            tool,
            args,
        } = &record.event
        {
            content.push(ContentBlock::ToolUse {
                id: call_id.clone(),
                name: tool.clone(),
                input: args.clone(),
            });
            consumed += 1;
        } else {
            break;
        }
    }
    consumed
}

/// Try loading the derived view cache (FR-5, Tech Spec §3.2a/§3.3). Returns
/// `None` — forcing a fallback replay — on any doubt: the cache file is
/// absent, unreadable, corrupt, the wrong version, has a session-id mismatch,
/// or the transcript has grown or shrunk since the cache was written. Only an
/// exact transcript byte-length match is trusted.
///
/// The guard is pure metadata (`fs::metadata().len()`) plus a version/id
/// check — no re-tokenization, so validating the cache is cheap (FR-5).
#[must_use]
pub fn try_load_view_cache(transcript_path: &Path) -> Option<ViewCache> {
    let cache_path = view_cache_path(transcript_path);
    let text = std::fs::read_to_string(&cache_path).ok()?;
    let cache: ViewCache = serde_json::from_str(&text).ok()?;
    if cache.version != VIEW_CACHE_VERSION {
        return None;
    }
    // Session-id sanity: the transcript filename stem is `<uuid>`. If it
    // parses and does not match the cache's session id, the cache is from a
    // different session. A non-UUID stem (unusual path) skips this check —
    // the byte-length match below is the authoritative guard.
    if let Some(stem) = transcript_path.file_stem().and_then(|s| s.to_str()) {
        if let Ok(id) = uuid::Uuid::parse_str(stem) {
            if id != cache.session_id.0 {
                return None;
            }
        }
    }
    let actual_len = std::fs::metadata(transcript_path).map(|m| m.len()).ok()?;
    if actual_len != cache.transcript_byte_len {
        return None;
    }
    Some(cache)
}

/// Whether a session ended without a clean `session_end` — a crash, a kill, or
/// a recorded `abnormal_exit` — so the next launch should offer to resume it
/// (Design §8.3). An empty or cleanly-ended transcript is not interrupted.
#[must_use]
pub fn interrupted(records: &[TranscriptRecord]) -> bool {
    match records.last().map(|r| &r.event) {
        None => false,
        Some(TranscriptEvent::SessionEnd { .. }) => false,
        Some(_) => true,
    }
}

/// Whether the transcript recorded any compaction, so the engine knows the
/// rebuilt conversation has a pinned compaction summary at `conversation[1]`
/// (FR-3 windowing composition).
#[must_use]
pub fn has_compaction(records: &[TranscriptRecord]) -> bool {
    records
        .iter()
        .any(|r| matches!(r.event, TranscriptEvent::Compaction { .. }))
}

/// The `(provider, model)` a session started with, from its `session_start`
/// (needed to reconstruct the provider on resume).
#[must_use]
pub fn session_meta(records: &[TranscriptRecord]) -> Option<(String, String)> {
    records.iter().find_map(|r| match &r.event {
        TranscriptEvent::SessionStart {
            provider, model, ..
        } => Some((provider.clone(), model.clone())),
        _ => None,
    })
}

/// The session id from `session_start` (identifies the session on resume).
#[must_use]
pub fn session_id(records: &[TranscriptRecord]) -> Option<SessionId> {
    records.iter().find_map(|r| match &r.event {
        TranscriptEvent::SessionStart { session_id, .. } => Some(*session_id),
        _ => None,
    })
}

/// The session title, if one was recorded.
#[must_use]
pub fn session_title(records: &[TranscriptRecord]) -> Option<String> {
    records.iter().rev().find_map(|r| match &r.event {
        TranscriptEvent::SessionTitle { title } => Some(title.clone()),
        _ => None,
    })
}

/// A one-line summary of a session on disk, for `emberly sessions` and the
/// in-app session picker (Design §8.2, §8.3). Derived entirely from the
/// transcript — no separate index to drift out of sync.
pub struct SessionSummary {
    pub id: SessionId,
    pub path: PathBuf,
    /// The session title (from the first user message), or `None` if untitled.
    pub title: Option<String>,
    pub provider: String,
    pub model: String,
    /// File modification time, for recency sorting and display.
    pub modified: std::time::SystemTime,
    /// True if the session ended without a clean `session_end` (resumable).
    pub interrupted: bool,
    /// Number of records read (a rough sense of session size).
    pub events: usize,
}

/// Summarize every `<id>.jsonl` in `dir`, most-recently-modified first. Files
/// that cannot be read at all are skipped (never a crash); malformed *lines*
/// within a file are already tolerated by [`read_records`].
#[must_use]
pub fn list_sessions(dir: &Path) -> Vec<SessionSummary> {
    let mut sessions = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return sessions,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        let Ok(loaded) = read_records(&path) else {
            continue;
        };
        let (provider, model) =
            session_meta(&loaded.records).unwrap_or_else(|| ("unknown".into(), "unknown".into()));
        sessions.push(SessionSummary {
            id: session_id(&loaded.records).unwrap_or_default(),
            title: session_title(&loaded.records),
            provider,
            model,
            modified,
            interrupted: interrupted(&loaded.records),
            events: loaded.records.len(),
            path,
        });
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    sessions
}

/// The most recently modified `<id>.jsonl` in `dir`, for `resume` with no id
/// and the offer-on-launch flow.
#[must_use]
pub fn latest_session(dir: &Path) -> Option<PathBuf> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
        if let Some(modified) = modified {
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ToolCallId;
    use crate::types::TokenUsage;
    use std::io::Write;
    use time::OffsetDateTime;

    fn rec(event: TranscriptEvent) -> TranscriptRecord {
        TranscriptRecord::new(OffsetDateTime::UNIX_EPOCH, event)
    }

    #[test]
    fn rebuilds_a_tool_using_turn() {
        let records = vec![
            rec(TranscriptEvent::UserMessage {
                text: "do it".into(),
                original_task: true,
                images: Vec::new(),
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "on it".into(),
                reasoning: None,
            }),
            rec(TranscriptEvent::ToolCall {
                call_id: ToolCallId::new("c1"),
                tool: "bash".into(),
                args: serde_json::json!({"command": "ls"}),
            }),
            rec(TranscriptEvent::ToolResult {
                call_id: ToolCallId::new("c1"),
                ok: true,
                output: "file.txt".into(),
                truncated: false,
                full_output_ref: None,
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "done".into(),
                reasoning: None,
            }),
        ];
        let messages = rebuild_conversation(&records);
        // user, assistant(text+tool_use), tool_result, assistant.
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content.len(), 2, "text + one tool_use");
        assert_eq!(messages[2].role, Role::Tool);
        assert_eq!(messages[3].role, Role::Assistant);
    }

    #[test]
    fn compaction_collapses_the_middle_and_keeps_the_tail() {
        let records = vec![
            rec(TranscriptEvent::UserMessage {
                text: "task".into(),
                original_task: true,
                images: Vec::new(),
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "a".into(),
                reasoning: None,
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "b".into(),
                reasoning: None,
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "recent".into(),
                reasoning: None,
            }),
            // Keep [0] (task) and [3] (recent); replace [1..3] with the summary.
            rec(TranscriptEvent::Compaction {
                summary: "summary so far".into(),
                replaced_from: 1,
                replaced_to: 3,
                trigger: Default::default(),
            }),
            rec(TranscriptEvent::UserMessage {
                text: "continue".into(),
                original_task: false,
                images: Vec::new(),
            }),
        ];
        let messages = rebuild_conversation(&records);
        // [task][summary][recent][continue]
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::User);
        assert!(
            matches!(&messages[1].content[0], ContentBlock::Text { text } if text == "summary so far")
        );
        assert!(matches!(&messages[2].content[0], ContentBlock::Text { text } if text == "recent"));
        assert!(
            matches!(&messages[3].content[0], ContentBlock::Text { text } if text == "continue")
        );
    }

    #[test]
    fn read_records_warns_and_skips_unreadable_lines() {
        let dir = std::env::temp_dir().join(format!("emberly-resume-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.jsonl");
        // A good record, an unknown type, and a newer-schema record.
        let good = r#"{"v":1,"ts":"1970-01-01T00:00:00Z","type":"user_message","text":"hi"}"#;
        let unknown = r#"{"v":1,"ts":"1970-01-01T00:00:00Z","type":"future_event","x":1}"#;
        let newer = r#"{"v":999,"ts":"1970-01-01T00:00:00Z","type":"even_newer","x":1}"#;
        if let Err(e) = std::fs::write(&path, format!("{good}\n{unknown}\n{newer}\n")) {
            panic!("write: {e}");
        }
        let loaded = match read_records(&path) {
            Ok(l) => l,
            Err(e) => panic!("read: {e}"),
        };
        assert_eq!(loaded.records.len(), 1, "the good record survives");
        assert_eq!(
            loaded.warnings.len(),
            2,
            "both bad lines warned, not crashed"
        );
        assert!(loaded
            .warnings
            .iter()
            .any(|w| w.contains("newer transcript schema")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_sessions_summarizes_newest_first() {
        let dir = std::env::temp_dir().join(format!("emberly-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        // A clean session and an interrupted one; write the clean one first so
        // the interrupted one is newer (list is newest-first).
        let clean = concat!(
            r#"{"v":2,"ts":"1970-01-01T00:00:00Z","type":"session_start","session_id":"00000000-0000-0000-0000-000000000001","provider":"anthropic","model":"claude","project_root":"/p","sandbox":{"state":"unavailable","reason":"x"},"config_provenance":[],"prompts_version":1}"#,
            "\n",
            r#"{"v":2,"ts":"1970-01-01T00:00:01Z","type":"session_title","title":"clean one"}"#,
            "\n",
            r#"{"v":2,"ts":"1970-01-01T00:00:02Z","type":"session_end"}"#,
        );
        let crashed = concat!(
            r#"{"v":2,"ts":"1970-01-01T00:00:00Z","type":"session_start","session_id":"00000000-0000-0000-0000-000000000002","provider":"openai","model":"gpt","project_root":"/p","sandbox":{"state":"unavailable","reason":"x"},"config_provenance":[],"prompts_version":1}"#,
            "\n",
            r#"{"v":2,"ts":"1970-01-01T00:00:01Z","type":"session_title","title":"crashed one"}"#,
        );
        if let Err(e) = std::fs::write(dir.join("a.jsonl"), clean) {
            panic!("write clean: {e}");
        }
        if let Err(e) = std::fs::write(dir.join("b.jsonl"), crashed) {
            panic!("write crashed: {e}");
        }
        let sessions = list_sessions(&dir);
        assert_eq!(sessions.len(), 2);
        // Both parsed their metadata.
        assert!(sessions
            .iter()
            .any(|s| s.title.as_deref() == Some("clean one") && !s.interrupted));
        let crashed = sessions
            .iter()
            .find(|s| s.title.as_deref() == Some("crashed one"));
        match crashed {
            Some(s) => {
                assert!(s.interrupted, "no session_end → interrupted");
                assert_eq!(s.provider, "openai");
            }
            None => panic!("crashed session missing from list"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_sessions_on_missing_dir_is_empty_not_error() {
        let missing = std::env::temp_dir().join("emberly-nope-does-not-exist-xyz");
        assert!(list_sessions(&missing).is_empty());
    }

    #[test]
    fn interrupted_unless_cleanly_ended() {
        let mid = vec![rec(TranscriptEvent::UserMessage {
            text: "x".into(),
            original_task: true,
            images: Vec::new(),
        })];
        assert!(interrupted(&mid));

        let mut clean = mid.clone();
        clean.push(rec(TranscriptEvent::SessionEnd { reason: None }));
        assert!(!interrupted(&clean));

        let mut crashed = mid.clone();
        crashed.push(rec(TranscriptEvent::AbnormalExit {
            reason: "boom".into(),
        }));
        assert!(
            interrupted(&crashed),
            "a recorded crash still offers resume"
        );

        assert!(!interrupted(&[]));
    }

    // --- Staleness guard tests (FR-5, group 3) ---

    /// Write a transcript and a matching `-view.json` cache, return both paths.
    fn write_cache_fixture(
        sid: SessionId,
        transcript_len_delta: i64,
        cache_overrides: Option<ViewCache>,
    ) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("emberly-cache-{}-{sid}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let transcript_path = dir.join(format!("{sid}.jsonl"));
        // Write a transcript body of a known length.
        let body = r#"{"v":2,"ts":"1970-01-01T00:00:00Z","type":"session_start","session_id":"00000000-0000-0000-0000-000000000000","provider":"test","model":"test","project_root":"/p","sandbox":{"state":"unavailable","reason":"x"},"config_provenance":[],"prompts_version":1}
"#;
        let _ = std::fs::write(&transcript_path, body);
        let real_len = std::fs::metadata(&transcript_path)
            .map(|m| m.len())
            .unwrap_or(0);
        // Adjust the transcript to create the desired delta (grown / shorter / exact).
        if transcript_len_delta > 0 {
            // Append bytes to grow the file past the recorded length.
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .append(true)
                .open(&transcript_path)
            {
                let _ = f.write_all(b"x".repeat(transcript_len_delta as usize).as_slice());
            }
        }
        // Build the cache claiming the *original* byte length (before any delta).
        let cache = cache_overrides.unwrap_or(ViewCache {
            version: VIEW_CACHE_VERSION,
            session_id: sid,
            conversation: vec![Message::user_text("hello")],
            turn_map: vec![0],
            next_turn: 1,
            compacted: false,
            original_task_recorded: true,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            transcript_byte_len: real_len,
        });
        let cache_path = view_cache_path(&transcript_path);
        let json = serde_json::to_string(&cache).unwrap_or_default();
        let _ = std::fs::write(&cache_path, json);
        (transcript_path, cache_path)
    }

    #[test]
    fn view_cache_loads_when_byte_lengths_match() {
        let sid = SessionId::new();
        let (transcript_path, _cache_path) = write_cache_fixture(sid, 0, None);
        let cache = try_load_view_cache(&transcript_path);
        assert!(cache.is_some(), "exact byte-length match → trusted");
        let _ =
            std::fs::remove_dir_all(transcript_path.parent().unwrap_or(std::path::Path::new("")));
    }

    #[test]
    fn view_cache_absent_returns_none() {
        let dir = std::env::temp_dir().join(format!("emberly-cache-absent-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let transcript_path = dir.join("00000000-0000-0000-0000-000000000001.jsonl");
        let _ = std::fs::write(&transcript_path, "some content\n");
        assert!(try_load_view_cache(&transcript_path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_cache_corrupt_returns_none() {
        let sid = SessionId::new();
        let dir =
            std::env::temp_dir().join(format!("emberly-cache-corrupt-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let transcript_path = dir.join(format!("{sid}.jsonl"));
        let _ = std::fs::write(&transcript_path, "some content\n");
        let cache_path = view_cache_path(&transcript_path);
        let _ = std::fs::write(&cache_path, "this is not json {{{");
        assert!(try_load_view_cache(&transcript_path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_cache_version_mismatch_returns_none() {
        let sid = SessionId::new();
        let stale = ViewCache {
            version: 999,
            session_id: sid,
            conversation: vec![],
            turn_map: vec![],
            next_turn: 0,
            compacted: false,
            original_task_recorded: false,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            transcript_byte_len: 0,
        };
        let (transcript_path, _cache_path) = write_cache_fixture(sid, 0, Some(stale));
        assert!(
            try_load_view_cache(&transcript_path).is_none(),
            "unknown version → stale"
        );
        let _ =
            std::fs::remove_dir_all(transcript_path.parent().unwrap_or(std::path::Path::new("")));
    }

    #[test]
    fn view_cache_transcript_grown_returns_none() {
        let sid = SessionId::new();
        // Delta > 0: append 3 bytes to the transcript after the cache was written.
        let (transcript_path, _cache_path) = write_cache_fixture(sid, 3, None);
        let cache = try_load_view_cache(&transcript_path);
        assert!(
            cache.is_none(),
            "transcript grew past the recorded offset → stale"
        );
        let _ =
            std::fs::remove_dir_all(transcript_path.parent().unwrap_or(std::path::Path::new("")));
    }

    #[test]
    fn view_cache_transcript_shorter_returns_none() {
        let sid = SessionId::new();
        // Write a fixture, then truncate the transcript to be shorter.
        let (transcript_path, _cache_path) = write_cache_fixture(sid, 0, None);
        let _ = std::fs::write(&transcript_path, "short");
        assert!(
            try_load_view_cache(&transcript_path).is_none(),
            "transcript is shorter than recorded → stale"
        );
        let _ =
            std::fs::remove_dir_all(transcript_path.parent().unwrap_or(std::path::Path::new("")));
    }

    #[test]
    fn view_cache_session_id_mismatch_returns_none() {
        let sid = SessionId::new();
        let wrong_sid = SessionId::new();
        let cache = ViewCache {
            version: VIEW_CACHE_VERSION,
            session_id: wrong_sid,
            conversation: vec![],
            turn_map: vec![],
            next_turn: 0,
            compacted: false,
            original_task_recorded: false,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            transcript_byte_len: 0,
        };
        let (transcript_path, _cache_path) = write_cache_fixture(sid, 0, Some(cache));
        // The transcript is named after `sid` but the cache claims `wrong_sid`.
        assert!(
            try_load_view_cache(&transcript_path).is_none(),
            "session-id mismatch → stale"
        );
        let _ =
            std::fs::remove_dir_all(transcript_path.parent().unwrap_or(std::path::Path::new("")));
    }
}
