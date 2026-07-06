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
            TranscriptEvent::AssistantMessage { text } => {
                let mut content = vec![ContentBlock::Text { text: text.clone() }];
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
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "on it".into(),
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
            }),
            rec(TranscriptEvent::AssistantMessage { text: "a".into() }),
            rec(TranscriptEvent::AssistantMessage { text: "b".into() }),
            rec(TranscriptEvent::AssistantMessage {
                text: "recent".into(),
            }),
            // Keep [0] (task) and [3] (recent); replace [1..3] with the summary.
            rec(TranscriptEvent::Compaction {
                summary: "summary so far".into(),
                replaced_from: 1,
                replaced_to: 3,
            }),
            rec(TranscriptEvent::UserMessage {
                text: "continue".into(),
                original_task: false,
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
    fn interrupted_unless_cleanly_ended() {
        let mid = vec![rec(TranscriptEvent::UserMessage {
            text: "x".into(),
            original_task: true,
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
}
