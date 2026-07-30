//! The scratch gate as seen by tools (T-17, FR-8, Tech Spec §5.2/§8.3).
//!
//! The `scratch_write` tool lets the model stash a temporary file — a script,
//! intermediate output, or working note — in its session's disposable working
//! space. It is **harness-managed persistence** (like the transcript and
//! memory store, not an agent filesystem write) — the model supplies content
//! fields, never a path; the engine derives the real path within the fixed
//! per-session scratch directory and performs the write. It does not widen
//! HC-4 and is **not permission-gated** (§6, FR-8 honesty clause): the call
//! and its result are still ordinary transcript events under HC-7, like any
//! other tool call — only the permission-ask gate is skipped.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A scratch-write request — **content fields, never a path** (HC-4
/// boundary). The engine resolves `name` to a file within the session's
/// fixed scratch directory after validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchRequest {
    pub name: String,
    pub content: String,
}

/// The outcome the engine returns to the tool after a scratch write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScratchOutcome {
    /// The write succeeded.
    Written { name: String, bytes: usize },
    /// The request was rejected (e.g. an invalid name, or the write failed).
    /// HC-6 data, never a panic.
    Rejected { reason: String },
}

/// The scratch gate is unreachable (engine gone). Fail-closed: the tool maps
/// this to a [`ToolOutcome::failure`](crate::tool::ToolOutcome::failure)
/// (HC-6), never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchError;

/// The gate the `scratch_write` tool calls to write into the session's
/// scratch directory (T-17, Tech Spec §5.2/§8.3). Implemented by the engine;
/// injected into [`ToolCtx`](crate::ctx::ToolCtx). The model supplies content
/// fields, never a path — the engine derives the filename and performs the
/// write.
#[async_trait]
pub trait ScratchGate: Send + Sync {
    /// Execute a scratch-write request. Returns the outcome on success, or
    /// `Err` if the engine is unreachable (fail-closed, HC-6).
    async fn scratch_write(&self, req: ScratchRequest) -> Result<ScratchOutcome, ScratchError>;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// rejects every request. The engine replaces it with a real gate via
/// [`with_scratch_gate`](crate::ctx::ToolCtx::with_scratch_gate); contexts
/// that never write scratch files (most tests) keep this safe no-op.
pub(crate) struct DropScratchGate;

#[async_trait]
impl ScratchGate for DropScratchGate {
    async fn scratch_write(&self, _req: ScratchRequest) -> Result<ScratchOutcome, ScratchError> {
        Err(ScratchError)
    }
}

// ---- name validation (the HC-4 boundary, made testable) -------------------

/// An error from [`validate_name`] when a name is invalid (HC-4 boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchNameError(pub String);

/// Validate a scratch file name and reject path-escape attempts (Tech Spec
/// §8.3): an empty name, any path separator (`/`, `\`), `..`, or a leading
/// `~` — the same security boundary as the memory tool's [`slug`](crate::memory::slug)
/// guard. Unlike `slug`, this does **not** transform the name: a scratch
/// file's extension and case are meaningful (`analysis.py` must stay
/// `analysis.py`, never `analysis-py`) and must survive verbatim.
///
/// A pure function with no filesystem — unit-testable.
pub fn validate_name(name: &str) -> Result<String, ScratchNameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ScratchNameError("name must not be empty".into()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(ScratchNameError(
            "name must not contain path separators".into(),
        ));
    }
    if trimmed == ".." || trimmed.starts_with("..") || trimmed.contains("..") {
        return Err(ScratchNameError("name must not contain '..'".into()));
    }
    if trimmed.starts_with('~') {
        return Err(ScratchNameError("name must not be a home path".into()));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_preserves_case_and_extension() {
        // Unlike memory's slug, a scratch name is not transformed.
        assert_eq!(
            validate_name("Analysis.py").unwrap_or_default(),
            "Analysis.py"
        );
        assert_eq!(validate_name("notes.md").unwrap_or_default(), "notes.md");
    }

    #[test]
    fn validate_name_rejects_path_separators() {
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
    }

    #[test]
    fn validate_name_rejects_dot_dot() {
        assert!(validate_name("..").is_err());
        assert!(validate_name("../etc/passwd").is_err());
        assert!(validate_name("foo..bar").is_err());
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
    }

    #[test]
    fn validate_name_rejects_home_paths() {
        assert!(validate_name("~/home").is_err());
    }
}
