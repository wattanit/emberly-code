//! The built-in tool suite: read, write, edit, and bash, plus glob and grep
//! (T-5, T-6).

use std::sync::Arc;

use crate::registry::ToolRegistry;

mod ask_user;
mod bash;
mod edit;
mod end_agent;
mod glob;
mod grep;
mod list_agents;
mod memory;
mod message_agent;
mod read;
mod read_document;
mod read_image;
mod recall;
mod scratch;
mod skill;
mod spawn_agents;
mod task_list;
mod web_search;
mod write;

pub use ask_user::AskUserTool;
pub use bash::{BashTool, DEFAULT_ENV_ALLOWLIST, DEFAULT_TIMEOUT_SECS};
pub use edit::EditFileTool;
pub use end_agent::EndAgentTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use list_agents::ListAgentsTool;
pub use memory::MemoryTool;
pub use message_agent::MessageAgentTool;
pub use read::ReadFileTool;
pub use read_document::ReadDocumentTool;
pub use read_image::ReadImageTool;
pub use recall::RecallTool;
pub use scratch::ScratchWriteTool;
pub use skill::SkillTool;
pub use spawn_agents::SpawnAgentsTool;
pub use task_list::TaskListTool;
pub use web_search::WebSearchTool;
pub use write::WriteFileTool;

/// A registry with all built-in tools registered (bash with defaults).
#[must_use]
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(ReadImageTool));
    registry.register(Arc::new(ReadDocumentTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(EditFileTool));
    registry.register(Arc::new(BashTool::default()));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(GrepTool));
    registry.register(Arc::new(AskUserTool));
    registry.register(Arc::new(RecallTool));
    registry.register(Arc::new(TaskListTool::new()));
    registry.register(Arc::new(MemoryTool::new()));
    registry.register(Arc::new(SkillTool::new()));
    registry.register(Arc::new(ScratchWriteTool::new()));
    registry.register(Arc::new(SpawnAgentsTool::new()));
    registry.register(Arc::new(MessageAgentTool::new()));
    registry.register(Arc::new(ListAgentsTool::new()));
    registry.register(Arc::new(EndAgentTool::new()));
    registry
}
