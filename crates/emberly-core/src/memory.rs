//! The persistent memory store (FR-6, T-13, Tech Spec §8.1). Pure over an
//! injected directory path so it is unit-testable against a temp dir.
//!
//! Entry files are `<slug>.md` with TOML frontmatter (`+++` fence) holding
//! `name`, `description`, and optional `type`, followed by a markdown body. The
//! index is `<dir>/MEMORY.md` — one line per entry (`- <name> — <description>`).
//! No YAML dependency (HC-2); the existing `toml` crate parses the frontmatter.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use emberly_tools::memory::{MemoryOp, MemoryOutcome, MemoryRequest, MemoryScope, slug};

/// The metadata stored in TOML frontmatter (Tech Spec §8.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMeta {
    pub name: String,
    pub description: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
}

/// One entry on disk: metadata + markdown body.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub meta: EntryMeta,
    pub body: String,
}

/// The durable memory store. `project_dir` is `None` on an untrusted root
/// (structural trust-gating, Tech Spec §6.7).
pub struct MemoryStore {
    user_dir: PathBuf,
    project_dir: Option<PathBuf>,
}

impl MemoryStore {
    /// Build a store over two scope directories. `project_dir` is `None` on an
    /// untrusted root so project-scope ops are structurally rejected.
    #[must_use]
    pub fn new(user_dir: PathBuf, project_dir: Option<PathBuf>) -> Self {
        Self {
            user_dir,
            project_dir,
        }
    }

    /// The directory for a given scope, or `None` when project memory is
    /// unavailable (untrusted root).
    fn dir_for(&self, scope: MemoryScope) -> Option<&Path> {
        match scope {
            MemoryScope::User => Some(&self.user_dir),
            MemoryScope::Project => self.project_dir.as_deref(),
        }
    }

    /// Execute a memory request against the store. The authoritative name guard
    /// (`slug`) runs here as defense in depth (group 1 validates in the tool,
    /// group 3 re-validates at the engine, this is the belt to those braces).
    pub fn execute(&self, req: &MemoryRequest) -> MemoryOutcome {
        // Authoritative name guard.
        let slugified = match slug(&req.name) {
            Ok(s) => s,
            Err(e) => return MemoryOutcome::Rejected { reason: e.0 },
        };

        let dir = match self.dir_for(req.scope) {
            Some(d) => d,
            None => {
                return MemoryOutcome::Rejected {
                    reason: "project memory unavailable (untrusted root)".into(),
                }
            }
        };

        let entry_path = dir.join(format!("{slugified}.md"));
        // Belt-and-braces over the group-1 slug guard: assert the resolved
        // path stays under the scope dir (Tech Spec §8.1).
        if !entry_path.starts_with(dir) {
            return MemoryOutcome::Rejected {
                reason: "entry path escapes the scope directory".into(),
            };
        }
        match req.op {
            MemoryOp::Write => {
                if entry_path.exists() {
                    return MemoryOutcome::Rejected {
                        reason: format!("an entry named '{}' already exists; use 'update' to overwrite", req.name),
                    };
                }
                self.write_entry(dir, &entry_path, req)
            }
            MemoryOp::Update => self.write_entry(dir, &entry_path, req),
            MemoryOp::Remove => {
                if entry_path.exists() {
                    let _ = std::fs::remove_file(&entry_path);
                }
                let (user_count, project_count) = self.counts();
                MemoryOutcome::Written {
                    user: user_count,
                    project: project_count,
                }
            }
            MemoryOp::Recall => {
                let body = std::fs::read_to_string(&entry_path)
                    .ok()
                    .map(|text| {
                        let (_, body) = split_frontmatter(&text);
                        body.trim().to_string()
                    });
                MemoryOutcome::Recalled {
                    body,
                    origin: req.scope,
                }
            }
        }
    }

