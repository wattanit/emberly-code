//! `emberly-tui` — the `ratatui` terminal frontend: panes, sidebar, command
//! palette, diff overlay, permission prompt, and degraded-mode parity
//! (Design Guideline §2–§7, Tech Spec §9).
//!
//! One frontend consuming `UiEvent` and producing `Command`; the engine
//! cannot tell it apart from a future headless frontend (A-1).
//!
//! The line-mode frontend ([`line`]) is the degraded-mode / headless seed; the
//! rich `ratatui` frontend ([`tui`]) is the second implementation over the same
//! channel boundary. [`frontend::detect`] chooses between them.
#![forbid(unsafe_code)]

pub mod app;
pub mod commands;
pub mod diffview;
pub mod edit;
pub mod editor;
pub mod frontend;
pub mod hit;
pub mod line;
pub mod markdown;
pub mod render;
pub mod strings;
pub mod terminal;
pub mod text;
pub mod theme;
pub mod tui;

pub use app::{App, SessionInfo};
pub use editor::LineEditor;
pub use frontend::{detect, FrontendKind};
pub use line::{parse_permission_answer, LineRenderer};
pub use terminal::restore_terminal;
pub use theme::{Palette, Theme};
