//! `emberly export` (FR-12, Design §8.11, Tech Spec §8.6/§10) — export a
//! saved session, plus any subagents it spawned, to a self-contained HTML
//! file. A read-only CLI counterpart to the in-session `/export` command
//! (`emberly-tui`), sharing the same `emberly_core::render_session_html`
//! renderer so the two entry points never diverge.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// `emberly export [--session <id>] <output-path>`. With no `--session`, the
/// most recently modified session in `sessions_dir` is exported. Prints
/// exactly what it wrote and where (the `init`/`clean` voice, Design §8.1/
/// §8.8), plus the one calm, non-blocking sensitive-content disclosure line
/// (FR-12 — export does not redact).
pub fn export(sessions_dir: &Path, session_id: Option<&str>, output_path: &str) -> Result<()> {
    let transcript_path = resolve_transcript_path(sessions_dir, session_id)?;
    let loaded = emberly_core::resume::read_records(&transcript_path)
        .with_context(|| format!("reading {}", transcript_path.display()))?;

    let session_dir_name = transcript_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let subagents_dir = sessions_dir.join(session_dir_name).join("subagents");
    let subagents = emberly_core::collect_subagent_transcripts(&subagents_dir, &loaded.records);

    let html = emberly_core::render_session_html(&transcript_path, &loaded.records, &subagents);
    std::fs::write(output_path, html).with_context(|| format!("writing {output_path}"))?;

    println!("exported to {output_path}");
    println!("{}", emberly_tui::strings::export::SENSITIVE_CONTENT_NOTE);
    Ok(())
}

/// Resolve which session's transcript to export: the named `--session <id>`,
/// or the most recently modified session in `sessions_dir` (mirroring
/// `resume [id]`'s own resolution, `main.rs`).
fn resolve_transcript_path(sessions_dir: &Path, session_id: Option<&str>) -> Result<PathBuf> {
    match session_id {
        Some(id) => Ok(sessions_dir.join(format!("{id}.jsonl"))),
        None => emberly_core::resume::latest_session(sessions_dir)
            .context("no sessions found to export in .agents/sessions"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(label: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emberly-export-test-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn resolve_by_explicit_id_is_a_plain_path_join() {
        let dir = temp_dir("id");
        let path = resolve_transcript_path(&dir, Some("abc-123")).expect("resolves");
        assert_eq!(path, dir.join("abc-123.jsonl"));
    }

    #[test]
    fn resolve_with_no_id_and_no_sessions_is_an_error_not_a_panic() {
        let dir = temp_dir("empty");
        assert!(resolve_transcript_path(&dir, None).is_err());
    }

    #[test]
    fn export_writes_a_file_and_reports_it() {
        let dir = temp_dir("full");
        let transcript = dir.join("s1.jsonl");
        let _ = std::fs::write(
            &transcript,
            "{\"v\":2,\"ts\":\"2026-01-01T00:00:00Z\",\"type\":\"user_message\",\"text\":\"hi\",\"original_task\":true}\n",
        );
        let output = dir.join("out.html");
        let result = export(&dir, Some("s1"), output.to_str().expect("utf8 path"));
        assert!(result.is_ok(), "{result:?}");
        let html = std::fs::read_to_string(&output).expect("export file written");
        assert!(html.contains("hi"));
    }

    /// End-to-end (FR-12, Design §8.11): export a real session that spawned a
    /// real subagent, through the same real files this command reads in
    /// production (not the in-memory `render_session_html` unit tests in
    /// `emberly-core`), and confirm both source transcripts are read-only —
    /// byte-for-byte unchanged on disk after export, never a mutation.
    #[test]
    fn export_includes_a_real_subagent_and_never_mutates_either_source_file() {
        let dir = temp_dir("subagent");
        let session_id = "s1";
        let transcript = dir.join(format!("{session_id}.jsonl"));
        let parent_lines = concat!(
            "{\"v\":2,\"ts\":\"2026-01-01T00:00:00Z\",\"type\":\"user_message\",",
            "\"text\":\"migrate the db\",\"original_task\":true}\n",
            "{\"v\":2,\"ts\":\"2026-01-01T00:00:01Z\",\"type\":\"tool_result\",",
            "\"call_id\":\"call_1\",\"ok\":true,",
            "\"output\":\"abc-123 (db-migration): done\",\"truncated\":false}\n",
        );
        std::fs::write(&transcript, parent_lines).expect("parent transcript written");

        let subagents_dir = dir.join(session_id).join("subagents");
        std::fs::create_dir_all(&subagents_dir).expect("subagents dir created");
        let subagent_transcript = subagents_dir.join("abc-123.jsonl");
        let subagent_lines = concat!(
            "{\"v\":2,\"ts\":\"2026-01-01T00:00:02Z\",\"type\":\"user_message\",",
            "\"text\":\"run the pending migrations\",\"original_task\":true}\n",
        );
        std::fs::write(&subagent_transcript, subagent_lines).expect("subagent transcript written");

        let parent_before = std::fs::read(&transcript).expect("parent readable before export");
        let subagent_before =
            std::fs::read(&subagent_transcript).expect("subagent readable before export");

        let output = dir.join("out.html");
        let result = export(&dir, Some(session_id), output.to_str().expect("utf8 path"));
        assert!(result.is_ok(), "{result:?}");

        let html = std::fs::read_to_string(&output).expect("export file written");
        assert!(html.contains("migrate the db"), "{html}");
        assert!(html.contains("subagent: db-migration"), "{html}");
        assert!(html.contains("run the pending migrations"), "{html}");

        let parent_after = std::fs::read(&transcript).expect("parent readable after export");
        let subagent_after =
            std::fs::read(&subagent_transcript).expect("subagent readable after export");
        assert_eq!(
            parent_before, parent_after,
            "export must never mutate the source session transcript"
        );
        assert_eq!(
            subagent_before, subagent_after,
            "export must never mutate the source subagent transcript"
        );
    }
}
