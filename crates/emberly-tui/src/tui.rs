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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::Event;
use emberly_core::{resume, Command, FrontendPorts, MemoryOp, SessionId, TranscriptRecord};
use tokio::sync::mpsc;

use crate::app::{Action, App, PendingMemoryEdit, SessionInfo};
use crate::hit::HitMap;
use crate::render;
use crate::terminal::TerminalGuard;

/// Run the rich TUI until the user quits or the engine closes its event stream.
/// Sets up and tears down the terminal via [`TerminalGuard`] (HC-3). `history`
/// seeds the conversation timeline when resuming a session (Tech Spec §3.3).
// The frontend entry point threads several independent session inputs; a
// bundle struct would only move the argument list elsewhere.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    ports: FrontendPorts,
    session: SessionInfo,
    history: Vec<TranscriptRecord>,
    sessions_dir: PathBuf,
    profiles: Vec<String>,
    config_template: String,
    reasoning_view: crate::app::ReasoningView,
    mouse: bool,
    provider_writer: Arc<dyn emberly_core::ProviderProfileWriter>,
) -> io::Result<()> {
    // The single §3.4 capture gate (Tech Spec §9): reaching `tui::run` already
    // means rich mode (degraded runs `line::run`), so the one predicate is
    // `mouse_capture_enabled(true, ui.mouse)`. Off ⇒ no `EnableMouseCapture`,
    // the terminal owns the mouse.
    let capture_mouse = crate::terminal::mouse_capture_enabled(true, mouse);
    let mut guard = TerminalGuard::enter(capture_mouse)?;
    let mut app = App::new(
        session,
        sessions_dir,
        profiles,
        config_template,
        provider_writer,
    );
    // Set the trail view before seeding history so resumed reasoning items
    // render with the configured default (Design §4.4).
    app.timeline.reasoning = reasoning_view;
    app.seed_history(&history);
    app.anim.active = motion_enabled();

    // Shared with the input reader so an `$EDITOR` handoff can pause it (C-5).
    let input_paused = Arc::new(AtomicBool::new(false));
    let mut input_rx = spawn_input_reader(input_paused.clone());
    let mut events_rx = ports.events_rx;
    // The single animation ticker (Design §6.4): ~12fps, and only ever causes a
    // redraw while something is animating, so an idle screen stays quiet.
    let frame_ms = 1000 / u64::try_from(crate::app::ANIM_FPS).unwrap_or(12);
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    // Dropped when this function returns (on quit or engine close), which
    // closes the command channel — the engine then finishes and closes its
    // events. No hard cancel: an in-flight reply is still allowed to complete.
    let commands_tx = ports.commands_tx;

    redraw(&mut guard, &mut app)?;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Advance the animation only when something is actually moving;
                // otherwise skip the redraw so an idle screen (or a permission
                // prompt) stays perfectly still (Design §6.4).
                if app.is_animating() {
                    app.tick();
                    redraw(&mut guard, &mut app)?;
                }
            },
            event = events_rx.recv() => match event {
                Some(event) => {
                    app.apply_event(event);
                    // A memory-body reply for an edit stages a `$EDITOR` handoff
                    // (FR-6, §4.6): run it here, off the input path, then redraw.
                    if let Some(edit) = app.take_pending_memory_edit() {
                        run_memory_edit(
                            &mut app,
                            &commands_tx,
                            &input_paused,
                            &mut guard,
                            edit,
                        )
                        .await?;
                    }
                    redraw(&mut guard, &mut app)?;
                }
                None => break, // engine finished and closed its events
            },
            input = input_rx.recv() => match input {
                Some(Event::Key(key)) if is_press(&key) => {
                    let action = app.on_key(key);
                    if handle_action(action, &mut app, &commands_tx, &input_paused, &mut guard)
                        .await?
                    {
                        break;
                    }
                    redraw(&mut guard, &mut app)?;
                }
                Some(Event::Paste(text)) => {
                    app.on_paste(&text);
                    redraw(&mut guard, &mut app)?;
                }
                Some(Event::Mouse(mouse)) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.on_scroll(true),
                        MouseEventKind::ScrollDown => app.on_scroll(false),
                        // A click is "focus + Enter" on an interactive row
                        // (Design §3.4): it goes through the *same* Action path a
                        // keypress does, so the mouse adds no new authority. Only
                        // an **unmodified** left button is consumed — Shift/Ctrl/
                        // Alt clicks are left to the terminal so its native
                        // selection still works (Design §3.4; group 5 verifies).
                        MouseEventKind::Down(MouseButton::Left) if mouse.modifiers.is_empty() => {
                            let action = app.on_click(mouse.column, mouse.row);
                            if handle_action(
                                action,
                                &mut app,
                                &commands_tx,
                                &input_paused,
                                &mut guard,
                            )
                            .await?
                            {
                                break;
                            }
                        }
                        // Drag/move/right/up and modified clicks: not consumed
                        // (no redraw, so a native Shift-drag selection is smooth).
                        _ => continue,
                    }
                    redraw(&mut guard, &mut app)?;
                }
                Some(Event::Resize(_, _)) => {
                    redraw(&mut guard, &mut app)?;
                }
                Some(_) => {} // focus events — ignored
                None => break, // input thread ended (stdin closed)
            },
        }
    }

    Ok(())
}

