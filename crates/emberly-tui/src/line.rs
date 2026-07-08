//! The line-mode frontend (Tech Spec §9 degraded contract, A-1). Plain,
//! append-only, ASCII-only output and line-buffered input — no color, no
//! cursor repositioning. It is the seed of `--plain`/degraded mode and the
//! contract for a future headless frontend; the rich `ratatui` TUI arrives in
//! Phase 4.
//!
//! Rendering ([`LineRenderer::render`]) is a pure function over a writer, so it
//! is unit-testable without a terminal. The async [`run`] driver wires stdin
//! lines and [`UiEvent`]s to the engine's channels.

use std::io::{self, Write};

use emberly_core::{
    Command, FrontendPorts, Mode, PermissionDecision, PermissionId, PermissionRendering,
    SandboxStatus, UiEvent,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Renders [`UiEvent`]s as append-only lines. Stateless; holds no color or
/// cursor state (degraded mode, Design §7).
pub struct LineRenderer;

impl LineRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Write one event to `out`. Assistant deltas stream without a trailing
    /// newline; `AssistantDone` closes the line.
    pub fn render(&self, event: &UiEvent, out: &mut impl Write) -> io::Result<()> {
        match event {
            UiEvent::AssistantDelta { text } => write!(out, "{text}")?,
            UiEvent::AssistantDone => writeln!(out)?,
            UiEvent::ToolStarted { tool, summary, .. } => {
                writeln!(out, "\n> {tool}: {summary}")?;
            }
            UiEvent::ToolFinished {
                ok,
                summary,
                preview,
                ..
            } => {
                let tag = if *ok { "ok" } else { "FAILED" };
                writeln!(out, "  [{tag}] {summary}")?;
                // Show the result excerpt, indented, so the plain frontend also
                // says what the tool produced (Design §6.1).
                for line in preview.lines() {
                    writeln!(out, "    {line}")?;
                }
            }
            UiEvent::FileModified { path, adds, dels } => {
                writeln!(out, "  ~ {path} (+{adds} -{dels})")?;
            }
            UiEvent::FileDiff { unified, .. } => {
                // The +/- prefixes carry the change without colour (Design §7).
                for line in unified.lines() {
                    writeln!(out, "  {line}")?;
                }
            }
            UiEvent::PermissionRequest { rendering, .. } => {
                self.render_permission(rendering, out)?
            }
            UiEvent::HarnessError { what, why, next } => {
                writeln!(out, "\nerror: {what}")?;
                writeln!(out, "  why:  {why}")?;
                writeln!(out, "  next: {next}")?;
            }
            UiEvent::Retrying {
                attempt,
                max_attempts,
                delay_ms,
                reason,
            } => {
                writeln!(
                    out,
                    "  ... retrying ({attempt}/{max_attempts}) in {delay_ms}ms - {reason}"
                )?;
            }
            UiEvent::SessionMeta {
                title,
                provider,
                model,
                project_root,
                ..
            } => {
                writeln!(
                    out,
                    "session: {title}  [{provider}/{model}]  {project_root}"
                )?;
            }
            UiEvent::SandboxStatus { status } => {
                let summary = match status {
                    SandboxStatus::Confined { backend } => backend.clone(),
                    SandboxStatus::Partial { backend, missing } => {
                        format!("{backend} (partial: {missing})")
                    }
                    SandboxStatus::Unavailable { reason } => format!("unavailable ({reason})"),
                };
                writeln!(out, "sandbox: {summary}")?;
            }
            UiEvent::ModeChanged { mode } => {
                let word = match mode {
                    Mode::Normal => "normal",
                    Mode::AutoAcceptEdits => "auto-accept edits",
                    Mode::Auto => "auto",
                };
                writeln!(out, "mode: {word}")?;
            }
            UiEvent::Notice { message } => {
                // Harness-voice notice: the degraded-sandbox explanation, a
                // saved permission grant, a refused mode switch (Design §8.2).
                for line in message.lines() {
                    writeln!(out, "note: {line}")?;
                }
            }
            // Context % and cost belong to the rich TUI's status line (Phase 4);
            // the plain frontend stays quiet on those.
            _ => {}
        }
        Ok(())
    }

    /// The permission prompt in degraded form (Design §5, §7): full content
    /// never truncated, an OUTSIDE-PROJECT banner in capitals, deny as the
    /// default (Enter), and a distinct, deliberate approve key.
    fn render_permission(
        &self,
        rendering: &PermissionRendering,
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::permission as p;
        writeln!(out)?;
        if rendering.outside_root {
            // Capitalised banner carries the meaning colour would in rich mode
            // (Design §7); shared verbatim with the TUI for parity.
            writeln!(out, "!! {} !!", p::OUTSIDE_ROOT_BANNER)?;
        }
        writeln!(out, "{}: {}", p::HEADING, rendering.summary)?;
        writeln!(out, "  {}: {}", p::WHY_LABEL, rendering.reason)?;
        if !rendering.affected_paths.is_empty() {
            writeln!(
                out,
                "  {}: {}",
                p::PATHS_LABEL,
                rendering.affected_paths.join(", ")
            )?;
        }
        writeln!(out, "--- details ---")?;
        writeln!(out, "{}", rendering.detail.trim_end())?;
        writeln!(out, "---------------")?;
        writeln!(
            out,
            "Allow?  [y] {}   [s] {}   [Enter] {}",
            p::ALLOW_ONCE,
            p::ALLOW_SESSION,
            p::DENY
        )?;
        Ok(())
    }
}

