//! The built-in tool suite. Phase 1 ships read/write/edit/bash; glob and grep
//! (T-5, T-6) arrive in Phase 2.

use std::sync::Arc;

use crate::registry::ToolRegistry;

mod bash;
mod edit;
mod read;
mod write;

pub use bash::BashTool;
pub use edit::EditFileTool;
pub use read::ReadFileTool;
pub use write::WriteFileTool;

/// A registry with the Phase 1 built-in tools registered (bash with defaults).
#[must_use]
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool));
    registry.register(Arc::new(WriteFileTool));
    registry.register(Arc::new(EditFileTool));
    registry.register(Arc::new(BashTool::default()));
    registry
}
