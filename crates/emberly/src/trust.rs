//! Workspace trust (FR-1, Tech Spec §6.7, Design §8.4).
//!
//! A **consent gate, not containment.** Before Emberly reads, edits, or runs
//! anything in a folder the user has not trusted before, it asks — in plain
//! language, safe-default *decline* — and does not start a session until the
//! user consents. This lives in the binary, imports nothing from
//! `emberly-sandbox`, and changes no ruleset: it decides *whether* the agent
//! runs here, never *what* it may do (FR-1 honesty clause).
//!
//! Trust is **global-only**: the store is `~/.config/emberly/trust.toml`
//! (0600), keyed by canonical path, and **subtree-trusted** (trusting a folder
//! trusts its subdirectories). A project cannot pre-declare itself trusted.

use std::collections::HashSet;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config;

/// The on-disk trust store: canonical paths the user has accepted.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustStore {
    #[serde(default)]
    trusted: Vec<TrustEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrustEntry {
    /// Canonical folder path.
    path: String,
    /// Always true today (declines never persist); kept per Tech Spec §6.7.
    #[serde(default = "default_true")]
    accepted: bool,
    /// Unix seconds when accepted.
    #[serde(default)]
    ts: u64,
}

fn default_true() -> bool {
    true
}

/// The result of the startup gate.
pub enum Gate {
    /// Trust holds — proceed. `newly_trusted` is true only when the user just
    /// granted it (the engine records a `trust_decision` in that case).
    Proceed { newly_trusted: bool },
    /// The user declined — do not start a session (FR-1).
    Declined,
}

/// The startup trust gate (FR-1). `root` must already be canonicalized.
/// Trusted (store ∪ global allowlist) → proceed silently. Otherwise prompt with
/// a safe-default decline; accept records the path and proceeds.
pub fn gate(root: &Path) -> Result<Gate> {
    if is_trusted(root)? {
        return Ok(Gate::Proceed {
            newly_trusted: false,
        });
    }
    if prompt_accepts(root) {
        record_trust(root)?;
        Ok(Gate::Proceed {
            newly_trusted: true,
        })
    } else {
        println!(
            "Not trusted — no session started. Run `emberly trust list` to review trusted folders."
        );
        Ok(Gate::Declined)
    }
}

/// Whether `root` (canonical) is trusted: it or any ancestor is an accepted
/// store entry, or lies within a `trust.trusted_dirs` allowlist entry (FR-1
/// subtree trust).
fn is_trusted(root: &Path) -> Result<bool> {
    let mut trusted: HashSet<PathBuf> = load_store()?
        .trusted
        .into_iter()
        .filter(|e| e.accepted)
        .map(|e| PathBuf::from(e.path))
        .collect();
    for dir in config::global_trust_dirs() {
        if let Ok(canon) = expand_tilde(&dir).canonicalize() {
            trusted.insert(canon);
        }
    }
    Ok(is_member(root, &trusted))
}

/// Subtree membership: `root` is trusted if it or any ancestor is in the
/// trusted set (FR-1 — trusting a folder trusts its subtree). Pure, so the
/// membership rule is testable without touching the filesystem.
fn is_member(root: &Path, trusted: &HashSet<PathBuf>) -> bool {
    root.ancestors().any(|a| trusted.contains(a))
}

/// Record `root` (canonical) as trusted and persist the store at 0600.
fn record_trust(root: &Path) -> Result<()> {
    let mut store = load_store()?;
    let path = root.display().to_string();
    if !store.trusted.iter().any(|e| e.path == path) {
        store.trusted.push(TrustEntry {
            path,
            accepted: true,
            ts: now_secs(),
        });
    }
    save_store(&store)
}

/// `emberly trust list` — the accepted folders and when (Tech Spec §10).
pub fn list() -> Result<()> {
    let store = load_store()?;
    if store.trusted.is_empty() {
        println!("No trusted folders yet.");
        return Ok(());
    }
    println!("Trusted folders:");
    for e in &store.trusted {
        println!("  {}", e.path);
    }
    Ok(())
}

