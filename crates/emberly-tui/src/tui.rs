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
                Some(Event::Paste(text)) => {
                    app.on_paste(&text);
                    guard.terminal().draw(|f| render(f, &app))?;
                }
                Some(Event::Resize(_, _)) => {
                    guard.terminal().draw(|f| render(f, &app))?;
                }
                Some(_) => {} // mouse / focus — handled in later groups
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

/// Input box grows with content up to this many text rows before scrolling.
const MAX_INPUT_ROWS: usize = 6;

/// Draw one frame: a conversation pane over the (grapheme-aware) input editor
/// and a status bar. The two-pane sidebar layout, markdown, and diffs arrive in
/// groups 4–6.
fn render(frame: &mut Frame, app: &App) {
    // The input box grows with the number of logical lines (up to a cap), then
    // scrolls internally — so a long multi-line prompt is visible without
    // permanently stealing the conversation's space.
    let input_rows = app.editor.line_count().clamp(1, MAX_INPUT_ROWS);
    let input_height = u16::try_from(input_rows).unwrap_or(1).saturating_add(2); // + borders

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),               // conversation
            Constraint::Length(input_height), // input
            Constraint::Length(1),            // status
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

/// The prompt gutter width (`"› "`), reserved on every input row so cursor
/// math and multi-line alignment share one offset.
const GUTTER: u16 = 2;

fn render_input(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width <= GUTTER || inner.height == 0 {
        return;
    }
    let text_width = usize::from(inner.width - GUTTER);
    let rows = usize::from(inner.height);

    let (cursor_row, cursor_col) = app.editor.cursor_row_col();
    let lines: Vec<&str> = app.editor.lines().collect();

    // Vertical scroll: keep the cursor's logical row visible.
    let top = cursor_row.saturating_sub(rows.saturating_sub(1));
    // Horizontal scroll: applied only to the cursor's row so its caret stays in
    // view; other rows render from column 0 (clipped to the width).
    let h_scroll = cursor_col.saturating_sub(text_width.saturating_sub(1));

    for (screen_row, line_idx) in (top..top + rows).enumerate() {
        let Some(line) = lines.get(line_idx) else {
            break;
        };
        let start_col = if line_idx == cursor_row { h_scroll } else { 0 };
        let visible = crate::text::slice_cols(line, start_col, text_width);
        let y = inner.y + u16::try_from(screen_row).unwrap_or(0);

        // The ember prompt marker on the first logical row only; a blank gutter
        // keeps continuation rows aligned under the text.
        let gutter = if line_idx == 0 {
            Span::styled(
                format!("{} ", strings::markers::USER_PROMPT),
                theme.accent(),
            )
        } else {
            Span::raw("  ")
        };
        let row_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                gutter,
                Span::styled(visible, theme.primary()),
            ])),
            row_area,
        );
    }

    // Place (and thereby show) the terminal cursor at the caret, accounting for
    // the gutter and any horizontal scroll on the cursor's row.
    if cursor_row >= top && cursor_row < top + rows {
        let screen_row = u16::try_from(cursor_row - top).unwrap_or(0);
        let col_in_view = cursor_col.saturating_sub(h_scroll);
        let x = inner.x + GUTTER + u16::try_from(col_in_view).unwrap_or(0);
        frame.set_cursor_position((x.min(inner.x + inner.width - 1), inner.y + screen_row));
    }
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
