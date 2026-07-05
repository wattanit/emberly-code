//! [`ToolRegistry`] — the set of tools available to a session. The engine
//! iterates it to build the provider tool schemas and to dispatch a tool call
//! by name.

use std::collections::HashMap;
use std::sync::Arc;

use crate::tool::{Tool, ToolSpec};

/// A name-indexed collection of tools. Cheap to clone (tools are behind
/// `Arc`).
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool under its own [`ToolSpec::name`]. A later registration
    /// with the same name replaces the earlier one.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        let name = tool.spec().name;
        self.tools.insert(name, tool);
        self
    }

    /// Look up a tool by name (e.g. to dispatch a tool call).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// The specs of all registered tools, for advertising to the provider.
    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec()).collect()
    }

    /// The names of all registered tools.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