/// `emberly trust revoke <path>` — remove a folder, re-arming the gate for it
/// (Tech Spec §10). Matches on the canonical path, falling back to the literal
/// argument so a since-deleted folder can still be revoked.
pub fn revoke(arg: &str) -> Result<()> {
    let path = config::global_trust_path().context("no home directory for the trust store")?;
    match revoke_at(&path, arg)? {
        Some(target) => println!("Revoked trust for {target}. It will be asked about again."),
        None => println!("{arg} was not in the trust store."),
    }
    Ok(())
}

/// Remove `arg` (canonical or literal) from the store at `path`. Returns the
/// removed target on success, `None` if it was not present. Split out so the
/// revoke round-trip is testable without the global path.
fn revoke_at(path: &Path, arg: &str) -> Result<Option<String>> {
    let target = expand_tilde(arg)
        .canonicalize()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| arg.to_string());
    let mut store = load_store_at(path)?;
    let before = store.trusted.len();
    store.trusted.retain(|e| e.path != target && e.path != arg);
    if store.trusted.len() == before {
        return Ok(None);
    }
    save_store_at(path, &store)?;
    Ok(Some(target))
}

// ---- store I/O -----------------------------------------------------------

fn load_store() -> Result<TrustStore> {
    match config::global_trust_path() {
        Some(path) => load_store_at(&path),
        None => Ok(TrustStore::default()),
    }
}

fn load_store_at(path: &Path) -> Result<TrustStore> {
    if !path.exists() {
        return Ok(TrustStore::default());
    }
    // The store records which folders may be operated on — treat it as private
    // (0600), like the keys file (Tech Spec §6.7/§8).
    config::enforce_private_permissions(path)?;
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn save_store(store: &TrustStore) -> Result<()> {
    let path = config::global_trust_path().context("no home directory for the trust store")?;
    save_store_at(&path, store)
}

fn save_store_at(path: &Path, store: &TrustStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = toml::to_string(store).context("serializing the trust store")?;
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    set_private(path)
}

#[cfg(unix)]
fn set_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("setting 0600 on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> Result<()> {
    Ok(())
}

// ---- the prompt ----------------------------------------------------------

/// The trust prompt (Design §8.4): calm, plain, safe-default decline. Prints
/// before either frontend takes the terminal, so it reads the same in rich and
/// plain mode. A non-interactive launch cannot answer, so it declines cleanly.
fn prompt_accepts(root: &Path) -> bool {
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "emberly: {} is not a trusted folder and this is a non-interactive session — \
             declining (FR-1). Run `emberly` here interactively to trust it, or add it to \
             [trust].trusted_dirs in your global config.",
            root.display()
        );
        return false;
    }
    println!();
    println!("This folder has not been trusted before:");
    println!("  {}", root.display());
    println!("Trusting it lets Emberly read, edit, and run commands in this folder.");
    println!("Only trust code you have reason to trust — project files can carry instructions.");
    // The honesty clause, in situ (FR-1): trust is not a waiver of later prompts.
    println!("(Trusting does not switch off later permission prompts — it decides only whether Emberly runs here.)");
    print!("Trust this folder?  type \"trust\", \"yes\", or \"y\" to proceed · anything else = DON'T TRUST: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    accepts(&line)
}

/// The trust decision from a typed line (Design §8.4): a deliberate `trust`,
/// `yes`, or `y` grants; **anything else declines** (the safe default). Pure,
/// so the no-unsafe-default rule is testable.
fn accepts(line: &str) -> bool {
    matches!(line.trim().to_lowercase().as_str(), "trust" | "yes" | "y")
}

