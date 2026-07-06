//! The rich `ratatui` frontend driver (Tech Spec §9). Owns the terminal
//! lifecycle ([`crate::terminal`]), runs the event loop, and renders the
//! [`App`] view-model.
//!
//! The loop `select!`s over three sources — engine [`UiEvent`]s, terminal input
//! events, and (from group 9) an animation tick — and never lets any one block
//! the others: input handling is independent of streaming, and a redraw never
//! delays a command. Input is read on a dedicated OS thread because
//! `crossterm::event::read` is blocking; it forwards events over a channel,
//! mirroring the line-mode stdin reader.
//!
//! Group 1 renders a minimal single-pane layout to prove the loop and the
//! terminal lifecycle end to end; group 4 replaces [`render`] with the real
//! two-pane layout, and groups 5–7 fill in markdown, diffs, and the permission
//! prompt.

use std::io;

use crossterm::event::Event;
use emberly_core::FrontendPorts;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::app::{Action, App, ConvItem, SessionInfo};
use crate::strings;
use crate::terminal::TerminalGuard;

/// Run the rich TUI until the user quits or the engine closes its event stream.
/// Sets up and tears down the terminal via [`TerminalGuard`] (HC-3).
pub async fn run(ports: FrontendPorts, session: SessionInfo) -> io::Result<()> {
    let mut guard = TerminalGuard::enter()?;
    let mut app = App::new(session);

    let mut input_rx = spawn_input_reader();
    let mut events_rx = ports.events_rx;
    // Dropped when this function returns (on quit or engine close), which
    // closes the command channel — the engine then finishes and closes its
    // events. No hard cancel: an in-flight reply is still allowed to complete.
    let commands_tx = ports.commands_tx;

    guard.terminal().draw(|f| render(f, &app))?;

    loop {
        tokio::select! {
            event = events_rx.recv() => match event {
                Some(event) => {
                    app.apply_event(event);
                    guard.terminal().draw(|f| render(f, &app))?;
                }
                None => break, // engine finished and closed its events
            },
            input = input_rx.recv() => match input {
                Some(Event::Key(key)) if is_press(&key) => {
                    match app.on_key(key) {
                        Action::Quit => break,
                        Action::Command(cmd) => {
                            let _ = commands_tx.send(cmd).await;
                        }
                        Action::None => {}
                    }
                    guard.terminal().draw(|f| render(f, &app))?;
                }
                Some(Event::Resize(_, _)) => {
                    guard.terminal().draw(|f| render(f, &app))?;
                }
                Some(_) => {} // paste / mouse / focus — handled in later groups
                None => break, // input thread ended (stdin closed)
            },
        }
    }

    Ok(())
}

/// Only act on key *presses*. On terminals that report key-release events
/// (Kitty protocol) this avoids double-handling; elsewhere it is a no-op since
/// every event is a press.
fn is_press(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyEventKind;
    key.kind == KeyEventKind::Press
}

/// Spawn the blocking terminal-input reader on its own OS thread, forwarding
/// events over a channel. `crossterm::event::read` blocks, so it cannot live in
/// the async `select!`; the thread ends when the channel closes or the read
/// errors (terminal gone).
fn spawn_input_reader() -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel::<Event>(64);
    std::thread::spawn(move || {
        while let Ok(event) = crossterm::event::read() {
            if tx.blocking_send(event).is_err() {
                break;
            }
        }
    });
    rx
}

/// Minimal group-1 render: a bordered conversation pane over a one-line input
/// and a one-line status bar. This is deliberately plain — the two-pane layout,
/// sidebar, markdown, and diffs arrive in groups 4–6.
fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // conversation
            Constraint::Length(3), // input
            Constraint::Length(1), // status
        ])
        .split(frame.area());

    render_conversation(frame, app, chunks[0]);
    render_input(frame, app, chunks[1]);
    render_status(frame, app, chunks[2]);
}