/// Draw one frame and store the click hit-map it built on `app` (Design §3.4).
/// A fresh [`HitMap`] is built per frame from the live render geometry, so a
/// subsequent click resolves against exactly what is on screen.
fn redraw(guard: &mut TerminalGuard, app: &mut App) -> io::Result<()> {
    let mut hit_map = HitMap::new();
    let view: &App = app;
    guard
        .terminal()
        .draw(|f| render::frame(f, view, &mut hit_map))?;
    app.hit_map = hit_map;
    Ok(())
}

/// Apply an [`Action`] produced by a key **or** a click — both go through this
/// one path, so the mouse can never do anything the keyboard cannot (the §3.4
/// invariant). Returns `true` when the app should quit.
async fn handle_action(
    action: Action,
    app: &mut App,
    commands_tx: &mpsc::Sender<Command>,
    input_paused: &Arc<AtomicBool>,
    guard: &mut TerminalGuard,
) -> io::Result<bool> {
    match action {
        Action::Quit => return Ok(true),
        Action::Command(cmd) => {
            // A memory mutation (inspector delete/edit-commit) re-emits only
            // MemoryStatus; re-list so the open inspector refreshes its rows
            // (FR-6, group 2 note).
            let refresh_memory = matches!(cmd, Command::MemoryMutate { .. });
            let _ = commands_tx.send(cmd).await;
            if refresh_memory {
                let _ = commands_tx.send(Command::MemoryList).await;
            }
        }
        Action::NewSession => {
            // Mint the id here so the view can update without a round trip; the
            // engine adopts the same id.
            let id = SessionId::new();
            let _ = commands_tx
                .send(Command::NewSession { session_id: id })
                .await;
            app.begin_new_session(id);
        }
        Action::ResumeSession(id) => {
            switch_session(app, commands_tx, id).await;
        }
        Action::EditFile(path) => {
            // Hand the terminal to $EDITOR (C-5): pause the input reader so the
            // editor gets the keystrokes, leave the TUI, run the editor, then
            // restore and redraw.
            input_paused.store(true, Ordering::Relaxed);
            guard.suspend()?;
            let status = crate::edit::run_editor(&path);
            guard.resume()?;
            input_paused.store(false, Ordering::Relaxed);
            let edited = matches!(status, crate::edit::EditStatus::Edited);
            app.note_edit(&path, status);
            // Apply the saved edit to the running session (C-5).
            if edited {
                let _ = commands_tx.send(Command::ReloadConfig).await;
            }
        }
        Action::None => {}
    }
    Ok(false)
}

/// Perform the memory-edit `$EDITOR` handoff (FR-6, §4.6): stage the current
/// body to a temp file, hand the terminal to `$EDITOR`, and — on a saved edit —
/// commit the new body through the engine via `MemoryMutate` (the harness
/// performs the write, never the TUI), then re-list so the inspector refreshes.
/// `description`/`type_` ride through unchanged so a body edit never erases the
/// entry's metadata.
async fn run_memory_edit(
    app: &mut App,
    commands_tx: &mpsc::Sender<Command>,
    input_paused: &Arc<AtomicBool>,
    guard: &mut TerminalGuard,
    edit: PendingMemoryEdit,
) -> io::Result<()> {
    // A stable temp path per entry; the name is sanitized for the filesystem.
    let slug: String = edit
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let path =
        std::env::temp_dir().join(format!("emberly-memory-{}-{slug}.md", std::process::id()));
    if let Err(e) = std::fs::write(&path, &edit.body) {
        app.notice(format!("could not stage memory edit: {e}"));
        return Ok(());
    }
    // Hand the terminal to $EDITOR (mirrors the config/prompt edit path).
    input_paused.store(true, Ordering::Relaxed);
    guard.suspend()?;
    let status = crate::edit::run_editor(&path);
    guard.resume()?;
    input_paused.store(false, Ordering::Relaxed);
    match status {
        crate::edit::EditStatus::Edited => match std::fs::read_to_string(&path) {
            Ok(new_body) => {
                let _ = commands_tx
                    .send(Command::MemoryMutate {
                        op: MemoryOp::Update,
                        scope: edit.scope,
                        name: edit.name.clone(),
                        description: edit.description.clone(),
                        type_: edit.type_.clone(),
                        body: Some(new_body),
                    })
                    .await;
                // Refresh the open inspector list (mutate re-emits MemoryStatus
                // only; the frontend re-lists — group 2 note).
                let _ = commands_tx.send(Command::MemoryList).await;
                app.notice(format!("updated memory: {}", edit.name));
            }
            Err(e) => app.notice(format!("could not read the edited memory entry: {e}")),
        },
        crate::edit::EditStatus::NoEditor => {
            app.notice("no editor configured — set $EDITOR or $VISUAL, then try again");
        }
        crate::edit::EditStatus::Failed(why) => {
            app.notice(format!("editor failed: {why}"));
        }
    }
    let _ = std::fs::remove_file(&path);
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

/// Spawn the terminal-input reader on its own OS thread, forwarding events over
/// a channel. It polls with a short timeout (rather than a bare blocking
/// `read`) so it can be **paused**: while `paused` is set — during an `$EDITOR`
/// handoff (C-5) — the thread reads nothing, leaving the terminal's input to the
/// child editor. The thread ends when the channel closes or the read errors.
fn spawn_input_reader(paused: Arc<AtomicBool>) -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel::<Event>(64);
    std::thread::spawn(move || loop {
        if paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        match crossterm::event::poll(Duration::from_millis(50)) {
            Ok(true) => match crossterm::event::read() {
                Ok(event) => {
                    if tx.blocking_send(event).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            Ok(false) => {} // timeout — loop and re-check `paused`
            Err(_) => break,
        }
    });
    rx
}