// ---- helpers -------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Expand a leading `~` to `$HOME` (allowlist entries are user-authored).
fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home).join(rest);
            }
        }
    }
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-trust-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn set(paths: &[&Path]) -> HashSet<PathBuf> {
        paths.iter().map(|p| p.to_path_buf()).collect()
    }

    #[test]
    fn subtree_trust_covers_descendants_only() {
        let root = tmp();
        let trusted = set(&[&root]);
        // The folder itself, and any subdirectory, are trusted.
        assert!(is_member(&root, &trusted));
        assert!(is_member(&root.join("a/b/c"), &trusted));
        // A sibling / unrelated folder is not.
        let other = tmp();
        assert!(!is_member(&other, &trusted));
        // An ancestor of a trusted folder is NOT trusted (trust flows down).
        assert!(!is_member(root.parent().expect("parent"), &trusted));
    }

    #[test]
    fn accepts_only_deliberate_words_no_unsafe_default() {
        assert!(accepts("trust"));
        assert!(accepts("  Trust\n"));
        assert!(accepts("yes"));
        assert!(accepts("y"));
        assert!(accepts("  Y\n"));
        // Everything else declines — including empty (this is a more deliberate
        // gate than the permission prompt).
        assert!(!accepts(""));
        assert!(!accepts("\n"));
        assert!(!accepts("no"));
        assert!(!accepts("sure"));
    }

    #[test]
    fn store_round_trips_and_membership_reflects_it() {
        let dir = tmp();
        let store_path = dir.join("trust.toml");
        let proj = tmp();
        let mut store = TrustStore::default();
        store.trusted.push(TrustEntry {
            path: proj.display().to_string(),
            accepted: true,
            ts: 123,
        });
        save_store_at(&store_path, &store).expect("save");
        let loaded = load_store_at(&store_path).expect("load");
        assert_eq!(loaded.trusted.len(), 1);
        assert_eq!(loaded.trusted[0].path, proj.display().to_string());

        let trusted: HashSet<PathBuf> = loaded
            .trusted
            .iter()
            .map(|e| PathBuf::from(&e.path))
            .collect();
        assert!(is_member(&proj.join("sub"), &trusted));
    }

    #[test]
    fn revoke_removes_an_entry_and_re_arms_the_gate() {
        let dir = tmp();
        let store_path = dir.join("trust.toml");
        // The store holds canonical paths (as `record_trust` writes them); temp
        // dirs are symlinked on macOS, so canonicalize before storing.
        let proj = tmp().canonicalize().expect("canon");
        let proj_str = proj.display().to_string();
        let mut store = TrustStore::default();
        store.trusted.push(TrustEntry {
            path: proj_str.clone(),
            accepted: true,
            ts: 0,
        });
        save_store_at(&store_path, &store).expect("save");

        // Revoking removes it…
        let removed = revoke_at(&store_path, &proj_str).expect("revoke");
        assert_eq!(removed.as_deref(), Some(proj_str.as_str()));
        let after = load_store_at(&store_path).expect("reload");
        assert!(after.trusted.is_empty(), "gate re-armed");

        // …and revoking again is a no-op, not an error.
        assert_eq!(revoke_at(&store_path, &proj_str).expect("revoke2"), None);
    }

    #[test]
    fn missing_store_is_empty_not_an_error() {
        let dir = tmp();
        let loaded = load_store_at(&dir.join("nope.toml")).expect("missing = default");
        assert!(loaded.trusted.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn store_is_written_0600_and_group_readable_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp();
        let store_path = dir.join("trust.toml");
        save_store_at(&store_path, &TrustStore::default()).expect("save");
        // Written private.
        let mode = std::fs::metadata(&store_path)
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "trust store must be 0600");
        // A group/world-readable store is refused on read (a repo shouldn't be
        // able to widen it — Tech Spec §6.7/§8).
        std::fs::set_permissions(&store_path, std::fs::Permissions::from_mode(0o644))
            .expect("chmod");
        assert!(load_store_at(&store_path).is_err(), "0644 store rejected");
    }

    #[test]
    fn expand_tilde_uses_home() {
        // Read-only: don't mutate the process env (parallel tests read HOME).
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                assert_eq!(expand_tilde("~/proj"), PathBuf::from(&home).join("proj"));
            }
        }
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }
}
