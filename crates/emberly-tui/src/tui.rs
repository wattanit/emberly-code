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
use std::path::PathBuf;

use crossterm::event::Event;
use emberly_core::{resume, Command, FrontendPorts, SessionId, TranscriptRecord};
use tokio::sync::mpsc;

use crate::app::{Action, App, SessionInfo};
use crate::render;
use crate::terminal::TerminalGuard;

/// Run the rich TUI until the user quits or the engine closes its event stream.
/// Sets up and tears down the terminal via [`TerminalGuard`] (HC-3). `history`
/// seeds the conversation timeline when resuming a session (Tech Spec §3.3).
pub async fn run(
    ports: FrontendPorts,
    session: SessionInfo,
    history: Vec<TranscriptRecord>,
    sessions_dir: PathBuf,
) -> io::Result<()> {
    let mut guard = TerminalGuard::enter()?;
    let mut app = App::new(session, sessions_dir);
    app.seed_history(&history);
    app.motion = motion_enabled();

    let mut input_rx = spawn_input_reader();
    let mut events_rx = ports.events_rx;
    // The single animation ticker (Design §6.4): ~12fps, and only ever causes a
    // redraw while something is animating, so an idle screen stays quiet.
    let frame_ms = 1000 / u64::try_from(crate::app::ANIM_FPS).unwrap_or(12);
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    // Dropped when this function returns (on quit or engine close), which
    // closes the command channel — the engine then finishes and closes its
    // events. No hard cancel: an in-flight reply is still allowed to complete.
    let commands_tx = ports.commands_tx;

    guard.terminal().draw(|f| render::frame(f, &app))?;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Advance the animation only when something is actually moving;
                // otherwise skip the redraw so an idle screen (or a permission
                // prompt) stays perfectly still (Design §6.4).
                if app.is_animating() {
                    app.tick();
                    guard.terminal().draw(|f| render::frame(f, &app))?;
                }
            },
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
                        Action::NewSession => {
                            // Mint the id here so the view can update without a
                            // round trip; the engine adopts the same id.
                            let id = SessionId::new();
                            let _ = commands_tx
                                .send(Command::NewSession { session_id: id })
                                .await;
                            app.begin_new_session(id);
                        }
                        Action::ResumeSession(id) => {
                            switch_session(&mut app, &commands_tx, id).await;
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

/// Resume the saved session `id` from the picker: read its transcript, tell the
/// engine to switch, and reseed the view. A read failure leaves the current
/// session untouched and surfaces a calm notice (the engine is not told).
async fn switch_session(app: &mut App, commands_tx: &mpsc::Sender<Command>, id: SessionId) {
    let path = app.sessions_dir.join(format!("{id}.jsonl"));
    match resume::read_records(&path) {
        Ok(loaded) => {
            let title = resume::session_title(&loaded.records).unwrap_or_default();
            let _ = commands_tx
                .send(Command::ResumeSession { session_id: id })
                .await;
            app.begin_resumed_session(id, title, &loaded.records);
        }
        Err(_) => app.notice("could not read that session"),
    }
}

/// Only act on key *presses*. On terminals that report key-release events
/// (Kitty protocol) this avoids double-handling; elsewhere it is a no-op since
/// every event is a press.
fn is_press(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyEventKind;
    key.kind == KeyEventKind::Press
}

/// Motion is on by default; `EMBERLY_MOTION=0`/`false` (or `NO_MOTION`) turns
/// it off (Design §6.4 off-switch). Degraded mode never reaches here — it runs
/// the line frontend, which has no ticker.
fn motion_enabled() -> bool {
    if std::env::var_os("NO_MOTION").is_some() {
        return false;
    }
    !matches!(
        std::env::var("EMBERLY_MOTION").as_deref(),
        Ok("0") | Ok("false")
    )
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