impl Default for LineRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// The next auto-accept tier in the cycle (normal → auto-accept-edits → auto →
/// normal). The engine still gates the auto tiers on confinement (Tech Spec
/// §6.6); this only proposes the next one.
#[must_use]
pub fn next_mode(current: Mode) -> Mode {
    match current {
        Mode::Normal => Mode::AutoAcceptEdits,
        Mode::AutoAcceptEdits => Mode::Auto,
        Mode::Auto => Mode::Normal,
    }
}

/// Interpret a permission answer line. Deny is the safe default: only an
/// explicit, deliberate key allows (Design §5 — Enter/anything else denies).
#[must_use]
pub fn parse_permission_answer(line: &str) -> PermissionDecision {
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" => PermissionDecision::AllowOnce,
        "s" | "session" => PermissionDecision::AllowForSession,
        _ => PermissionDecision::Deny,
    }
}

/// Run the line-mode frontend: render events to stdout, forward stdin lines to
/// the engine as commands. Returns when either channel closes.
///
/// While a permission prompt is open, the next input line is its answer; a
/// `/cancel` line cancels the current turn; anything else is a user message.
pub async fn run(ports: FrontendPorts) -> io::Result<()> {
    let renderer = LineRenderer::new();
    let mut stdout = io::stdout();
    let mut events_rx = ports.events_rx;
    // Held in an Option so stdin EOF can drop it, signaling the engine to
    // finish; we keep draining events until it closes its side.
    let mut commands_tx = Some(ports.commands_tx);

    let (lines_tx, mut lines_rx) = mpsc::channel::<String>(16);
    tokio::spawn(async move {
        let mut reader = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if lines_tx.send(line).await.is_err() {
                break;
            }
        }
    });

    let mut pending: Option<PermissionId> = None;
    // Track the current tier so `/mode` can cycle it (the engine gates the auto
    // tiers on confinement and echoes a ModeChanged / Notice back).
    let mut mode = Mode::default();
    let mut stdin_open = true;
    loop {
        tokio::select! {
            event = events_rx.recv() => match event {
                Some(event) => {
                    renderer.render(&event, &mut stdout)?;
                    stdout.flush()?;
                    if let UiEvent::ModeChanged { mode: changed } = &event {
                        mode = *changed;
                    }
                    if let UiEvent::PermissionRequest { id, .. } = event {
                        pending = Some(id);
                    }
                }
                None => break, // engine finished and closed its events
            },
            line = lines_rx.recv(), if stdin_open => match (line, commands_tx.as_ref()) {
                (Some(line), Some(tx)) => {
                    if let Some(id) = pending.take() {
                        let _ = tx.send(Command::PermissionAnswer { id, decision: parse_permission_answer(&line) }).await;
                    } else if line.trim() == "/cancel" {
                        let _ = tx.send(Command::Cancel).await;
                    } else if line.trim() == "/mode" {
                        let _ = tx.send(Command::SetMode { mode: next_mode(mode) }).await;
                    } else if !line.trim().is_empty() {
                        let _ = tx.send(Command::UserInput { text: line }).await;
                    }
                }
                _ => {
                    // stdin closed: drop the command sender so the engine
                    // finishes the current turn and closes its events. We keep
                    // draining events until it does (no forced cancel — an
                    // in-flight reply should still be shown).
                    commands_tx = None;
                    stdin_open = false;
                }
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use emberly_core::{PermissionRendering, ToolCallId};

    fn render_to_string(event: &UiEvent) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let ok = LineRenderer::new().render(event, &mut buf).is_ok();
        assert!(ok, "render should not fail writing to a Vec");
        String::from_utf8(buf).unwrap_or_default()
    }

    #[test]
    fn assistant_delta_streams_without_newline() {
        assert_eq!(
            render_to_string(&UiEvent::AssistantDelta { text: "hi".into() }),
            "hi"
        );
        assert_eq!(render_to_string(&UiEvent::AssistantDone), "\n");
    }

    #[test]
    fn file_diff_shows_plus_minus_prefixes() {
        let out = render_to_string(&UiEvent::FileDiff {
            path: "a.rs".into(),
            unified: "--- a/a.rs\n+++ b/a.rs\n-old\n+new".into(),
        });
        assert!(out.contains("-old"), "deletions carry a - prefix");
        assert!(out.contains("+new"), "additions carry a + prefix");
    }

    #[test]
    fn degraded_output_has_no_ansi_escapes() {
        // Degraded mode is colourless and append-only: no ANSI/cursor control
        // ever reaches the stream (Design §7).
        let events = [
            UiEvent::AssistantDelta {
                text: "สวัสดี".into(),
            },
            UiEvent::ToolStarted {
                call_id: ToolCallId::new("c"),
                tool: "bash".into(),
                summary: "run: ls".into(),
            },
            UiEvent::ToolFinished {
                call_id: ToolCallId::new("c"),
                ok: true,
                summary: "exit 0".into(),
                preview: "total 0".into(),
            },
            UiEvent::FileDiff {
                path: "a".into(),
                unified: "-x\n+y".into(),
            },
            UiEvent::Retrying {
                attempt: 1,
                max_attempts: 3,
                delay_ms: 500,
                reason: "429".into(),
            },
        ];
        for e in events {
            let s = render_to_string(&e);
            assert!(!s.contains('\u{1b}'), "no ANSI escape in {s:?}");
        }
    }

    #[test]
    fn tool_finished_shows_status() {
        let ok = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("c1"),
            ok: true,
            summary: "wrote a.txt".into(),
            preview: "done".into(),
        });
        assert!(ok.contains("[ok]"));
        assert!(ok.contains("done"), "preview shown in degraded mode");
        let failed = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("c1"),
            ok: false,
            summary: "denied".into(),
            preview: String::new(),
        });
        assert!(failed.contains("[FAILED]"));
    }

    #[test]
    fn permission_prompt_is_full_and_deny_default() {
        let rendering = PermissionRendering {
            tool: "bash".into(),
            summary: "run: rm -rf build".into(),
            detail: "rm -rf build".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash requires your approval".into(),
        };
        let out = render_to_string(&UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering,
        });
        assert!(out.contains("PERMISSION REQUIRED"));
        assert!(out.contains("rm -rf build"), "full command must be shown");
        assert!(out.contains("[Enter] DENY"), "deny is the default");
    }

    #[test]
    fn outside_root_prompt_has_loud_banner() {
        let rendering = PermissionRendering {
            tool: "write_file".into(),
            summary: "write /etc/x".into(),
            detail: "…".into(),
            affected_paths: vec!["/etc/x".into()],
            outside_root: true,
            reason: "this action affects files OUTSIDE the project root".into(),
        };
        let out = render_to_string(&UiEvent::PermissionRequest {
            id: PermissionId(2),
            rendering,
        });
        assert!(out.contains("OUTSIDE YOUR PROJECT"));
    }

    #[test]
    fn answer_parsing_defaults_to_deny() {
        assert_eq!(parse_permission_answer("y"), PermissionDecision::AllowOnce);
        assert_eq!(
            parse_permission_answer("YES"),
            PermissionDecision::AllowOnce
        );
        assert_eq!(
            parse_permission_answer("s"),
            PermissionDecision::AllowForSession
        );
        assert_eq!(parse_permission_answer(""), PermissionDecision::Deny);
        assert_eq!(parse_permission_answer("no"), PermissionDecision::Deny);
        assert_eq!(
            parse_permission_answer("anything"),
            PermissionDecision::Deny
        );
    }
}
