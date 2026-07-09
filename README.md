# Emberly Code

An interactive AI coding agent for the terminal. You describe a task in plain
language; the agent reads your code, proposes and makes edits, runs commands,
and iterates — asking permission before it touches anything — until the task is
done or you step in.

It is **provider-agnostic** (works with Anthropic and any OpenAI-compatible
endpoint, including local models), keeps a **complete, resumable transcript** of
every session, and is built to run reliably on standard and static (musl)
deployments.

> **Status:** v0.1 — feature-complete, install from source. The interactive
> TUI, live providers, session persistence, the permission *rule engine*, the
> auto-accept *modes*, and OS *confinement* (Linux Landlock, macOS Seatbelt) all
> work today. Prebuilt binaries and Windows support are not yet shipped (see
> [Safety](#safety--transparency)).

---

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [Using a session](#using-a-session)
- [Command-line reference](#command-line-reference)
- [Safety & transparency](#safety--transparency)
- [For developers](#for-developers)

---

## Install

No prebuilt binaries yet — build from source (needs a recent stable Rust
toolchain; no C toolchain required):

```sh
git clone <this-repo> && cd emberly-code
cargo build --release
# the binary is target/release/emberly — put it on your PATH, e.g.:
install -m 0755 target/release/emberly ~/.local/bin/emberly
```

## Quick start

1. **Pick a provider and give it a key.** Anthropic, for example:

   ```sh
   export EMBERLY_PROVIDER=anthropic
   export EMBERLY_MODEL=claude-sonnet-5
   export ANTHROPIC_API_KEY=sk-...
   ```

   Or point at any OpenAI-compatible endpoint (OpenAI, Ollama, vLLM,
   OpenRouter, …):

   ```sh
   export EMBERLY_PROVIDER=openai
   export EMBERLY_MODEL=gpt-5.2
   export EMBERLY_BASE_URL=http://localhost:11434/v1   # e.g. Ollama
   export OPENAI_API_KEY=...                            # if the endpoint needs one
   ```

2. **Run it in your project directory:**

   ```sh
   cd ~/my-project
   emberly
   ```

3. Type your task and press **Enter**. Emberly asks before running commands or
   editing files; you approve or deny each action.

Without a provider configured, Emberly still starts in an **offline placeholder
mode** so you can explore the interface — it just can't call a model.

To keep settings with a project instead of in your shell, run `emberly init`
(see below).

## Configuration

Everything is optional — Emberly runs on built-in defaults. Settings resolve in
this order, later winning over earlier:

1. Built-in defaults
2. Global config — `~/.config/emberly/config.toml` (or `$XDG_CONFIG_HOME/emberly/`)
3. Project config — `.agents/config.toml` in your project
4. `EMBERLY_*` environment variables
5. `--provider` / `--model` command-line flags

Run **`emberly init`** to scaffold `.agents/` in the current project with a
commented `config.toml`, the default prompts (editable, under
`.agents/prompts/`), a `permissions.toml` template, and a `.gitignore` that
keeps session transcripts out of version control. Existing files are never
overwritten.

**`.agents/config.toml`:**

```toml
provider = "anthropic"          # or "openai" (covers Ollama/vLLM/OpenRouter)
model    = "claude-sonnet-5"
# base_url = "http://localhost:11434/v1"   # for openai-compatible endpoints
# context_window = 200000
# max_output     = 8192

# Optional per-model pricing → live session cost estimate.
# [pricing."claude-sonnet-5"]
# input  = 3.0     # USD per million input tokens
# output = 15.0
```

**API keys** are read from `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`, or from
`~/.config/emberly/keys.toml` (which must be mode `0600`). Keys are never read
from project files, so they don't get committed.

**Project instructions:** an `AGENTS.md` (or `CLAUDE.md`) at your project root
is picked up automatically and given to the agent as standing context.

**Environment variables:** `EMBERLY_PROVIDER`, `EMBERLY_MODEL`,
`EMBERLY_BASE_URL`, `EMBERLY_CONTEXT_WINDOW`, `EMBERLY_MAX_OUTPUT`. For display,
`NO_COLOR` or `TERM=dumb` force plain mode and `EMBERLY_MOTION=0` disables
animation.

Run **`emberly config show`** to see the resolved settings and where each value
came from.

## Using a session

The screen is a conversation timeline — your messages and the agent's, tool
calls, and diffs, top to bottom — with a sidebar showing the model, context
usage, cost, and changed files.

**Input**

| Key | Action |
|---|---|
| `Enter` | Send your message |
| `Shift+Enter` / `Alt+Enter` | Newline (compose a multi-line message) |
| `Esc` | Cancel the current turn / dismiss an overlay |
| `↑` `↓` `PgUp` `PgDn` / mouse wheel | Scroll the conversation or an overlay |

**Commands** — open the palette with **`Ctrl-P`**, or type any `/name`:

| Command | Key | What it does |
|---|---|---|
| `/help` | `Ctrl-P` | List commands and keybindings |
| `/view` | | View the last assistant message in full |
| `/diff` | `Ctrl-O` | Open the latest file's diff |
| `/files` | | List files changed this session |
| `/session` | | List saved sessions and switch to one |
| `/new` (`/clear`) | | Start a fresh session (the current one is saved) |
| `/mode` | `Shift-Tab` | Cycle permission mode (normal → auto-accept edits → auto) |
| `/sidebar` | `Ctrl-B` | Toggle the sidebar |
| `/cancel` | | Cancel the in-flight turn |
| `/quit` | `Ctrl-D` | Exit |

**Permission prompts.** When the agent wants to run a command or change a file,
it shows exactly what it will do and asks you to allow or deny. A denial is fed
back to the agent as information, not treated as an error — it adapts and keeps
going. "Allow for this session" grants until you quit; "always allow in this
project" writes a line to `.agents/permissions.toml` (shown to you) so the rule
sticks next time.

**Permission modes.** `Shift-Tab` (or `/mode`) cycles how much the agent may do
without asking: **normal** (ask per the rules), **auto-accept edits** (file
writes inside the project auto-apply; commands still ask), and **auto** (all
tools auto-run inside the project root). The two auto tiers require active OS
confinement — without a kernel fence they simply aren't offered, and the app
tells you why. The current mode shows in the status bar.

**Sessions never disappear.** Every session is written to
`.agents/sessions/<id>.jsonl` as it happens (durably, line by line). If Emberly
crashes or is killed, the next launch offers to resume. You can also:

- `emberly resume` — continue the most recent session
- `emberly resume <id>` — continue a specific one
- `emberly sessions` — list them, newest first, each with its resume command
- `/session` (in-app) — pick one from a menu and switch without leaving

**Long sessions.** Use `/compact` to summarize the older part of the
conversation when the context fills up; recent messages are kept verbatim and
the summary is recorded in the transcript, so resuming still works.

## Command-line reference

```
emberly                     Start (or offer to resume) an interactive session
emberly --plain             Run in plain line mode (no full-screen TUI)
emberly resume [id]         Resume the latest session, or one by id
emberly sessions            List saved sessions in this project
emberly init                Scaffold .agents/ (config, prompts, permissions)
emberly config show         Show the resolved configuration and its sources
emberly --version           Print the version

  --provider <name>         Override the provider for this run (anthropic|openai)
  --model <name>            Override the model for this run
```

Plain mode is also selected automatically when output isn't a terminal or
`NO_COLOR`/`TERM=dumb` are set — so Emberly degrades gracefully over pipes and
minimal terminals.

## Safety & transparency

Emberly is built around auditability and bounded action, in two layers — a
**rule engine** (when to ask) beneath an **OS sandbox** (what is possible):

- **Permission rule engine.** Every tool call resolves to allow / ask / deny,
  most-specific rule first, layering built-in defaults → global config →
  project `.agents/permissions.toml` → in-session grants. A default allowlist
  lets harmless read-only commands run without nagging; anything that writes, or
  anything outside the project root, asks. A denial is fed back to the agent as
  data, not an error.
- **OS sandbox.** Spawned commands run under kernel-level confinement — **Linux
  Landlock** (a self-exec shim, no `unsafe`) and **macOS Seatbelt** (via
  `sandbox-exec`, no C FFI). Writes are confined to the project root, `.git/` is
  read-only except for genuine `git`, and outside-root access is refused. The
  harness process itself is never confined — only its children. Where a platform
  has no backend (or it's disabled), Emberly runs an honest, clearly-labeled
  **degraded mode**: the allowlist is suspended (every command asks), the auto
  modes are locked, and the hard lines are enforced at the tool layer instead of
  the kernel. The active status is always shown in the UI.
- **`.git` protection.** Belt and braces: the file tools refuse `.git/` writes
  regardless of the sandbox, and the sandbox enforces it at the kernel when
  active.
- **Complete audit trail.** The append-only JSONL transcript is the ground
  truth — never rewritten — recording prompts, model output, every tool call
  and result, and every permission decision. A crashed or killed session is
  offered for resume on next launch.

## For developers

Cargo workspace, six crates, strictly one-way dependency flow
(`emberly` → {`tui`, `core`}; `tui` → `core`; `core` → {`providers`, `tools`,
`sandbox`}):

| Crate | Responsibility |
|---|---|
| `emberly-core` | Engine: agent loop, event model, sessions, context management |
| `emberly-providers` | `Provider` trait + Anthropic and OpenAI-compatible clients |
| `emberly-tools` | `Tool` trait + built-in tool suite |
| `emberly-sandbox` | Permission rule engine + OS confinement (security-critical) |
| `emberly-tui` | `ratatui` terminal frontend |
| `emberly` | Thin binary: CLI, wiring, supervisor |

**Guarantees enforced in code and CI**

- **Safe Rust in first-party code** (HC-1): every crate carries
  `#![forbid(unsafe_code)]`.
- **Panic-free core** (HC-3): `core`, `providers`, `tools`, and `sandbox`
  additionally `#![deny(clippy::unwrap_used, clippy::expect_used)]`.
- **No C dependencies in the default build** (HC-2): pure-Rust tree, `rustls`
  over OpenSSL, git CLI over `libgit2`; fully static musl targets.
  `cargo deny` bans the canonical C offenders.

**Working on it**

```sh
cargo build                                # whole workspace
cargo test                                 # test suite
cargo clippy --all-targets --all-features  # lint gate (HC-1/HC-3)
cargo fmt --all                            # formatting
```

Release targets (v1): `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, `aarch64-apple-darwin` (best-effort
`x86_64-apple-darwin`). Windows is deferred (no Landlock/Seatbelt equivalent).

**Documents**

- [`docs/emberly-code-requirements.md`](docs/emberly-code-requirements.md) — WHAT and WHY
- [`docs/emberly-code-design-guideline.md`](docs/emberly-code-design-guideline.md) — how it looks, feels, speaks
- [`docs/emberly-code-tech-spec.md`](docs/emberly-code-tech-spec.md) — HOW it is built
- [`docs/phase1/IMPLEMENTATION_PLAN.md`](docs/phase1/IMPLEMENTATION_PLAN.md) — phased build plan (+ per-phase `PHASE*_TODO.md`)

## License

Apache-2.0.
