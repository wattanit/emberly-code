//! The memory gate as seen by tools (T-13, FR-6, Tech Spec §5.2/§8.1).
//!
//! The `memory` tool lets the model persist durable facts that survive across
//! sessions. It is **harness-managed persistence** (like the transcript and
//! trust store, not an agent filesystem write) — the model supplies content
//! fields, never a path; the engine derives the filename and performs the write
//! within its own store. It does not widen HC-4 and is **not permission-gated**
//! (§6, FR-6 honesty clause). Project-scope memory loads **only under a trusted
//! root** (FR-1, Tech Spec §6.7).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Which store a memory entry lives in (Tech Spec §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// User-global (`~/.config/emberly/memory/`), always loaded.
    User,
    /// Project-scoped (`.agents/memory/`), loaded only under a trusted root.
    Project,
}

/// What the model asks the engine to do with a memory entry (Tech Spec §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOp {
    /// Create a new entry (fails if it already exists).
    Write,
    /// Update an existing entry (create-or-overwrite).
    Update,
    /// Delete an entry.
    Remove,
    /// Retrieve an entry's body.
    Recall,
}

/// A memory tool request — **content fields, never a path** (HC-4 boundary).
/// The engine derives the filename from `name` via [`slug`] within the fixed
/// scope directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRequest {
    pub op: MemoryOp,
    pub scope: MemoryScope,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// The outcome the engine returns to the tool after a memory op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryOutcome {
    /// A write/update/remove succeeded. The counts refresh the sidebar
    /// (`MemoryStatus`).
    Written { user: usize, project: usize },
    /// A recall returned an entry body (or `None` when the entry is missing).
    Recalled {
        body: Option<String>,
        origin: MemoryScope,
    },
    /// The op was rejected (e.g. project scope unavailable on an untrusted
    /// root, or a name failed validation). HC-6 data, never a panic.
    Rejected { reason: String },
}

/// The memory gate is unreachable (engine gone). Fail-closed: the tool maps
/// this to a [`ToolOutcome::failure`](crate::tool::ToolOutcome::failure)
/// (HC-6), never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryError;

/// The gate the `memory` tool calls to read and write durable memory entries
/// (T-13, Tech Spec §5.2/§8.1). Implemented by the engine; injected into
/// [`ToolCtx`](crate::ctx::ToolCtx). The model supplies content fields, never a
/// path — the engine derives the filename and performs the write.
#[async_trait]
pub trait MemoryGate: Send + Sync {
    /// Execute a memory request. Returns the outcome on success, or `Err` if
    /// the engine is unreachable (fail-closed, HC-6).
    async fn memory_op(&self, req: MemoryRequest) -> Result<MemoryOutcome, MemoryError>;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// rejects every request. The engine replaces it with a real channel-backed
/// gate via [`with_memory_gate`](crate::ctx::ToolCtx::with_memory_gate);
/// contexts that never persist memory (most tests) keep this safe no-op.
pub(crate) struct DropMemoryGate;

#[async_trait]
impl MemoryGate for DropMemoryGate {
    async fn memory_op(&self, _req: MemoryRequest) -> Result<MemoryOutcome, MemoryError> {
        Err(MemoryError)
    }
}

// ---- name → slug + validation (the HC-4 boundary, made testable) ----------

/// An error from [`slug`] when a name is invalid (HC-4 boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryNameError(pub String);

/// Slugify a memory entry name and reject path-escape attempts (Tech Spec §8.1).
///
/// Rejects an empty name, any path separator (`/`, `\`), `..`, and
/// absolute-path markers *before* slugging. Then lowercases and replaces runs
/// of non-`[a-z0-9]` with `-`, trimming. The model cannot escape the scope dir
/// through the name field.
///
/// This is a pure function with no filesystem — unit-testable.
pub fn slug(name: &str) -> Result<String, MemoryNameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(MemoryNameError("name must not be empty".into()));
    }
    // Reject anything that looks like a path escape, before slugging.
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(MemoryNameError(
            "name must not contain path separators".into(),
        ));
    }
    if trimmed == ".." || trimmed.starts_with("..") || trimmed.contains("..") {
        return Err(MemoryNameError("name must not contain '..'".into()));
    }
    if trimmed.starts_with('~') {
        return Err(MemoryNameError("name must not be a home path".into()));
    }
    // Slug: lowercase, replace non-[a-z0-9] runs with `-`, trim.
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else {
            if !prev_dash {
                slug.push('-');
                prev_dash = true;
            }
        }
    }
    // Trim trailing dashes.
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        return Err(MemoryNameError(
            "name must contain at least one alphanumeric character".into(),
        ));
    }
    Ok(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_normalizes_a_simple_name() {
        assert_eq!(slug("My Memory").unwrap_or_default(), "my-memory");
        assert_eq!(slug("API Keys").unwrap_or_default(), "api-keys");
    }

    #[test]
    fn slug_rejects_path_separators() {
        assert!(slug("a/b").is_err());
        assert!(slug("a\\b").is_err());
    }

    #[test]
    fn slug_rejects_dot_dot() {
        assert!(slug("..").is_err());
        assert!(slug("../etc/passwd").is_err());
        assert!(slug("foo..bar").is_err());
    }

    #[test]
    fn slug_rejects_empty() {
        assert!(slug("").is_err());
        assert!(slug("   ").is_err());
    }

    #[test]
    fn slug_rejects_absolute_paths() {
        assert!(slug("/abs").is_err());
        assert!(slug("~/home").is_err());
    }
}
