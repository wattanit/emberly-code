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
#
# You don't need this file. emberly ships four ready-to-use provider
# profiles, and any one of them works as soon as it has a key. Everything
# below is commented out — uncomment only the lines you want to change.
# Leaving a line commented keeps emberly's built-in default; it does not
# turn that setting off.
#
#   anthropic  →  Anthropic Messages API       (needs ANTHROPIC_API_KEY)
#   openai     →  OpenAI Chat Completions API  (needs OPENAI_API_KEY)
#   zai        →  Z.ai coding plan             (needs ZAI_API_KEY)
#   local      →  http://localhost:11434/v1 — Ollama/vLLM (keyless)
#
# When a setting is given more than one place, this order decides which
# value wins (later beats earlier): your global config at
# ~/.config/emberly/config.toml, then this file, then any EMBERLY_*
# environment variable, then --provider/--model on the command line.

# ── Choose your provider and model ────────────────────────────────────────
# Set these to pick which profile emberly uses by default — one of the four
# built-in ones below, or one of your own further down this file.
# provider = "zai"
# model    = "glm-4.6"

# How the model's reasoning is shown while it works. This setting applies
# everywhere, so it lives up here rather than inside [ui] further down.
# reasoning = "collapsed"   # collapsed | expanded | hidden

# ── Anthropic ──────────────────────────────────────────────────────────────
# Already set up for you: adapter "anthropic", endpoint
# https://api.anthropic.com, and a key read from your ANTHROPIC_API_KEY
# environment variable. You only need this section to change the endpoint,
# or to describe a model as shown below.
# [providers.anthropic]
# base_url = "https://api.anthropic.com"
# auth     = { scheme = "x-api-key", key = "anthropic" }   # reads ANTHROPIC_API_KEY
#
# Describing a model is optional. Without it, emberly assumes a 200,000-token
# context window and a 4,096-token output limit, and there's nothing for the
# /effort command to control. Here's what you can tell emberly about a model:
# [providers.anthropic.models."claude-sonnet-5"]
# context_window = 200000
# max_output     = 8192
# pricing        = { input = 3.0, output = 15.0 }    # dollars per million tokens — check your plan for the current rate
# effort         = "medium"                          # the default reasoning-effort level; setting this turns on /effort
# effort_levels  = ["low", "medium", "high", "max"]   # which levels /effort offers; leave this out to offer all four
# vision         = true                               # this model can read images you attach
# documents      = true                               # this model can read PDFs and other documents you attach

# ── OpenAI ───────────────────────────────────────────────────────────────
# Already set up for you: adapter "openai", endpoint
# https://api.openai.com/v1, and a key read from your OPENAI_API_KEY
# environment variable.
# [providers.openai]
# base_url = "https://api.openai.com/v1"
# auth     = { scheme = "bearer", key = "openai" }   # reads OPENAI_API_KEY
#
# See the anthropic section above for what each of these fields means.
# [providers.openai.models."gpt-5.1"]
# context_window = 400000
# max_output     = 128000
# pricing        = { input = 1.25, output = 10.0 }   # dollars per million tokens — check your plan for the current rate
# effort         = "medium"
# effort_levels  = ["low", "medium", "high"]

# ── Z.ai ─────────────────────────────────────────────────────────────────
# Already set up for you: adapter "openai" (Z.ai speaks the same wire
# format), endpoint https://api.z.ai/api/paas/v4, and a key read from your
# ZAI_API_KEY environment variable.
# [providers.zai]
# base_url = "https://api.z.ai/api/paas/v4"
# auth     = { scheme = "bearer", key = "zai" }   # reads ZAI_API_KEY
#
# [providers.zai.models."glm-4.6"]
# context_window = 200000
# max_output     = 128000
# pricing        = { input = 0.6, output = 2.2 }   # dollars per million tokens — check your plan for the current rate

# ── Local (Ollama, vLLM, or anything else OpenAI-compatible) ─────────────
# Already set up for you: adapter "openai", endpoint
# http://localhost:11434/v1, and no key required.
# [providers.local]
# base_url = "http://localhost:11434/v1"
#
# [providers.local.models."llama3"]
# context_window = 128000
# max_output     = 8192

