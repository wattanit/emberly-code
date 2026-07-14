//! Frontend selection (Design §7, A-1). Line-mode and the rich `ratatui` TUI
//! are two implementations over the same [`FrontendPorts`]; this module decides
//! which one runs and dispatches to it. The engine cannot tell them apart —
//! the whole point of the channel boundary.
//!
//! The degraded predicate (`--plain`, `NO_COLOR`, `TERM=dumb`, non-tty stdout)
//! is split into a pure [`decide`] (unit-tested) and [`detect`] (which gathers
//! the environment). Line mode is a supported, tested configuration and the
//! contract for a future headless frontend (Design §7).

use std::io::{self, IsTerminal};

use emberly_core::{FrontendPorts, TranscriptRecord};

use crate::app::SessionInfo;
use crate::{line, tui};

/// Which frontend to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendKind {
    /// The full `ratatui` two-pane interface.
    Rich,
    /// Line-oriented, append-only, no cursor repositioning — degraded mode and
    /// the future headless contract (Design §7).
    Plain,
}

/// Decide which frontend to use from the already-gathered inputs (pure).
///
/// Degraded mode wins whenever colour/cursor control is unwanted or
/// unavailable: the `--plain` flag, `NO_COLOR`, `TERM=dumb`, or a non-terminal
/// stdout (a pipe or file). Otherwise the rich TUI runs.
#[must_use]
pub fn decide(
    force_plain: bool,
    no_color: bool,
    term_dumb: bool,
    stdout_is_tty: bool,
) -> FrontendKind {
    if force_plain || no_color || term_dumb || !stdout_is_tty {
        FrontendKind::Plain
    } else {
        FrontendKind::Rich
    }
}

/// Detect the frontend from the process environment. `force_plain` is the
/// `--plain` flag.
#[must_use]
pub fn detect(force_plain: bool) -> FrontendKind {
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let term_dumb = matches!(std::env::var("TERM"), Ok(term) if term == "dumb");
    decide(force_plain, no_color, term_dumb, io::stdout().is_terminal())
}

/// Run the selected frontend to completion over `ports`. `history` seeds the
/// timeline when resuming (Tech Spec §3.3); it is empty for a fresh session.
// Threads several independent session inputs; a bundle struct would only move
// the argument list elsewhere.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    kind: FrontendKind,
    ports: FrontendPorts,
    session: SessionInfo,
    history: Vec<TranscriptRecord>,
    sessions_dir: std::path::PathBuf,
    profiles: Vec<String>,
    config_template: String,
    reasoning: Option<String>,
    mouse: bool,
) -> io::Result<()> {
    let reasoning_view = crate::app::ReasoningView::parse(reasoning.as_deref().unwrap_or(""));
    match kind {
        FrontendKind::Rich => {
            tui::run(
                ports,
                session,
                history,
                sessions_dir,
                profiles,
                config_template,
                reasoning_view,
                mouse,
            )
            .await
        }
        // Line mode notes the resumed-event count in its banner (see the
        // binary); it does not replay the timeline or offer the picker, but it
        // supports `/config`, `/prompt`, `/reload`, and `/effort` (C-5, P-9).
        FrontendKind::Plain => {
            line::run(ports, sessions_dir, config_template, reasoning_view).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_only_on_a_tty_with_no_overrides() {
        assert_eq!(decide(false, false, false, true), FrontendKind::Rich);
    }

    #[test]
    fn any_degraded_signal_forces_plain() {
        // --plain, NO_COLOR, TERM=dumb, and non-tty each force line mode.
        assert_eq!(decide(true, false, false, true), FrontendKind::Plain);
        assert_eq!(decide(false, true, false, true), FrontendKind::Plain);
        assert_eq!(decide(false, false, true, true), FrontendKind::Plain);
        assert_eq!(decide(false, false, false, false), FrontendKind::Plain);
    }
}
