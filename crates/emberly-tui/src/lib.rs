//! `emberly-tui` — the `ratatui` terminal frontend: panes, sidebar, command
//! palette, diff overlay, permission prompt, and degraded-mode parity
//! (Design Guideline §2–§7, Tech Spec §9).
//!
//! One frontend consuming `UiEvent` and producing `Command`; the engine
//! cannot tell it apart from a future headless frontend (A-1).
//!
//! Phase 1 (current): the line-mode frontend ([`line`]) — plain, append-only,
//! the degraded-mode seed. The full `ratatui` TUI arrives in Phase 4.
#![forbid(unsafe_code)]

pub mod line;

pub use line::{parse_permission_answer, LineRenderer};
