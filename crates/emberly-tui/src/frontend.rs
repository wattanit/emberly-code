//! Frontend selection (Design §7, A-1). Line-mode and the rich `ratatui` TUI
//! are two implementations over the same [`FrontendPorts`]; this module decides
//! which one runs and dispatches to it. The engine cannot tell them apart —
//! the whole point of the channel boundary.
//!
//! Group 1 gives a first-cut degraded predicate (`--plain`, `NO_COLOR`,
//! `TERM=dumb`, non-tty stdout); group 10 finalizes it and makes the line-mode
//! path the tested headless contract.

use std::io::{self, IsTerminal};

use emberly_core::FrontendPorts;

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

/// Decide which frontend to use. `force_plain` is the `--plain` flag.
///
/// Degraded mode wins whenever colour/cursor control is unwanted or
/// unavailable: an explicit flag, `NO_COLOR`, `TERM=dumb`, or a non-terminal
/// stdout (a pipe or file). Otherwise the rich TUI runs.
#[must_use]
pub fn detect(force_plain: bool) -> FrontendKind {
    if force_plain {
        return FrontendKind::Plain;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return FrontendKind::Plain;
    }
    if matches!(std::env::var("TERM"), Ok(term) if term == "dumb") {
        return FrontendKind::Plain;
    }
    if !io::stdout().is_terminal() {
        return FrontendKind::Plain;
    }
    FrontendKind::Rich
}

/// Run the selected frontend to completion over `ports`.
pub async fn run(kind: FrontendKind, ports: FrontendPorts, session: SessionInfo) -> io::Result<()> {
    match kind {
        FrontendKind::Rich => tui::run(ports, session).await,
        FrontendKind::Plain => line::run(ports).await,
    }
}
