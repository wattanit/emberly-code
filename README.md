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

   Emberly ships baked-in profiles — `anthropic`, `openai`, `zai` (the Z.ai
   coding plan), and `local` (a keyless `http://localhost:11434/v1` for
   Ollama/vLLM). Selecting one is just its name plus a key; each profile's key
   comes from `<PROFILE-REF>_API_KEY` (or `keys.toml`):

   ```sh
   export EMBERLY_PROVIDER=zai
   export EMBERLY_MODEL=glm-4.6
   export ZAI_API_KEY=...
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
provider = "zai"                # a baked-in profile, or one you define below
model    = "glm-4.6"
```

**Adding a provider is configuration, not code.** A provider is a *profile*
naming a wire-format `adapter` (`anthropic` or `openai`), an endpoint, and a
key *reference*. Any service that speaks a format Emberly already parses is
reachable this way — no rebuild:

```toml
[providers.myserver]
adapter  = "openai"                          # or "anthropic"
base_url = "https://my-endpoint.example/v1"
auth     = { scheme = "bearer", key = "myserver" }   # → MYSERVER_API_KEY

# Optional per-model metadata → live session cost estimate:
# [providers.myserver.models."my-model"]
# context_window = 128000
# max_output     = 8192
# pricing = { input = 1.0, output = 2.0 }    # USD per million tokens
```

Baked-in profiles (`anthropic`, `openai`, `zai`, `local`) can be tweaked the
same way — set just the field you want to change; the rest is kept.

**API keys** are read from `<REF>_API_KEY` (e.g. `ANTHROPIC_API_KEY`,
`ZAI_API_KEY`) or from `~/.config/emberly/keys.toml` — a flat `ref = "secret"`
table that must be mode `0600`. Keys are never read from project files, so they
don't get committed.

**Project instructions:** an `AGENTS.md` (or `CLAUDE.md`) at your project root
is picked up automatically and given to the agent as standing context.

**Environment variables:** `EMBERLY_PROVIDER` (active profile) and
`EMBERLY_MODEL`. For display, `NO_COLOR` or `TERM=dumb` force plain mode and
`EMBERLY_MOTION=0` disables animation.

Run **`emberly config show`** to see the resolved settings and where each value
came from.

**Editing config and prompts from a session.** You don't have to leave Emberly
to tune it:

- **`/config`** opens `.agents/config.toml` in your `$EDITOR` (`$VISUAL` is
  tried first). If the project has none yet, it's created from the template.
- **`/prompt [system|compact]`** opens a prompt file, seeded from the baked-in
  default if you have no override yet. Edits are written to the **project
  tier** (`.agents/`), never to the global config or the built-in defaults,
  and Emberly tells you whether you're editing an existing value or creating a
  new override.
- On save, the change is **applied to the running session** — a new system
  prompt takes effect on your next message, and a newly-added provider profile
  shows up in the `/model` picker right away. Startup-only settings (e.g.
  `sandbox.require`) are named as needing a restart rather than applied
  silently. **`/reload`** re-reads everything on demand (handy if you edited a
  file outside Emberly).
- In plain/`--plain` mode there's no editor handoff: `/config` and `/prompt`
  print the file path to edit with your own tools, and `/reload` applies it.

**Reasoning effort and the thinking trail.** For models that expose a
reasoning control, you own the latency/cost/quality trade per task:

- Enable it per model in config with an `effort` default (and optionally an
  `effort_levels` subset) — see `.agents/config.toml`. A model with no such
  control ignores the setting; it's never an error.
- **`/effort [low|medium|high|max]`** sets the level for your next message —
  no argument opens a picker. The engine maps it to the provider's native
  knob (Anthropic's thinking budget, OpenAI's `reasoning_effort`). The active
  level shows in the sidebar; switching model re-seeds it to that model's
  default. Every change is announced and recorded to the transcript.
