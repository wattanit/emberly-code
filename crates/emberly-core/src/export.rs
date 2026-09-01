//! Session export (FR-12, Tech Spec §8.6): a read-only, self-contained HTML
//! rendering of a session's full transcript, plus any subagents it spawned,
//! for sharing outside the harness. Reads existing transcript/derived-view
//! state; writes nothing back and adds no new persistence mechanism. First-
//! party string building — no templating-engine dependency (Tech Spec §12).

use std::collections::HashMap;
use std::path::Path;

use crate::resume::{self, try_load_view_cache};
use crate::transcript::{CompactTrigger, TranscriptEvent, TranscriptRecord};

/// One subagent's own nested transcript, labeled with its human-chosen name
/// where resolvable from the parent's own `spawn_agents` tool-result text
/// (FR-9) — falls back to its bare id when it isn't.
pub struct SubagentTranscript {
    pub id: String,
    pub name: Option<String>,
    pub records: Vec<TranscriptRecord>,
}

/// Read every subagent transcript under `subagents_dir` (the
/// `<sessions_dir>/<session_id>/subagents/<id>.jsonl` layout, Tech Spec
/// §8.4), labeling each from `parent_records`. A missing or unreadable
/// directory is simply no subagents — the common case — never an error.
#[must_use]
pub fn collect_subagent_transcripts(
    subagents_dir: &Path,
    parent_records: &[TranscriptRecord],
) -> Vec<SubagentTranscript> {
    let names = subagent_names(parent_records);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(subagents_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(loaded) = resume::read_records(&path) else {
            continue;
        };
        out.push(SubagentTranscript {
            id: id.to_string(),
            name: names.get(id).cloned(),
            records: loaded.records,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Parse `spawn_agents` tool-result lines — `"<id> (<name>): …"`, built by
/// `emberly_tools::builtin::spawn_agents` — into an id→name map. Best-effort
/// labeling, not load-bearing: a line that doesn't parse just leaves that
/// subagent labeled by its bare id.
fn subagent_names(records: &[TranscriptRecord]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for r in records {
        if let TranscriptEvent::ToolResult { output, .. } = &r.event {
            for line in output.lines() {
                let Some((id, rest)) = line.split_once(" (") else {
                    continue;
                };
                let Some((name, _)) = rest.split_once("):") else {
                    continue;
                };
                if !id.is_empty() && !name.is_empty() {
                    names.insert(id.to_string(), name.to_string());
                }
            }
        }
    }
    names
}

/// Render a session — plus any subagents it spawned — to one self-contained
/// HTML document: conversation, tool activity (respecting the truncation/
/// reduction markers already in the transcript), permission decisions, mode
/// changes, and a cost/usage summary when the derived-view cache for it is
/// still fresh (Requirements FR-12, P-6). No content redaction — the file
/// may carry anything the session touched; the Design §8.11 disclosure at
/// export time is the mitigation, not a filter built here.
#[must_use]
pub fn render_session_html(
    transcript_path: &Path,
    records: &[TranscriptRecord],
    subagents: &[SubagentTranscript],
) -> String {
    let title = resume::session_title(records).unwrap_or_else(|| "untitled session".into());
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html><head><meta charset=\"utf-8\">\n<title>");
    out.push_str(&escape(&title));
    out.push_str(" — Emberly session export</title>\n<style>");
    out.push_str(STYLE);
    out.push_str("</style></head><body>\n");
    out.push_str(&format!("<h1>{}</h1>\n", escape(&title)));

    if let Some(cache) = try_load_view_cache(transcript_path) {
        out.push_str(&format!(
            "<p class=\"usage\">{} in / {} out tokens · ${:.4} estimated cost</p>\n",
            cache.session_usage.input, cache.session_usage.output, cache.session_cost_usd
        ));
    } else {
        out.push_str(
            "<p class=\"usage\">usage/cost summary unavailable (no cached accounting for this session)</p>\n",
        );
    }

    out.push_str("<section class=\"session\">\n");
    render_records(records, &mut out);
    out.push_str("</section>\n");

    for sub in subagents {
        let label = sub.name.as_deref().unwrap_or(&sub.id);
        out.push_str(&format!(
            "<section class=\"subagent\"><h2>subagent: {}</h2>\n",
            escape(label)
        ));
        render_records(&sub.records, &mut out);
        out.push_str("</section>\n");
    }

    out.push_str("</body></html>\n");
    out
}

/// Render one session's (or subagent's) own record list as an ordered list
/// of flow items, in the register each event already carries (agent-world
/// vs harness-world, Design §6.1) — plain, honest, and complete.
fn render_records(records: &[TranscriptRecord], out: &mut String) {
    out.push_str("<ol class=\"flow\">\n");
    for r in records {
        let Some(item) = render_event(&r.event) else {
            continue;
        };
        out.push_str(&format!(
            "<li class=\"{}\">{}</li>\n",
            item.css_class, item.html
        ));
    }
    out.push_str("</ol>\n");
}

struct FlowItem {
    css_class: &'static str,
    html: String,
}

fn item(css_class: &'static str, html: String) -> Option<FlowItem> {
    Some(FlowItem { css_class, html })
}

/// One transcript event's flow rendering, or `None` for events with nothing
/// to show in an export (e.g. `session_start`, already summarized in the
/// page header).
fn render_event(event: &TranscriptEvent) -> Option<FlowItem> {
    match event {
        TranscriptEvent::UserMessage { text, images, .. } => {
            let mut html = format!("<strong>user:</strong> {}", escape(text));
            for image in images {
                html.push_str(&format!(
                    "<div class=\"attachment\">📎 {} · {}×{} · {}</div>",
                    escape(&image.name),
                    image.width,
                    image.height,
                    escape(&image.format_label)
                ));
            }
            item("user", html)
        }
        TranscriptEvent::AssistantMessage { text, .. } if !text.is_empty() => item(
            "assistant",
            format!("<strong>assistant:</strong> {}", escape(text)),
        ),
        TranscriptEvent::AssistantMessage { .. } => None,
        TranscriptEvent::ToolCall { tool, args, .. } => item(
            "tool",
            format!(
                "<code>{}</code> <span class=\"args\">{}</span>",
                escape(tool),
                escape(&args.to_string())
            ),
        ),
        TranscriptEvent::ToolResult {
            ok,
            output,
            truncated,
            ..
        } => {
            let status = if *ok { "ok" } else { "FAILED" };
            let note = if *truncated {
                " (truncated at ingestion)"
            } else {
                ""
            };
            item(
                "tool-result",
                format!("→ {status}{note}: {}", escape(output)),
            )
        }
        TranscriptEvent::PermissionDecision {
            decision, executed, ..
        } => {
            let mut html = format!("permission: {decision:?}");
            if let Some(cmd) = executed {
                html.push_str(&format!(" — {}", escape(cmd)));
            }
            item("permission", html)
        }
        TranscriptEvent::ModeChange { mode } => item("mode", format!("mode changed: {mode:?}")),
        TranscriptEvent::ModelSwitch { provider, model } => item(
            "model",
            format!("switched model: {}/{}", escape(provider), escape(model)),
        ),
        TranscriptEvent::Compaction {
            summary, trigger, ..
        } => {
            let who = match trigger {
                CompactTrigger::Manual => "manual",
                CompactTrigger::Auto => "automatic",
            };
            item(
                "compaction",
                format!("{who} compaction: {}", escape(summary)),
            )
        }
        TranscriptEvent::LoopHalt { reason, .. } => {
            item("halt", format!("loop guardrail halted: {}", escape(reason)))
        }
        TranscriptEvent::CompletionGateHalt {
            failing, attempts, ..
        } => item(
            "halt",
            format!(
                "completion gate halted after {attempts} attempt(s): {} failing check(s)",
                failing.len()
            ),
        ),
        TranscriptEvent::AskUser {
            question, answer, ..
        } => item(
            "ask",
            format!(
                "asked: {} → {}",
                escape(question),
                answer
                    .as_deref()
                    .map(escape)
                    .unwrap_or_else(|| "(declined)".into())
            ),
        ),
        TranscriptEvent::ProviderError { what, why, .. } => item(
            "error",
            format!("provider error: {} — {}", escape(what), escape(why)),
        ),
        TranscriptEvent::TrustDecision { path, trusted } => item(
            "trust",
            format!("workspace trust granted for {} ({trusted})", escape(path)),
        ),
        _ => None,
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const STYLE: &str = "
body { font-family: system-ui, sans-serif; max-width: 60rem; margin: 2rem auto; padding: 0 1rem; color: #1a1a1a; background: #fff; }
h1 { font-size: 1.4rem; }
h2 { font-size: 1.1rem; margin-top: 2rem; border-top: 1px solid #ddd; padding-top: 1rem; }
p.usage { color: #666; font-size: 0.9rem; }
ol.flow { list-style: none; padding: 0; }
ol.flow li { padding: 0.4rem 0; border-bottom: 1px solid #eee; white-space: pre-wrap; word-break: break-word; }
li.user { font-weight: 500; }
li.tool code { background: #f2f2f2; padding: 0.1rem 0.3rem; border-radius: 3px; }
li.tool-result { color: #333; font-family: ui-monospace, monospace; font-size: 0.9rem; }
li.halt, li.error { color: #a33; }
.attachment { color: #666; font-size: 0.9rem; }
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ToolCallId;
    use time::OffsetDateTime;

    fn rec(event: TranscriptEvent) -> TranscriptRecord {
        TranscriptRecord::new(OffsetDateTime::UNIX_EPOCH, event)
    }

    #[test]
    fn renders_conversation_and_tool_activity() {
        let records = vec![
            rec(TranscriptEvent::UserMessage {
                text: "hi".into(),
                original_task: true,
                images: Vec::new(),
            }),
            rec(TranscriptEvent::ToolCall {
                call_id: ToolCallId("c1".into()),
                tool: "read_file".into(),
                args: serde_json::json!({"path": "a.rs"}),
            }),
            rec(TranscriptEvent::ToolResult {
                call_id: ToolCallId("c1".into()),
                ok: true,
                output: "contents".into(),
                truncated: false,
                full_output_ref: None,
            }),
            rec(TranscriptEvent::AssistantMessage {
                text: "done".into(),
                reasoning: None,
            }),
        ];
        let html = render_session_html(Path::new("/nonexistent.jsonl"), &records, &[]);
        assert!(html.contains("hi"));
        assert!(html.contains("read_file"));
        assert!(html.contains("contents"));
        assert!(html.contains("done"));
        assert!(html.contains("usage/cost summary unavailable"));
    }

    #[test]
    fn escapes_html_special_characters() {
        let records = vec![rec(TranscriptEvent::UserMessage {
            text: "<script>alert(1)</script>".into(),
            original_task: true,
            images: Vec::new(),
        })];
        let html = render_session_html(Path::new("/nonexistent.jsonl"), &records, &[]);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn includes_subagent_sections_labeled_by_name() {
        let parent = vec![rec(TranscriptEvent::ToolResult {
            call_id: ToolCallId("c1".into()),
            ok: true,
            output: "abc-123 (db-migration): done".into(),
            truncated: false,
            full_output_ref: None,
        })];
        let subagent_records = vec![rec(TranscriptEvent::UserMessage {
            text: "migrate the db".into(),
            original_task: true,
            images: Vec::new(),
        })];
        let subagents = vec![SubagentTranscript {
            id: "abc-123".into(),
            name: subagent_names(&parent).get("abc-123").cloned(),
            records: subagent_records,
        }];
        let html = render_session_html(Path::new("/nonexistent.jsonl"), &parent, &subagents);
        assert!(html.contains("subagent: db-migration"));
        assert!(html.contains("migrate the db"));
    }

    #[test]
    fn collect_subagent_transcripts_on_missing_dir_is_empty_not_an_error() {
        let out = collect_subagent_transcripts(Path::new("/does/not/exist"), &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn never_appears_to_mutate_records_it_only_reads() {
        // A pure smoke check that render_session_html takes shared refs only
        // (enforced by the type signature) — the compiler is the real guard
        // here; this test documents the intent alongside it.
        let records = vec![rec(TranscriptEvent::SessionTitle { title: "t".into() })];
        let before = records.clone();
        let _ = render_session_html(Path::new("/nonexistent.jsonl"), &records, &[]);
        assert_eq!(records, before);
    }
}
