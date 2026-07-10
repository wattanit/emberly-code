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
use std::path::{Path, PathBuf};

use emberly_core::{
    Command, FrontendPorts, Mode, PermissionDecision, PermissionId, PermissionRendering,
    SandboxStatus, UiEvent,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::app::ReasoningView;

/// Renders [`UiEvent`]s as append-only lines. Holds no color or cursor state
/// (degraded mode, Design §7); the only state is the reasoning-block toggle so
/// the plain `--- reasoning ---` block is labeled once (Design §4.4).
pub struct LineRenderer {
    reasoning_view: ReasoningView,
    /// True while streaming a reasoning block, so the answer that follows gets a
    /// separating label.
    in_reasoning: bool,
}

impl LineRenderer {
    #[must_use]
    pub fn new(reasoning_view: ReasoningView) -> Self {
        Self {
            reasoning_view,
            in_reasoning: false,
        }
    }

    /// Write one event to `out`. Assistant deltas stream without a trailing
    /// newline; `AssistantDone` closes the line.
    pub fn render(&mut self, event: &UiEvent, out: &mut impl Write) -> io::Result<()> {
        match event {
            UiEvent::AssistantDelta { text } => {
                // Close a preceding reasoning block with a plain label so the
                // answer is never confused with the thinking (Design §4.4, §7).
                if self.in_reasoning {
                    writeln!(out, "\n--- answer ---")?;
                    self.in_reasoning = false;
                }
                write!(out, "{text}")?;
            }
            UiEvent::ReasoningDelta { text } => {
                // `hidden` suppresses the trail in the view; the trace is still
                // recorded by the engine (P-10).
                if self.reasoning_view != ReasoningView::Hidden {
                    if !self.in_reasoning {
                        writeln!(out, "--- reasoning ---")?;
                        self.in_reasoning = true;
                    }
                    write!(out, "{text}")?;
                }
            }
            UiEvent::AssistantDone => {
                self.in_reasoning = false;
                writeln!(out)?;
            }
            UiEvent::ToolStarted {
                tool,
                summary,
                explanation,
                ..
            } => {
                writeln!(out, "\n> {tool}: {summary}")?;
                // The model's caption (T-9), indented under the call. ASCII
                // lead so degraded mode carries it without a glyph or colour
                // (Design §4.5/§7); absent → nothing.
                if let Some(explanation) = explanation {
                    writeln!(out, "    - {explanation}")?;
                }
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
        Self::new(ReasoningView::default())
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
pub async fn run(
    ports: FrontendPorts,
    sessions_dir: PathBuf,
    config_template: String,
    reasoning_view: ReasoningView,
) -> io::Result<()> {
    // `.agents/` is the parent of the sessions dir; `/config` and `/prompt`
    // resolve their targets under it (C-5).
    let agents_dir = sessions_dir
        .parent()
        .map_or(sessions_dir.clone(), Path::to_path_buf);
    let mut renderer = LineRenderer::new(reasoning_view);
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
                    } else if let Some(args) = line
                        .trim()
                        .strip_prefix("/model")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        // Degraded mode has no picker, so `/model` needs a
                        // profile argument (C-6); the engine validates it.
                        let mut parts = args.split_whitespace();
                        if let Some(profile) = parts.next() {
                            let model = parts.next().map(str::to_string);
                            let _ = tx.send(Command::SwitchModel { profile: profile.to_string(), model }).await;
                        } else {
                            println!("usage: /model <profile> [model]");
                        }
                    } else if line.trim() == "/reload" {
                        let _ = tx.send(Command::ReloadConfig).await;
                    } else if line.trim() == "/config" {
                        // Line mode does not launch $EDITOR (stdin is the line
                        // reader / often a pipe): seed + point at the file, then
                        // the user edits it and runs /reload (C-5, degraded §7).
                        match crate::edit::config_target(&agents_dir, &config_template) {
                            Ok((path, existed)) => println!(
                                "{} {} — edit it, then /reload to apply",
                                if existed { "editing" } else { "created" },
                                path.display()
                            ),
                            Err(e) => println!("could not prepare config: {e}"),
                        }
                    } else if let Some(rest) = line
                        .trim()
                        .strip_prefix("/prompt")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        match crate::edit::prompt_target(&agents_dir, rest.trim()) {
                            Ok((path, existed)) => println!(
                                "{} {} — edit it, then /reload to apply",
                                if existed { "editing" } else { "created" },
                                path.display()
                            ),
                            Err(msg) => println!("{msg}"),
                        }
                    } else if let Some(rest) = line
                        .trim()
                        .strip_prefix("/effort")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        // Degraded mode has no picker, so `/effort` needs a level
                        // argument (P-9); the engine drops it if the model has no
                        // control.
                        match emberly_core::Effort::parse(rest.trim()) {
                            Some(effort) => {
                                let _ = tx.send(Command::SetEffort { effort }).await;
                            }
                            None => println!("usage: /effort <low|medium|high|max>"),
                        }
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
        render_with(ReasoningView::Collapsed, &[event])
    }

    /// Render a sequence of events through one renderer (state carries across
    /// events, e.g. the reasoning→answer transition).
    fn render_with(view: ReasoningView, events: &[&UiEvent]) -> String {
        let mut renderer = LineRenderer::new(view);
        let mut buf: Vec<u8> = Vec::new();
        for event in events {
            let ok = renderer.render(event, &mut buf).is_ok();
            assert!(ok, "render should not fail writing to a Vec");
        }
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
    fn reasoning_renders_a_labeled_block_then_the_answer() {
        let out = render_with(
            ReasoningView::Collapsed,
            &[
                &UiEvent::ReasoningDelta {
                    text: "let me think".into(),
                },
                &UiEvent::AssistantDelta {
                    text: "answer".into(),
                },
                &UiEvent::AssistantDone,
            ],
        );
        assert!(out.contains("--- reasoning ---"), "labeled block: {out:?}");
        assert!(out.contains("let me think"));
        assert!(out.contains("--- answer ---"), "answer separated: {out:?}");
        assert!(out.contains("answer"));
    }

    #[test]
    fn hidden_view_suppresses_reasoning_in_plain_mode() {
        let out = render_with(
            ReasoningView::Hidden,
            &[
                &UiEvent::ReasoningDelta {
                    text: "secret".into(),
                },
                &UiEvent::AssistantDelta {
                    text: "answer".into(),
                },
            ],
        );
        assert!(!out.contains("reasoning"), "no trail when hidden: {out:?}");
        assert!(!out.contains("secret"));
        assert!(out.contains("answer"));
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
    fn tool_call_explanation_renders_indented_under_the_call() {
        let out = render_to_string(&UiEvent::ToolStarted {
            call_id: ToolCallId::new("c"),
            tool: "bash".into(),
            summary: "run: sed -i s/debug/info/ log.conf".into(),
            explanation: Some("raise the log level to info".into()),
        });
        assert!(
            out.contains("run: sed"),
            "the call stays the headline: {out:?}"
        );
        assert!(
            out.contains("- raise the log level to info"),
            "caption indented under the call, ASCII lead (Design §4.5/§7): {out:?}"
        );
    }

    #[test]
    fn no_explanation_renders_no_caption_line() {
        let out = render_to_string(&UiEvent::ToolStarted {
            call_id: ToolCallId::new("c"),
            tool: "read_file".into(),
            summary: "read src/main.rs".into(),
            explanation: None,
        });
        // Only the call line — no dangling indented caption (no placeholder).
        assert_eq!(out.trim(), "> read_file: read src/main.rs");
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
                // Exercise the caption path in the no-ANSI sweep too.
                explanation: Some("list the working tree".into()),
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
