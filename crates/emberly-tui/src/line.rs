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
    Command, FrontendPorts, PermissionDecision, PermissionId, PermissionRendering, UiEvent,
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
            UiEvent::ToolFinished { ok, summary, .. } => {
                let tag = if *ok { "ok" } else { "FAILED" };
                writeln!(out, "  [{tag}] {summary}")?;
            }
            UiEvent::FileModified { path, adds, dels } => {
                writeln!(out, "  ~ {path} (+{adds} -{dels})")?;
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
                    "  … retrying ({attempt}/{max_attempts}) in {delay_ms}ms — {reason}"
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
            // Context %, cost, sandbox, mode, compaction belong to the rich
            // TUI's status line (Phase 4); the plain frontend stays quiet.
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
        writeln!(out)?;
        if rendering.outside_root {
            writeln!(out, "!! THIS ACTION AFFECTS FILES OUTSIDE YOUR PROJECT !!")?;
        }
        writeln!(out, "PERMISSION REQUIRED: {}", rendering.summary)?;
        writeln!(out, "  why: {}", rendering.reason)?;
        if !rendering.affected_paths.is_empty() {
            writeln!(out, "  paths: {}", rendering.affected_paths.join(", "))?;
        }
        writeln!(out, "--- details ---")?;
        writeln!(out, "{}", rendering.detail.trim_end())?;
        writeln!(out, "---------------")?;
        writeln!(
            out,
            "Allow?  [y] allow once   [s] allow this session   [Enter] DENY"
        )?;
        Ok(())
    }
}

impl Default for LineRenderer {
    fn default() -> Self {
        Self::new()
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
    let mut stdin_open = true;
    loop {
        tokio::select! {
            event = events_rx.recv() => match event {
                Some(event) => {
                    renderer.render(&event, &mut stdout)?;
                    stdout.flush()?;
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
    fn tool_finished_shows_status() {
        let ok = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("c1"),
            ok: true,
            summary: "wrote a.txt".into(),
        });
        assert!(ok.contains("[ok]"));
        let failed = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("c1"),
            ok: false,
            summary: "denied".into(),
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