    /// Write an entry file with frontmatter + body, then regenerate the index.
    fn write_entry(
        &self,
        dir: &Path,
        path: &Path,
        req: &MemoryRequest,
    ) -> MemoryOutcome {
        let meta = EntryMeta {
            name: req.name.clone(),
            description: req.description.clone().unwrap_or_default(),
            type_: req.type_.clone(),
        };
        let body = req.body.clone().unwrap_or_default();
        let content = serialize_entry(&meta, &body);
        if let Err(e) = std::fs::create_dir_all(dir) {
            return MemoryOutcome::Rejected {
                reason: format!("cannot create memory directory: {e}"),
            };
        }
        if let Err(e) = std::fs::write(path, &content) {
            return MemoryOutcome::Rejected {
                reason: format!("cannot write memory entry: {e}"),
            };
        }
        let _ = regenerate_index(dir);
        let (user_count, project_count) = self.counts();
        MemoryOutcome::Written {
            user: user_count,
            project: project_count,
        }
    }

    /// Load the index text for a scope (empty string when no entries).
    fn load_index_text(&self, scope: MemoryScope) -> String {
        match self.dir_for(scope) {
            Some(dir) => load_index(dir).0,
            None => String::new(),
        }
    }

    /// The user-global index text for pinning.
    pub fn user_index(&self) -> String {
        self.load_index_text(MemoryScope::User)
    }

    /// The project index text for pinning (empty when project scope unavailable).
    pub fn project_index(&self) -> String {
        self.load_index_text(MemoryScope::Project)
    }

    /// Entry counts for both scopes (feeds `MemoryStatus`).
    fn counts(&self) -> (usize, usize) {
        let user = self.dir_for(MemoryScope::User).map_or(0, count_entries);
        let project = self.dir_for(MemoryScope::Project).map_or(0, count_entries);
        (user, project)
    }

    /// The (user, project) counts for `MemoryStatus`.
    pub fn status_counts(&self) -> (usize, usize) {
        self.counts()
    }
}

/// Count `.md` entry files in a dir (excluding `MEMORY.md`).
fn count_entries(dir: &Path) -> usize {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if name != "MEMORY" {
                count += 1;
            }
        }
    }
    count
}

/// Load or regenerate the index for a dir. Returns the index text + entry count.
pub fn load_index(dir: &Path) -> (String, usize) {
    let index_path = dir.join("MEMORY.md");
    if let Ok(text) = std::fs::read_to_string(&index_path) {
        let count = count_entries(dir);
        return (text, count);
    }
    // No index yet — generate it.
    let (text, count) = build_index(dir);
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&index_path, &text);
    (text, count)
}

/// Regenerate `MEMORY.md` for a dir after a mutation.
fn regenerate_index(dir: &Path) -> (String, usize) {
    let (text, count) = build_index(dir);
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("MEMORY.md"), &text);
    (text, count)
}

/// Build the one-line-per-entry index from the `.md` files in a dir.
fn build_index(dir: &Path) -> (String, usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (String::new(), 0),
    };
    let mut items: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name == "MEMORY" {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let (front, _) = split_frontmatter(&text);
            if let Some(front) = front {
                if let Ok(meta) = toml::from_str::<EntryMeta>(front) {
                    items.push((meta.name, meta.description));
                }
            }
        }
    }
    // Stable sort by name.
    items.sort_by_key(|(name, _)| name.to_lowercase());
    let count = items.len();
    let mut out = String::new();
    for (name, desc) in &items {
        if desc.is_empty() {
            out.push_str(&format!("- {name}\n"));
        } else {
            out.push_str(&format!("- {name} — {desc}\n"));
        }
    }
    (out, count)
}

/// Split a leading `+++` TOML frontmatter fence from the body. Returns
/// `(frontmatter, body)` or `(text, "")` when there is no fence.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix("+++\n").or_else(|| text.strip_prefix("+++\r\n"));
    match text {
        Some(rest) => {
            if let Some(end) = rest.find("\n+++\n").or_else(|| rest.find("\n+++\r\n")) {
                let front = &rest[..end];
                let body_start = end + "\n+++\n".len();
                let body = rest.get(body_start..).unwrap_or("");
                (Some(front), body)
            } else if let Some(end) = rest.find("+++\n") {
                let front = &rest[..end];
                let body_start = end + "+++\n".len();
                let body = rest.get(body_start..).unwrap_or("");
                (Some(front), body)
            } else {
                (None, text.unwrap_or(""))
            }
        }
        None => (None, text.unwrap_or("")),
    }
}

