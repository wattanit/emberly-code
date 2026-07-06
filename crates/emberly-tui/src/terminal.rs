//! Terminal lifecycle (Tech Spec §9, HC-3). Entering the rich TUI puts the
//! terminal into raw mode + the alternate screen; leaving it — by *any* path,
//! including a panic — must put it back, or the user is left with a broken
//! shell.
//!
//! Two mechanisms guarantee that:
//! - [`TerminalGuard`] restores on `Drop` (normal return and unwinding).
//! - The guard also installs a panic hook that calls [`restore_terminal`]
//!   *before* the previous hook prints, so the panic message and backtrace land
//!   on a usable terminal rather than inside the cleared alternate screen.
//!
//! [`restore_terminal`] is idempotent (every step ignores its error), so the
//! double-call from "panic hook + Drop during unwind" is harmless.

use std::io::{self, Stdout};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// Owns the ratatui terminal and the entered terminal modes. Constructing it
/// enters raw mode + the alternate screen and hides the cursor; dropping it
/// restores everything (HC-3).
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Enter raw mode + the alternate screen, hide the cursor, and enable
    /// bracketed paste. Installs the panic-safe restore hook.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            cursor::Hide
        )?;
        install_panic_hook();
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self { terminal })
    }

    /// The underlying ratatui terminal, for drawing a frame.
    pub fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

/// Restore the terminal to its pre-TUI state: show the cursor, leave the
/// alternate screen and bracketed-paste mode, and disable raw mode. Idempotent
/// and best-effort — every step ignores its own error so this is safe to call
/// from a panic hook, from `Drop`, and from both in succession.
pub fn restore_terminal() -> io::Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        cursor::Show
    );
    let _ = disable_raw_mode();
    Ok(())
}

/// Wrap the current panic hook so the terminal is restored before the existing
/// hook (which prints the calm bug notice + backtrace, see the binary's
/// `install_panic_hook`) runs. Without this the message would be written into
/// the alternate screen and lost when we leave it.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        previous(info);
    }));
}
