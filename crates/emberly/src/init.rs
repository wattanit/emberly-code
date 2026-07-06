//! `emberly init` (Requirements C-2, Design §8.1). Materializes the active
//! defaults into `.agents/` so a project's configuration and prompts are
//! visible and editable. Never overwrites existing files — re-running is safe.
//! The output is a considerate colleague, not an installer wizard: what was
//! created, where, and the one next command worth knowing.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::config::DEFAULT_SYSTEM_PROMPT;

/// A commented `config.toml` — everything works without it, so the template is
/// mostly guidance (Requirements C-1).
const CONFIG_TEMPLATE: &str = r#"# emberly project configuration (.agents/config.toml)
# Everything here is optional — emberly runs on baked-in defaults. Values set
# here override the global config; EMBERLY_* env vars override these.

# provider = "anthropic"        # or "openai" (covers Ollama/vLLM/OpenRouter)
# model    = "claude-sonnet-5"
# base_url = "https://api.openai.com/v1"   # for openai-compatible endpoints
# context_window = 200000
# max_output     = 8192

# Per-model pricing (USD per million tokens) → session cost estimate.
# [pricing."claude-sonnet-5"]
# input  = 3.0
# output = 15.0
"#;

/// A documented `permissions.toml`. The rule engine is Phase 2; this reserves
/// the pattern and file location so the shape is stable.
const PERMISSIONS_TEMPLATE: &str = r#"# emberly permission rules (.agents/permissions.toml)
# The rule engine lands in a later phase; until then every tool action is
# prompted per-invocation. This file reserves the format:
#
#   [[rule]]
#   tool  = "bash"
#   match = "cargo *"
#   action = "allow"      # allow | ask | deny
"#;

/// Keep transcripts (and their sidecars) out of version control; config and
/// prompts are shareable.
const GITIGNORE_TEMPLATE: &str =
    "# emberly: session transcripts are local, not shared\nsessions/\n";

/// Materialize `.agents/` defaults for `project_root` (C-2).
pub fn init(project_root: &Path) -> anyhow::Result<()> {
    let agents = project_root.join(".agents");
    let mut created = Vec::new();

    write_if_absent(&agents.join("config.toml"), CONFIG_TEMPLATE, &mut created)?;
    write_if_absent(
        &agents.join("prompts").join("system.md"),
        &format!("{DEFAULT_SYSTEM_PROMPT}\n"),
        &mut created,
    )?;
    write_if_absent(
        &agents.join("prompts").join("compact.md"),
        &format!("{}\n", emberly_core::SUMMARY_PROMPT),
        &mut created,
    )?;
    write_if_absent(
        &agents.join("permissions.toml"),
        PERMISSIONS_TEMPLATE,
        &mut created,
    )?;
    write_if_absent(&agents.join(".gitignore"), GITIGNORE_TEMPLATE, &mut created)?;

    report(project_root, &created);
    Ok(())
}

/// Write `contents` to `path` only if it does not already exist (never clobber);
/// records created paths for the summary.
fn write_if_absent(path: &Path, contents: &str, created: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    created.push(path.to_path_buf());
    Ok(())
}

/// Print exactly what was created and the one next command (Design §8.1).
fn report(project_root: &Path, created: &[PathBuf]) {
    if created.is_empty() {
        println!("Nothing to do — .agents/ is already set up.");
        return;
    }
    println!("Created in {}:", project_root.join(".agents").display());
    for path in created {
        // Show the path relative to the project root for a tidy list.
        let shown = path.strip_prefix(project_root).unwrap_or(path);
        println!("  {}", shown.display());
    }
    println!();
    println!("Next: set your provider and model in .agents/config.toml, then run `emberly`.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-init-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn init_materializes_the_tree() {
        let root = tmp();
        init(&root).unwrap();
        assert!(root.join(".agents/config.toml").exists());
        assert!(root.join(".agents/prompts/system.md").exists());
        assert!(root.join(".agents/prompts/compact.md").exists());
        assert!(root.join(".agents/permissions.toml").exists());
        assert!(root.join(".agents/.gitignore").exists());
        // The materialized system prompt is the baked-in default.
        let system = std::fs::read_to_string(root.join(".agents/prompts/system.md")).unwrap();
        assert!(system.contains("Emberly Code"));
    }

    #[test]
    fn init_never_clobbers_existing_files() {
        let root = tmp();
        let config = root.join(".agents/config.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "provider = \"mine\"\n").unwrap();
        init(&root).unwrap();
        // The user's file is preserved; other files are still created.
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "provider = \"mine\"\n"
        );
        assert!(root.join(".agents/prompts/system.md").exists());
    }
}
