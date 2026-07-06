//! The rich `ratatui` frontend driver (Tech Spec §9). Owns the terminal
//! lifecycle ([`crate::terminal`]) and runs the event loop; frame rendering
//! lives in [`crate::render`].
//!
//! The loop `select!`s over engine [`UiEvent`]s and terminal input events (and,
//! from group 9, an animation tick) and never lets any one block the others:
//! input handling is independent of streaming, and a redraw never delays a
//! command. Input is read on a dedicated OS thread because
//! `crossterm::event::read` is blocking; it forwards events over a channel,
//! mirroring the line-mode stdin reader.

use std::io;

use crossterm::event::Event;
use emberly_core::FrontendPorts;
use tokio::sync::mpsc;

use crate::app::{Action, App, SessionInfo};
use crate::render;
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

    guard.terminal().draw(|f| render::frame(f, &app))?;

    loop {
        tokio::select! {
            event = events_rx.recv() => match event {
                Some(event) => {
                    app.apply_event(event);
                    guard.terminal().draw(|f| render::frame(f, &app))?;
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
                    guard.terminal().draw(|f| render::frame(f, &app))?;
                }
                Some(Event::Paste(text)) => {
                    app.on_paste(&text);
                    guard.terminal().draw(|f| render::frame(f, &app))?;
                }
                Some(Event::Mouse(mouse)) => {
                    use crossterm::event::MouseEventKind;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.on_scroll(true),
                        MouseEventKind::ScrollDown => app.on_scroll(false),
                        _ => continue,
                    }
                    guard.terminal().draw(|f| render::frame(f, &app))?;
                }
                Some(Event::Resize(_, _)) => {
                    guard.terminal().draw(|f| render::frame(f, &app))?;
                }
                Some(_) => {} // focus events — ignored
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