fn render_conversation(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let title = if app.session.model.is_empty() {
        format!(" {} ", strings::brand::NAME)
    } else {
        format!(" {} — {} ", strings::brand::NAME, app.session.model)
    };

    // A pending permission prompt owns the pane (full content, Design §5). The
    // real scrollable prompt is group 7; this shares the strings and the
    // reserved safety styling from now.
    if let Some((_, rendering)) = &app.pending_permission {
        let mut lines = Vec::new();
        if rendering.outside_root {
            lines.push(Line::from(Span::styled(
                strings::permission::OUTSIDE_ROOT_BANNER,
                theme.safety_band(),
            )));
        }
        lines.push(Line::from(vec![
            Span::styled(strings::permission::HEADING, theme.warning()),
            Span::styled(format!(": {}", rendering.summary), theme.primary()),
        ]));
        lines.push(Line::from(Span::styled(
            format!("{}: {}", strings::permission::WHY_LABEL, rendering.reason),
            theme.chrome(),
        )));
        if !rendering.affected_paths.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(
                    "{}: {}",
                    strings::permission::PATHS_LABEL,
                    rendering.affected_paths.join(", ")
                ),
                theme.chrome(),
            )));
        }
        lines.push(Line::from(""));
        for line in rendering.detail.lines() {
            lines.push(Line::from(Span::styled(line.to_string(), theme.primary())));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("[y] {}   ", strings::permission::ALLOW_ONCE),
                theme.success(),
            ),
            Span::styled(
                format!("[s] {}   ", strings::permission::ALLOW_SESSION),
                theme.success(),
            ),
            Span::styled(
                format!("[Enter] {}", strings::permission::DENY),
                theme.error(),
            ),
        ]));
        let prompt = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.dim_accent())
                    .title(format!(" {} ", strings::permission::TITLE)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(prompt, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for item in &app.conversation {
        match item {
            ConvItem::User(text) => lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", strings::markers::USER_PROMPT),
                    theme.accent(),
                ),
                Span::styled(text.clone(), theme.primary()),
            ])),
            ConvItem::Assistant(text) => {
                for l in text.split('\n') {
                    lines.push(Line::from(Span::styled(l.to_string(), theme.primary())));
                }
            }
            ConvItem::Tool {
                tool,
                summary,
                done,
                ..
            } => {
                let (mark, style) = match done {
                    None => (strings::markers::RUNNING, theme.chrome()),
                    Some(true) => (strings::markers::OK, theme.success()),
                    Some(false) => (strings::markers::FAILED, theme.error()),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  [{mark}] "), style),
                    Span::styled(format!("{tool}: {summary}"), theme.chrome()),
                ]));
            }
            ConvItem::Notice(text) => lines.push(Line::from(Span::styled(
                format!("{} {text}", strings::markers::NOTICE),
                theme.chrome(),
            ))),
        }
    }

    let convo = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.chrome())
                .title(Span::styled(title, theme.accent())),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(convo, area);
}

fn render_input(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let input = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{} ", strings::markers::USER_PROMPT),
            theme.accent(),
        ),
        Span::styled(app.input.clone(), theme.primary()),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.chrome()),
    );
    frame.render_widget(input, area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let hints = if app.pending_permission.is_some() {
        strings::hints::PERMISSION
    } else {
        strings::hints::NORMAL
    };
    let status = format!(
        " {mode}  {ctx} {pct}%  {hints}",
        mode = mode_name(app.mode),
        ctx = strings::status::CONTEXT_ABBR,
        pct = app.context_pct,
    );
    let bar = Paragraph::new(status).style(theme.chrome());
    frame.render_widget(bar, area);
}

/// Terse, lower-case mode name for the status bar (Design §6.2).
fn mode_name(mode: emberly_core::Mode) -> &'static str {
    match mode {
        emberly_core::Mode::Normal => strings::mode::NORMAL,
        emberly_core::Mode::AutoAcceptEdits => strings::mode::AUTO_ACCEPT_EDITS,
        emberly_core::Mode::Auto => strings::mode::AUTO,
    }
}
