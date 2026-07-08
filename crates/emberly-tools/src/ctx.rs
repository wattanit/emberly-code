//! [`ToolCtx`] — the execution context handed to every tool (Tech Spec §5.1),
//! and [`TruncateConfig`], the truncation-at-ingestion knobs (Tech Spec §5.3).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::permission::{PermissionGate, PermissionOutcome, PermissionRequest};
use crate::sandbox::Sandbox;

/// Truncation-at-ingestion configuration (Requirements §8.1, Tech Spec §5.3).
/// Carried in [`ToolCtx`]; the truncation function that consumes it lands in
/// group 5. Defaults are placeholders to tune with real use (Tech Spec §16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TruncateConfig {
    /// Truncate when output exceeds this many lines.
    pub max_lines: usize,
    /// …or this many bytes.
    pub max_bytes: usize,
    /// Lines of head to keep.
    pub head_lines: usize,
    /// Lines of tail to keep.
    pub tail_lines: usize,
}

impl Default for TruncateConfig {
    fn default() -> Self {
        Self {
            max_lines: 400,
            max_bytes: 64 * 1024,
            head_lines: 150,
            tail_lines: 100,
        }
    }
}

/// Everything a tool needs to run: the project root (for confinement checks),
/// the truncation config, and the permission gate. Cheap to clone (the gate is
/// an `Arc`).
///
/// Tools route permission through [`authorize`](ToolCtx::authorize) and process
/// spawning through [`sandbox`](ToolCtx::sandbox), so neither the permission
/// layer nor OS confinement changes the tool interface.
#[derive(Clone)]
pub struct ToolCtx {
    project_root: PathBuf,
    truncate: TruncateConfig,
    gate: Arc<dyn PermissionGate>,
    sandbox: Arc<dyn Sandbox>,
}

impl ToolCtx {
    /// Build a context rooted at `project_root`, gated by `gate`, spawning
    /// through `sandbox` (confined or plain — the tool does not care which).
    #[must_use]
    pub fn new(
        project_root: impl Into<PathBuf>,
        truncate: TruncateConfig,
        gate: Arc<dyn PermissionGate>,
        sandbox: Arc<dyn Sandbox>,
    ) -> Self {
        Self {
            project_root: project_root.into(),
            truncate,
            gate,
            sandbox,
        }
    }

    /// The project root all tool actions are confined to.
    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// The active truncation configuration.
    #[must_use]
    pub fn truncate_config(&self) -> TruncateConfig {
        self.truncate
    }

    /// Request permission for an action. The single path to the gate — tools
    /// call this and honor the result; they cannot bypass it.
    pub async fn authorize(&self, request: PermissionRequest) -> PermissionOutcome {
        self.gate.authorize(request).await
    }

    /// The confined-spawn handle: how to run a subprocess under the active OS
    /// confinement (or directly, when degraded). Used by the bash tool.
    #[must_use]
    pub fn sandbox(&self) -> &dyn Sandbox {
        self.sandbox.as_ref()
    }
}
