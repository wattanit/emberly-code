//! `emberly-tools` — the [`Tool`] trait and the built-in tool suite:
//! read, write, edit, bash, glob, grep (Requirements §5, Tech Spec §5).
//!
//! Transport-agnostic by design so a future MCP adapter implements `Tool`
//! without engine changes (T-7).
//!
//! Phase 1, group 3 (current): the trait, [`ToolOutcome`] (structured
//! success/failure, HC-6), the [`ToolCtx`] execution context with its
//! [`PermissionGate`], and the [`ToolRegistry`]. The tools themselves land in
//! group 4 (read/write/edit/bash) and Phase 2 (glob/grep).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod ctx;
pub mod permission;
pub mod registry;
pub mod tool;

pub use ctx::{ToolCtx, TruncateConfig};
pub use permission::{PermissionGate, PermissionOutcome, PermissionRequest};
pub use registry::ToolRegistry;
pub use tool::{Tool, ToolOutcome, ToolSpec};
