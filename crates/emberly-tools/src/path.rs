//! Path normalization and project-root confinement for the file tools
//! (Tech Spec §5.2). Symlinks in the existing portion of a path are resolved
//! (so a symlink pointing outside the root is caught), `..`/`.` are collapsed,
//! and the result is classified as inside or outside the root.
//!
//! In Phase 1 (no OS sandbox) this is the tool-layer enforcement of HC-4/HC-5;
//! Phase 2 adds the kernel-level fence beneath it. Outside-root access is not
//! hard-blocked here — it is flagged so the permission gate can require an
//! explicit per-action approval (HC-4).

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// A path resolved against the project root.
pub struct ResolvedPath {
    /// Absolute, normalized path (symlinks resolved where the path exists).
    pub path: PathBuf,
    /// True if the resolved path lies outside the project root.
    pub outside_root: bool,
}

/// Collapse `.` and `..` lexically, without touching the filesystem (so it
/// works for not-yet-created files).
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the longest existing ancestor of `path` (resolving symlinks),
/// then re-attach the not-yet-existing tail. Lets us confine both existing
/// files (read/edit) and paths to be created (write).
fn canonicalize_longest_existing(path: &Path) -> Result<PathBuf, String> {
    let mut tail: Vec<OsString> = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(real) = std::fs::canonicalize(&current) {
            let mut result = real;
            for part in tail.iter().rev() {
                result.push(part);
            }
            return Ok(result);
        }
        let Some(name) = current.file_name().map(OsString::from) else {
            return Err(format!("cannot resolve path: {}", path.display()));
        };
        tail.push(name);
        let Some(parent) = current.parent().map(Path::to_path_buf) else {
            return Err(format!("cannot resolve path: {}", path.display()));
        };
        current = parent;
    }
}

/// Resolve `requested` (relative to `root`, or absolute) into an absolute,
/// normalized path and classify whether it escapes the root.
pub fn resolve_in_root(root: &Path, requested: &str) -> Result<ResolvedPath, String> {
    if requested.is_empty() {
        return Err("path must not be empty".to_string());
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| format!("cannot access project root {}: {e}", root.display()))?;

    let requested_path = Path::new(requested);
    let joined = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        canonical_root.join(requested_path)
    };

    let normalized = lexical_normalize(&joined);
    let resolved = canonicalize_longest_existing(&normalized)?;
    let outside_root = !resolved.starts_with(&canonical_root);

    Ok(ResolvedPath {
        path: resolved,
        outside_root,
    })
}

/// Whether the resolved path has any `.git` component — file tools hard-refuse
/// these (HC-5), at the tool layer, independent of the OS sandbox.
#[must_use]
pub fn is_under_git_dir(resolved: &Path) -> bool {
    resolved
        .components()
        .any(|c| matches!(c, Component::Normal(name) if name == ".git"))
}

/// Display a resolved path relative to the root when possible, for summaries.
#[must_use]
pub fn display_relative(root: &Path, resolved: &Path) -> String {
    match std::fs::canonicalize(root) {
        Ok(canonical_root) => resolved
            .strip_prefix(&canonical_root)
            .unwrap_or(resolved)
            .display()
            .to_string(),
        Err(_) => resolved.display().to_string(),
    }
}
