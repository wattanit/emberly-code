//! `emberly init` (Requirements C-2, Design §8.1). Materializes the active
//! defaults into `.agents/` so a project's configuration and prompts are
//! visible and editable. Never overwrites existing files — re-running is safe.
//! The output is a considerate colleague, not an installer wizard: what was
//! created, where, and the one next command worth knowing.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// A commented `config.toml` — everything works without it, so the template is
/// mostly guidance (Requirements C-1).
pub const CONFIG_TEMPLATE: &str = r#"# emberly project configuration (.agents/config.toml)
# Everything here is optional — emberly ships baked-in provider profiles:
#   anthropic  (Anthropic Messages API)
#   openai     (OpenAI Chat Completions)
#   zai        (Z.ai coding plan — OpenAI-compatible)
#   local      (http://localhost:11434/v1 — Ollama/vLLM, keyless)
# Values here override the global config; EMBERLY_* env vars override these.

# Pick the active profile and model. Keys are never stored here — put them in
# ~/.config/emberly/keys.toml (a flat `ref = "secret"` table) or the
# <REF>_API_KEY env var, e.g. ZAI_API_KEY for the `zai` profile.
# provider = "zai"
# model    = "glm-4.6"

# Adding your own provider is configuration, not code: pick an `adapter` (the
# wire format — "anthropic" or "openai"), an endpoint, and a key *reference*.
# [providers.myserver]
# adapter  = "openai"
# base_url = "https://my-endpoint.example/v1"
# auth     = { scheme = "bearer", key = "myserver" }   # needs MYSERVER_API_KEY

# Optional per-model metadata (context window, max output, pricing → cost est.):
# Set `effort` to enable the reasoning-effort control (/effort, low|medium|
# high|max); it maps to the provider's native knob or is ignored if none.
# [providers.anthropic.models."claude-sonnet-5"]
# context_window = 200000
# max_output     = 8192
# pricing = { input = 3.0, output = 15.0 }   # USD per million tokens
# effort  = "medium"                         # default level; enables /effort
# effort_levels = ["low", "medium", "high"]  # optional subset (default: all)

# Reasoning-trail view: how the model's thinking is shown (collapsed|expanded|
# hidden). Default collapsed; hidden still records the trace to the transcript.
# reasoning = "collapsed"

# [ui]
# Tool-call explanations: a dim caption under non-obvious tool calls, authored
# by the model. On by default. Set false to defeat it — the schema property and
# the prompt instruction are both dropped, so no tokens are spent on it.
# tool_explanations = true
#
# Pointer (mouse) interaction in the rich TUI (Design §3.4): wheel scroll and
# click-to-select, additive to the keyboard and never the sole path to anything.
# On by default. Set false to release the mouse entirely — the terminal keeps
# its native click-drag selection everywhere. Degraded mode (--plain/NO_COLOR/
# TERM=dumb) never captures the mouse regardless. With capture on, hold your
# terminal's selection modifier (usually Shift) to drag-select and copy as usual.
# mouse = true
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
        &format!("{}\n", emberly_core::prompts::system()),
        &mut created,
    )?;
    write_if_absent(
        &agents.join("prompts").join("compact.md"),
        &format!("{}\n", emberly_core::prompts::compact()),
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
