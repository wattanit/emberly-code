//! `emberly clean` (FR-8, Tech Spec §8.3/§10) — reclaim the disk space used by
//! session scratch directories (T-17). Scratch space is disposable and never
//! auto-cleaned by the engine; this is the user's reclaim path.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;

/// One session's scratch directory and its size on disk.
struct Target {
    session_id: String,
    dir: PathBuf,
    bytes: u64,
}

/// `emberly clean [<session-id>]`. With no id, every session directory under
/// `scratch_root` is a candidate; with an id, only that one. Reports the total
/// size, asks a plain `y`/`n` confirmation (Design §8.8), then deletes. A
/// target with nothing to clean says so and exits.
pub fn clean(scratch_root: &Path, session_id: Option<&str>) -> Result<()> {
    let targets = match session_id {
        Some(id) => {
            let dir = scratch_root.join(id);
            if dir.exists() {
                vec![Target {
                    session_id: id.to_string(),
                    bytes: dir_size(&dir),
                    dir,
                }]
            } else {
                Vec::new()
            }
        }
        None => scan_all(scratch_root),
    };

    if targets.is_empty() {
        println!("nothing to clean");
        return Ok(());
    }

    let total: u64 = targets.iter().map(|t| t.bytes).sum();
    println!(
        "Found {} of scratch data across {} session{}:",
        format_size(total),
        targets.len(),
        if targets.len() == 1 { "" } else { "s" }
    );
    for t in &targets {
        println!("  {} · {}", t.session_id, format_size(t.bytes));
    }

    if !confirm()? {
        println!("Not deleted.");
        return Ok(());
    }

    let mut removed = 0usize;
    for t in &targets {
        if std::fs::remove_dir_all(&t.dir).is_ok() {
            removed += 1;
        }
    }
    println!(
        "Removed scratch data for {removed} session{}.",
        if removed == 1 { "" } else { "s" }
    );
    Ok(())
}

/// Every immediate subdirectory of `scratch_root` — one per session that has
/// ever written a scratch file — sorted by session id for stable output.
fn scan_all(scratch_root: &Path) -> Vec<Target> {
    let mut targets = Vec::new();
    let Ok(entries) = std::fs::read_dir(scratch_root) else {
        return targets;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(session_id) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        targets.push(Target {
            session_id: session_id.to_string(),
            bytes: dir_size(&path),
            dir: path,
        });
    }
    targets.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    targets
}

/// Total bytes used by a directory tree. Scratch files are always flat (the
/// `scratch_write` tool rejects any name with a path separator), so this
/// never actually recurses in practice; the recursion is just defense against
/// anything unexpected on disk. Best-effort: unreadable entries are skipped
/// rather than failing the whole scan — this is a reclaim tool, not an audit.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return total;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// A human-readable byte size — plain units, no walls of text (Design §8.8).
fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if bytes >= MIB {
        format!(
            "{:.1} MiB",
            f64::from(u32::try_from(bytes / 1024).unwrap_or(u32::MAX)) / 1024.0
        )
    } else if bytes >= KIB {
        format!(
            "{:.1} KiB",
            f64::from(u32::try_from(bytes).unwrap_or(u32::MAX)) / 1024.0
        )
    } else {
        format!("{bytes} B")
    }
}

/// The clean confirmation (Design §8.8): a plain `y`/`n` question, same tone
/// and safe-default decline as the trust prompt (Design §8.4). A
/// non-interactive launch cannot answer, so it declines cleanly.
fn confirm() -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        eprintln!("emberly: non-interactive session — declining to delete without confirmation.");
        return Ok(false);
    }
    print!("Delete this scratch data? (y/N) ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return Ok(false);
    }
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emberly-clean-test-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn scan_all_finds_every_session_directory_sorted() {
        let root = temp_dir("scan");
        for (id, content) in [("b-session", "12345"), ("a-session", "12")] {
            let dir = root.join(id);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("f.txt"), content);
        }
        let targets = scan_all(&root);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].session_id, "a-session");
        assert_eq!(targets[0].bytes, 2);
        assert_eq!(targets[1].session_id, "b-session");
        assert_eq!(targets[1].bytes, 5);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_all_on_a_missing_root_is_empty_not_an_error() {
        let root = temp_dir("missing");
        let _ = std::fs::remove_dir_all(&root);
        assert!(scan_all(&root).is_empty());
    }

    #[test]
    fn dir_size_sums_files_in_a_directory() {
        let root = temp_dir("size");
        let _ = std::fs::write(root.join("a.txt"), "abc");
        let _ = std::fs::write(root.join("b.txt"), "de");
        assert_eq!(dir_size(&root), 5);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn format_size_picks_the_right_unit() {
        assert_eq!(format_size(42), "42 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
