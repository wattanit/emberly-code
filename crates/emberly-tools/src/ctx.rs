//! [`ToolCtx`] — the execution context handed to every tool (Tech Spec §5.1),
//! and [`TruncateConfig`], the truncation-at-ingestion knobs (Tech Spec §5.3).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ask_user::{AskUserGate, AskUserOutcome, DeclineAskGate};
use crate::memory::{DropMemoryGate, MemoryError, MemoryGate, MemoryOutcome, MemoryRequest};
use crate::permission::{PermissionGate, PermissionOutcome, PermissionRequest};
use crate::recall::{DeclineRecallGate, RecallGate, RecallOutcome};
use crate::sandbox::Sandbox;
use crate::task_list::{DropTaskListGate, TaskItem, TaskListError, TaskListGate};

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
    /// Whether salient tool-result reduction runs before the size backstop
    /// (FR-2, Tech Spec §5.3/§8). Default `true`; set `false` for raw results.
    pub reduce: bool,
}

impl Default for TruncateConfig {
    fn default() -> Self {
        Self {
            max_lines: 400,
            max_bytes: 64 * 1024,
            head_lines: 150,
            tail_lines: 100,
            reduce: true,
        }
    }
}

/// Default maximum image file size: 5 MiB (Tech Spec §5.2).
const IMAGE_MAX_BYTES_DEFAULT: usize = 5 * 1024 * 1024;

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
    ask: Arc<dyn AskUserGate>,
    recall: Arc<dyn RecallGate>,
    task_list: Arc<dyn TaskListGate>,
    memory: Arc<dyn MemoryGate>,
    /// Whether the active model accepts image input (P-11). `read_image`
    /// checks this to produce the HC-6 unsupported result before encoding.
    vision: bool,
    /// Maximum image file size in bytes (Tech Spec §5.2, default 5 MiB).
    image_max_bytes: usize,
}

impl ToolCtx {
    /// Build a context rooted at `project_root`, gated by `gate`, spawning
    /// through `sandbox` (confined or plain — the tool does not care which).
    /// The ask-user gate defaults to decline; the engine installs a real one
    /// with [`with_ask_gate`](ToolCtx::with_ask_gate).
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
            ask: Arc::new(DeclineAskGate),
            recall: Arc::new(DeclineRecallGate),
            task_list: Arc::new(DropTaskListGate),
            memory: Arc::new(DropMemoryGate),
            vision: false,
            image_max_bytes: IMAGE_MAX_BYTES_DEFAULT,
        }
    }

    /// Install the ask-user gate (T-8). Kept a builder so existing callers and
    /// tests, which never ask, need no change.
    #[must_use]
    pub fn with_ask_gate(mut self, ask: Arc<dyn AskUserGate>) -> Self {
        self.ask = ask;
        self
    }

    /// Install the recall gate (T-10). Kept a builder so existing callers and
    /// tests, which never recall, need no change.
    #[must_use]
    pub fn with_recall_gate(mut self, recall: Arc<dyn RecallGate>) -> Self {
        self.recall = recall;
        self
    }

    /// Install the task-list gate (T-11). Kept a builder so existing callers
    /// and tests, which never set a task list, need no change.
    #[must_use]
    pub fn with_task_list_gate(mut self, task_list: Arc<dyn TaskListGate>) -> Self {
        self.task_list = task_list;
        self
    }

    /// Install the memory gate (T-13). Kept a builder so existing callers and
    /// tests, which never persist memory, need no change.
    #[must_use]
    pub fn with_memory_gate(mut self, memory: Arc<dyn MemoryGate>) -> Self {
        self.memory = memory;
        self
    }

    /// Set whether the active model accepts image input (P-11). Kept a builder
    /// so existing callers and tests default to `false`.
    #[must_use]
    pub fn with_vision(mut self, vision: bool) -> Self {
        self.vision = vision;
        self
    }

    /// Set the maximum image file size in bytes (Tech Spec §5.2).
    #[must_use]
    pub fn with_image_max_bytes(mut self, max_bytes: usize) -> Self {
        self.image_max_bytes = max_bytes;
        self
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

    /// Put a question to the user and block until answered (T-8). The single
    /// path to the ask-user gate; touches no filesystem or network.
    pub async fn ask_user(&self, question: String, options: Vec<String>) -> AskUserOutcome {
        self.ask.ask(question, options).await
    }

    /// Retrieve elided turns from the engine's in-memory conversation (T-10).
    /// The single path to the recall gate; touches no filesystem or network.
    pub async fn recall(&self, from: usize, to: usize) -> RecallOutcome {
        self.recall.recall(from, to).await
    }

    /// Replace the engine's task list with the full list (T-11). The single
    /// path to the task-list gate; touches no filesystem or network, so it is
    /// **not permission-gated** (§6). The model always sends the complete list
    /// (replace, not merge).
    pub async fn set_task_list(
        &self,
        items: Vec<TaskItem>,
    ) -> Result<(), TaskListError> {
        self.task_list.set_task_list(items).await
    }

    /// Read or write a durable memory entry (T-13). The single path to the
    /// memory gate; harness-managed persistence that does not widen HC-4
    /// (FR-6). Not permission-gated.
    pub async fn memory_op(
        &self,
        req: MemoryRequest,
    ) -> Result<MemoryOutcome, MemoryError> {
        self.memory.memory_op(req).await
    }

    /// Whether the active model accepts image input (P-11).
    #[must_use]
    pub fn vision(&self) -> bool {
        self.vision
    }

    /// Maximum image file size in bytes (Tech Spec §5.2).
    #[must_use]
    pub fn image_max_bytes(&self) -> usize {
        self.image_max_bytes
    }
}