- When a provider streams its **reasoning** distinctly from the answer, it
  renders as a quiet, collapsed trail — `▸ reasoning (N lines)` — one step
  below the answer, never mistaken for it. **Ctrl-R** toggles it open. Set the
  default view with the **`reasoning`** config key: `collapsed` (default),
  `expanded`, or `hidden`. `hidden` only hides it from view — the trace is
  still recorded to the transcript. In plain mode the trail is a labeled
  `--- reasoning ---` block.

**Asking you a question, and tool-call explanations.** Two touches that make
the agent's work legible without getting in the way:

- **The agent can ask you.** When it genuinely needs your decision or
  information it can't get itself, it puts a calm question on screen and waits
  — offering choices when it has them, with a free-text answer always
  available. It is deliberately *not* the permission prompt: there is no
  "safe default" keypress, so **Enter never answers for you**; you pick an
  option or type a reply, and **Esc** declines (the agent is told you declined,
  so it can proceed or stop). In plain mode the same question appears with
  numbered options; an empty line declines.
- **Non-obvious tool calls get a one-line caption.** When what a call does
  isn't self-evident (an opaque `bash` command, a subtle edit), the model
  writes a short dim line under it — `↳ raise the log level to info`. Obvious
  calls get none (no clutter). It's **on by default**; set
  `ui.tool_explanations = false` to turn it off entirely — with it off the
  model is never asked for one, so no tokens are spent on it.

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
| `/mode` | `Shift-Tab` | Pick a permission mode (`Shift-Tab` cycles) |
| `/sidebar` | `Ctrl-B` | Toggle the sidebar |
| `/cancel` | | Cancel the in-flight turn |
| `/quit` | `Ctrl-D` | Exit |

**Permission prompts.** When the agent wants to run a command or change a file,
it shows exactly what it will do and asks you to allow or deny. A denial is fed
back to the agent as information, not treated as an error — it adapts and keeps
going. "Allow for this session" grants until you quit; "always allow in this
project" writes a line to `.agents/permissions.toml` (shown to you) so the rule
sticks next time.

**Permission modes.** `Shift-Tab` cycles how much the agent may do without
asking, and `/mode` (or the palette) opens a picker listing all three tiers:
**normal** (ask per the rules), **auto-accept edits** (file writes inside the
project auto-apply; commands still ask), and **auto** (all tools auto-run
inside the project root). You can also set one directly with
`/mode <normal|auto-accept-edits|auto>`. The two auto tiers require active OS
confinement — without a kernel fence they aren't offered (the picker marks them
and tells you why), and the app tells you why. The current mode shows in the
status bar.

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

  --provider <name>         Select the active provider profile for this run
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
- **Workspace trust.** The first time you run Emberly in a folder it hasn't
  seen, it asks — in plain language, before reading any project file or starting
  the agent — whether you trust the code there. The safe default is *decline*
  (declining starts no session); trusting is a deliberate choice. Trust is
  remembered per folder and **extends to its subtree**, so you're asked once per
  project, not once per directory. It is stored globally
  (`~/.config/emberly/trust.toml`, `0600`) — a repository can never pre-declare
  itself trusted. Pre-approve folders with `trust.trusted_dirs` in your *global*
  config; manage grants with **`emberly trust list`** and **`emberly trust
  revoke <path>`**. Trust is a *consent gate, not containment*: it decides
  whether Emberly runs here, never what it may do — every permission prompt and
  sandbox rule still applies.
- **Loop-breaking guardrail.** If the agent starts spinning — repeating the same
  steps without changing anything — Emberly halts it and hands the decision back
  to you rather than burning tokens indefinitely: **keep going**, **stop**, or
  **say something** to steer. It never quietly resumes or quietly gives up. It
  only trips on genuine no-progress (a loop that keeps changing files or getting
  new results is left alone), and it's tunable — `[loop]` `enabled`,
  `repeat_window`, `max_no_progress_turns`.
- **Complete audit trail.** The append-only JSONL transcript is the ground
  truth — never rewritten — recording prompts, model output, every tool call
  and result, every permission decision, the trust decision, and any loop halt.
  A crashed or killed session is offered for resume on next launch.

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
