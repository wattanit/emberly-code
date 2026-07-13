//! The built-in tool suite: read, write, edit, bash (Phase 1) plus glob and
//! grep (T-5, T-6; Phase 2).

use std::sync::Arc;

use crate::registry::ToolRegistry;

mod ask_user;
mod bash;
mod edit;
mod glob;
mod grep;
mod memory;
mod read;
mod read_image;
mod recall;
mod task_list;
mod write;

pub use ask_user::AskUserTool;
pub use bash::BashTool;
pub use edit::EditFileTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use read::ReadFileTool;
pub use read_image::ReadImageTool;
pub use recall::RecallTool;
pub use task_list::TaskListTool;
pub use memory::MemoryTool;
pub use write::WriteFileTool;

/// A registry with all built-in tools registered (bash with defaults).
#[must_use]
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(ReadImageTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(EditFileTool));
    registry.register(Arc::new(BashTool::default()));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(GrepTool));
    registry.register(Arc::new(AskUserTool));
    registry.register(Arc::new(RecallTool));
    registry.register(Arc::new(TaskListTool::new()));
    registry.register(Arc::new(MemoryTool::new()));
    registry
}