# ── MCP servers ────────────────────────────────────────────────────────────
# Connect an external tool server over stdio. Every tool it exposes shows up
# as mcp__<name>__<tool> and asks for permission the same way any other tool
# does — nothing here bypasses that. A server listed here only ever connects
# in a project you've trusted; list it in your global config instead if you
# want it available everywhere, where it is never trust-gated.
# [mcp.servers.myserver]
# transport = "stdio"                # the only transport this version supports
# command   = "npx"
# args      = ["-y", "@some/mcp-server"]
# enabled   = true

# ── Web search ─────────────────────────────────────────────────────────────
# Off until you point it at a real backend — there is no default endpoint, so
# nothing reaches the internet (and nothing costs you anything) unless you set
# this up yourself. Uncomment exactly one of the three blocks below; whichever
# `adapter` you pick decides the request shape and how results are parsed.
#
# Brave Search API (needs a key: https://brave.com/search/api/):
# [search]
# adapter     = "brave"
# endpoint    = "https://api.search.brave.com/res/v1/web/search"
# auth        = { scheme = "header", header = "X-Subscription-Token", key = "brave" }   # reads BRAVE_API_KEY
# max_results = 5     # results sent to the model per search; default 5
#
# Tavily Search API (needs a key: https://tavily.com/):
# [search]
# adapter  = "tavily"
# endpoint = "https://api.tavily.com/search"
# auth     = { scheme = "bearer", key = "tavily" }   # reads TAVILY_API_KEY
#
# A self-hosted SearXNG instance (often keyless):
# [search]
# adapter  = "searxng"
# endpoint = "http://localhost:8080/search"

# ── Interface ──────────────────────────────────────────────────────────────
# [ui]
# Whether a tool call gets a short, dim explanation of what it's doing,
# written by the model. On by default. Turn it off and you spend no tokens
# on it — the prompt instruction and the schema property are both dropped.
# tool_explanations = true
#
# Whether the rich TUI responds to your mouse: wheel scroll and
# click-to-select. This is always in addition to the keyboard, never a
# replacement for it. On by default. A degraded terminal (plain mode, no
# color, or a dumb terminal type) never uses the mouse regardless of this
# setting.
# mouse = true

# ── Using a provider that isn't one of the four above ─────────────────────
# Any endpoint that speaks the OpenAI or Anthropic wire format works here.
# Give it an `adapter` matching which one it speaks, its `base_url`, and a
# name for its key — emberly reads that key from an environment variable
# named after it, in capitals, with _API_KEY on the end.
# [providers.myserver]
# adapter  = "openai"                                  # or "anthropic"
# base_url = "https://my-endpoint.example/v1"
# auth     = { scheme = "bearer", key = "myserver" }   # reads MYSERVER_API_KEY
#
# Other ways to send that key, if bearer isn't what the endpoint expects:
#   { scheme = "x-api-key", key = "myserver" }                        # sends it in an x-api-key header, same as Anthropic
#   { scheme = "header", header = "X-Custom-Auth", key = "myserver" } # sends it under a header name you choose
#   { scheme = "none" }                                               # no key needed
"#;

/// A documented `permissions.toml` seeded with an example (commented out) —
/// the rule engine is fully implemented (`emberly-sandbox::rules`); this file
/// is empty of active rules by default so a fresh project starts conservative.
pub const PERMISSIONS_TEMPLATE: &str = r#"# emberly permission rules (.agents/permissions.toml)
# A small built-in allowlist (read-only commands like `ls`, `cat`, `git
# status`/`diff`/`log`) already runs without prompting; everything else asks
# until you add rules here to pre-approve matching tool actions:
#
#   [[rule]]
#   tool  = "bash"
#   match = "cargo *"
#   action = "allow"      # allow | ask | deny
"#;

/// Keep transcripts (and their sidecars) and scratch files out of version
/// control; config and prompts are shareable.
pub const GITIGNORE_TEMPLATE: &str =
    "# emberly: session transcripts and scratch files are local, not shared\nsessions/\nscratch/\n";

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
        // Both local-only directories are excluded — a session's scratch
        // files are as disposable as its transcript and must never get
        // staged into the project by an unqualified `git add .agents/`.
        let gitignore = std::fs::read_to_string(root.join(".agents/.gitignore")).unwrap();
        assert!(gitignore.contains("sessions/"));
        assert!(gitignore.contains("scratch/"));
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
