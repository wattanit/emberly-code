//! `emberly-tools` — the [`Tool`] trait and the built-in tool suite:
//! read, write, edit, bash, glob, grep (Requirements §5, Tech Spec §5).
//!
//! Transport-agnostic by design so a future MCP adapter implements `Tool`
//! without engine changes (T-7).
//!
//! The trait, [`ToolOutcome`] (structured success/failure, HC-6), the
//! [`ToolCtx`] execution context with its [`PermissionGate`], the
//! [`ToolRegistry`], and the built-in read/write/edit/bash/glob/grep tools
//! (T-5, T-6).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod ask_user;
pub mod builtin;
pub mod ctx;
pub mod diff;
pub mod image;
pub mod memory;
pub mod path;
pub mod permission;
pub mod recall;
pub mod reduce;
pub mod registry;
pub mod sandbox;
pub mod scratch;
pub mod search;
pub mod skills;
pub mod subagent;
pub mod task_list;
pub mod tool;
pub mod truncate;

pub use ask_user::{AskUserGate, AskUserOutcome};
pub use builtin::{
    default_registry, AskUserTool, BashTool, EditFileTool, EndAgentTool, GlobTool, GrepTool,
    ListAgentsTool, MemoryTool, MessageAgentTool, ReadDocumentTool, ReadFileTool, ReadImageTool,
    RecallTool, ScratchWriteTool, SkillTool, SpawnAgentsTool, TaskListTool, WebSearchTool,
    WriteFileTool, DEFAULT_ENV_ALLOWLIST, DEFAULT_TIMEOUT_SECS,
};
pub use ctx::{ToolCtx, TruncateConfig};
pub use image::{encode_image_bytes, EncodedImage};
pub use memory::{
    slug, MemoryError, MemoryGate, MemoryOp, MemoryOutcome, MemoryRequest, MemoryScope,
};
pub use permission::{PermissionGate, PermissionOutcome, PermissionRequest};
pub use recall::{RecallGate, RecallOutcome};
pub use reduce::{reduce_output, Reduction};
pub use registry::ToolRegistry;
pub use sandbox::{BashInvocation, PlainSandbox, Sandbox};
pub use scratch::{
    validate_name, ScratchError, ScratchGate, ScratchNameError, ScratchOutcome, ScratchRequest,
};
pub use search::{SearchAuth, SearchClient, SearchError, SearchResult};
pub use skills::{SkillError, SkillGate, SkillInvocation, SkillMeta, SkillOrigin};
pub use subagent::{
    SubagentEndOutcome, SubagentError, SubagentGate, SubagentListEntry, SubagentMessageOutcome,
    SubagentMessageRequest, SubagentSpawnBatch, SubagentSpawnOutcome, SubagentSpawnResult,
    SubagentSpawnSpec, SubagentStatus,
};
pub use task_list::{TaskItem, TaskListError, TaskListGate, TaskStatus};
pub use tool::{FileChange, ImageContent, Tool, ToolOutcome, ToolSpec};
pub use truncate::{truncate_output, Truncation};
