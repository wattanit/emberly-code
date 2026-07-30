//! The session scratch store (FR-8, T-17, Tech Spec §8.3). Pure over an
//! injected directory path so it is unit-testable against a temp dir.
//!
//! Unlike the memory store (§8.1), a scratch file's real path is derived
//! straight from the session id and project root — there is only ever one
//! scope, always inside the project — so this needs no channel round trip to
//! the engine's own state: writing a scratch file has no side effect on any
//! other engine-owned field (no cached index, no sidebar status to refresh),
//! so the gate can act directly instead of asking the engine loop to act on
//! its behalf.

use std::path::PathBuf;

use async_trait::async_trait;

use emberly_tools::{validate_name, ScratchError, ScratchGate, ScratchOutcome, ScratchRequest};

/// The session's disposable scratch directory
/// (`.agents/scratch/<session-id>/`). Created lazily on the first write, not
/// at construction (Requirements FR-8) — a session that never writes leaves
/// no directory behind.
pub struct ScratchStore {
    dir: PathBuf,
}

impl ScratchStore {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Execute a scratch-write request. The authoritative name guard
    /// (`validate_name`) runs here as defense in depth — the tool validates
    /// first (the HC-4 boundary made explicit there too); this is the belt to
    /// that brace.
    #[must_use]
    pub fn execute(&self, req: &ScratchRequest) -> ScratchOutcome {
        let name = match validate_name(&req.name) {
            Ok(n) => n,
            Err(e) => return ScratchOutcome::Rejected { reason: e.0 },
        };
        let entry_path = self.dir.join(&name);
        // Belt-and-braces over the tool's own guard: assert the resolved path
        // stays under the scratch dir (Tech Spec §8.3), mirroring the memory
        // store's own assertion (§8.1).
        if !entry_path.starts_with(&self.dir) {
            return ScratchOutcome::Rejected {
                reason: "entry path escapes the scratch directory".into(),
            };
        }
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            return ScratchOutcome::Rejected {
                reason: format!("cannot create scratch directory: {e}"),
            };
        }
        match std::fs::write(&entry_path, &req.content) {
            Ok(()) => ScratchOutcome::Written {
                name,
                bytes: req.content.len(),
            },
            Err(e) => ScratchOutcome::Rejected {
                reason: format!("cannot write scratch file: {e}"),
            },
        }
    }
}

#[async_trait]
impl ScratchGate for ScratchStore {
    async fn scratch_write(&self, req: ScratchRequest) -> Result<ScratchOutcome, ScratchError> {
        Ok(self.execute(&req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("emberly-scratch-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn write_creates_the_directory_lazily_and_the_file() {
        let dir = temp_dir();
        assert!(!dir.exists(), "the directory must not exist before a write");
        let store = ScratchStore::new(dir.clone());
        let outcome = store.execute(&ScratchRequest {
            name: "analysis.py".into(),
            content: "print('hi')".into(),
        });
        match outcome {
            ScratchOutcome::Written { name, bytes } => {
                assert_eq!(name, "analysis.py");
                assert_eq!(bytes, "print('hi')".len());
            }
            other => panic!("expected Written, got {other:?}"),
        }
        let written = std::fs::read_to_string(dir.join("analysis.py")).unwrap_or_default();
        assert_eq!(written, "print('hi')");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_replaces_an_existing_file() {
        let dir = temp_dir();
        let store = ScratchStore::new(dir.clone());
        let _ = store.execute(&ScratchRequest {
            name: "notes.md".into(),
            content: "first".into(),
        });
        let _ = store.execute(&ScratchRequest {
            name: "notes.md".into(),
            content: "second".into(),
        });
        let written = std::fs::read_to_string(dir.join("notes.md")).unwrap_or_default();
        assert_eq!(written, "second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_name_is_rejected_without_touching_the_filesystem() {
        let dir = temp_dir();
        let store = ScratchStore::new(dir.clone());
        let outcome = store.execute(&ScratchRequest {
            name: "foo..bar".into(),
            content: "x".into(),
        });
        match outcome {
            ScratchOutcome::Rejected { reason } => assert!(reason.contains("'..'")),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert!(
            !dir.exists(),
            "a rejected write must not create the directory"
        );
    }
}
