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

pub mod ask_user;
pub mod builtin;
pub mod ctx;
pub mod diff;
pub mod memory;
pub mod path;
pub mod permission;
pub mod recall;
pub mod reduce;
pub mod registry;
pub mod sandbox;
pub mod search;
pub mod skills;
pub mod task_list;
pub mod tool;
pub mod truncate;

pub use ask_user::{AskUserGate, AskUserOutcome};
pub use builtin::{
    default_registry, AskUserTool, BashTool, EditFileTool, GlobTool, GrepTool, MemoryTool,
    ReadFileTool, ReadImageTool, RecallTool, SkillTool, TaskListTool, WebSearchTool, WriteFileTool,
};
pub use ctx::{ToolCtx, TruncateConfig};
pub use memory::{
    slug, MemoryError, MemoryGate, MemoryOp, MemoryOutcome, MemoryRequest, MemoryScope,
};
pub use permission::{PermissionGate, PermissionOutcome, PermissionRequest};
pub use recall::{RecallGate, RecallOutcome};
pub use reduce::{reduce_output, Reduction};
pub use registry::ToolRegistry;
pub use sandbox::{BashInvocation, PlainSandbox, Sandbox};
pub use search::{SearchAuth, SearchClient, SearchError, SearchResult};
pub use skills::{SkillError, SkillGate, SkillInvocation, SkillMeta, SkillOrigin};
pub use task_list::{TaskItem, TaskListError, TaskListGate, TaskStatus};
pub use tool::{FileChange, ImageContent, Tool, ToolOutcome, ToolSpec};
pub use truncate::{truncate_output, Truncation};
