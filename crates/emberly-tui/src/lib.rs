//! `emberly-tui` — the `ratatui` terminal frontend: panes, sidebar, command
//! palette, diff overlay, permission prompt, and degraded-mode parity
//! (Design Guideline §2–§7, Tech Spec §9).
//!
//! One frontend consuming `UiEvent` and producing `Command`; the engine
//! cannot tell it apart from a future headless frontend (A-1). Phase 0: crate
//! skeleton only. The line-mode driver arrives in Phase 1, the full TUI in
//! Phase 4.
#![forbid(unsafe_code)]