/// Serialize an entry with TOML frontmatter + markdown body.
fn serialize_entry(meta: &EntryMeta, body: &str) -> String {
    let front = toml::to_string(meta).unwrap_or_default();
    format!("+++\n{front}+++\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-mem-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn write_creates_entry_and_index() {
        let dir = temp_dir();
        let store = MemoryStore::new(dir.clone(), None);
        let outcome = store.execute(&MemoryRequest {
            op: MemoryOp::Write,
            scope: MemoryScope::User,
            name: "API Keys".into(),
            description: Some("Where keys live".into()),
            type_: None,
            body: Some("Check ~/.config/keys".into()),
        });
        match outcome {
            MemoryOutcome::Written { user, project } => {
                assert_eq!(user, 1);
                assert_eq!(project, 0);
            }
            other => panic!("expected Written, got {other:?}"),
        }
        // The entry file exists.
        let entry = std::fs::read_to_string(dir.join("api-keys.md")).unwrap_or_default();
        assert!(entry.contains("API Keys"));
        assert!(entry.contains("Where keys live"));
        assert!(entry.contains("Check ~/.config/keys"));
        // The index was generated.
        let index = std::fs::read_to_string(dir.join("MEMORY.md")).unwrap_or_default();
        assert!(index.contains("API Keys"));
        assert!(index.contains("Where keys live"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recall_returns_body() {
        let dir = temp_dir();
        let store = MemoryStore::new(dir.clone(), None);
        store.execute(&MemoryRequest {
            op: MemoryOp::Write,
            scope: MemoryScope::User,
            name: "Note".into(),
            description: Some("desc".into()),
            type_: None,
            body: Some("Remember this".into()),
        });
        let outcome = store.execute(&MemoryRequest {
            op: MemoryOp::Recall,
            scope: MemoryScope::User,
            name: "Note".into(),
            description: None,
            type_: None,
            body: None,
        });
        match outcome {
            MemoryOutcome::Recalled { body: Some(b), .. } => assert_eq!(b, "Remember this"),
            other => panic!("expected Recalled with body, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recall_missing_returns_none() {
        let dir = temp_dir();
        let store = MemoryStore::new(dir.clone(), None);
        let outcome = store.execute(&MemoryRequest {
            op: MemoryOp::Recall,
            scope: MemoryScope::User,
            name: "nope".into(),
            description: None,
            type_: None,
            body: None,
        });
        match outcome {
            MemoryOutcome::Recalled { body: None, .. } => {}
            other => panic!("expected Recalled None, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_deletes_entry() {
        let dir = temp_dir();
        let store = MemoryStore::new(dir.clone(), None);
        store.execute(&MemoryRequest {
            op: MemoryOp::Write,
            scope: MemoryScope::User,
            name: "Temp".into(),
            description: Some("temp".into()),
            type_: None,
            body: Some("temp body".into()),
        });
        assert!(dir.join("temp.md").exists());
        store.execute(&MemoryRequest {
            op: MemoryOp::Remove,
            scope: MemoryScope::User,
            name: "Temp".into(),
            description: None,
            type_: None,
            body: None,
        });
        assert!(!dir.join("temp.md").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_scope_unavailable_when_none() {
        let dir = temp_dir();
        let store = MemoryStore::new(dir.clone(), None);
        let outcome = store.execute(&MemoryRequest {
            op: MemoryOp::Write,
            scope: MemoryScope::Project,
            name: "test".into(),
            description: Some("test".into()),
            type_: None,
            body: Some("body".into()),
        });
        match outcome {
            MemoryOutcome::Rejected { reason } => assert!(reason.contains("untrusted")),
            other => panic!("expected Rejected, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frontmatter_round_trips() {
        let meta = EntryMeta {
            name: "Test".into(),
            description: "A test".into(),
            type_: Some("fact".into()),
        };
        let serialized = serialize_entry(&meta, "body text");
        let (front, body) = split_frontmatter(&serialized);
        assert!(front.is_some());
        assert_eq!(body.trim(), "body text");
        let parsed: EntryMeta = toml::from_str(front.unwrap()).unwrap();
        assert_eq!(parsed.name, "Test");
        assert_eq!(parsed.description, "A test");
        assert_eq!(parsed.type_, Some("fact".into()));
    }
}
