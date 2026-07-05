//! `emberly-tools` — the [`Tool`] trait and the built-in tool suite:
//! read, write, edit, bash, glob, grep (Requirements §5, Tech Spec §5).
//!
//! Transport-agnostic by design so a future MCP adapter implements `Tool`
//! without engine changes (T-7).
//!
//! Phase 1 (current): the trait, [`ToolOutcome`] (structured success/failure,
//! HC-6), the [`ToolCtx`] execution context with its [`PermissionGate`], the
//! [`ToolRegistry`], and the built-in read/write/edit/bash tools. Glob and
//! grep (T-5, T-6) arrive in Phase 2.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod builtin;
pub mod ctx;
pub mod diff;
pub mod path;
pub mod permission;
pub mod registry;
pub mod tool;
pub mod truncate;

pub use builtin::{default_registry, BashTool, EditFileTool, ReadFileTool, WriteFileTool};
pub use ctx::{ToolCtx, TruncateConfig};
pub use permission::{PermissionGate, PermissionOutcome, PermissionRequest};
pub use registry::ToolRegistry;
pub use tool::{FileChange, Tool, ToolOutcome, ToolSpec};
pub use truncate::{truncate_output, Truncation};
