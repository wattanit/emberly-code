# Emberly Code

> **Currently on v0.4.4** — feature-complete for the milestone, install from source.

**An AI coding agent for your terminal — provider-agnostic, fully auditable, and
built in pure Rust.**

Tell Emberly what you want in plain words, and it goes to work in your codebase —
reading, editing, running commands, and correcting course as it goes. It asks
before it acts and writes down everything it does, so you stay in the loop and
never have to guess what happened.

Bring your own model: Anthropic, any OpenAI-compatible endpoint, or something
running on your own machine. Every session is a complete, resumable transcript,
and the whole thing ships as a single self-contained binary — no C, no runtime,
nothing else to install.

<!-- A dedicated landing page + docs site are planned; this README will slim
     down once they exist. Until then it is the single source of truth. -->

---

## Contents

- [Why Emberly Code](#why-emberly-code)
- [User guide](#user-guide)
  - [Install](#install)
  - [Configuration](#configuration)
  - [Commands & features](#commands--features)
- [Developer guide](#developer-guide)
- [License](#license)

---

## Why Emberly Code

### 🦀 Pure Rust — no C, no surprises
The entire default dependency tree is pure Rust: `rustls` (RustCrypto) instead
of OpenSSL, the `git` CLI instead of `libgit2`, no C toolchain required to
build. Every first-party crate is `#![forbid(unsafe_code)]`, and `cargo deny`
bans the canonical C offenders in CI. The result is a single, fully-static
`musl` binary you can drop onto a box with nothing else installed.

### 🔍 Fully transparent & auditable
Nothing happens off the record. Every session is an **append-only JSONL
transcript** — the ground truth, never rewritten — capturing prompts, model
output, every tool call and its result, every permission decision, the trust
decision, and any loop halt. `emberly config show` tells you not just the
resolved settings but **where each value came from**. Memory the agent stores
and skills it can invoke are **inspectable before they ever run**. What the
agent did, and why, is always reconstructable.

### 🧰 A complete agentic harness
Not a thin wrapper around a chat API — a real coding harness: a two-pane TUI,
multi-provider support, durable sessions with crash-safe resume, a permission
rule engine beneath an OS sandbox, reasoning-effort control and a thinking
trail, a live agent task list, image input, persistent cross-session memory, a
skill system, web search, and mouse support. All keyboard-first; the mouse only
ever adds convenience, never new authority.

### 📉 Built-in token optimiser & better economy
Emberly treats your context window and API bill as scarce resources. Large tool
outputs are **reduced to their salient parts** (with a `recall` tool to pull
back the full text on demand); older turns are **windowed and auto-compacted**
as the context fills; a live per-session **cost estimate** keeps the meter
visible. You get longer, cheaper sessions without babysitting the context.

> 📊 **Benchmarks landing soon.** _(Placeholder — real measured numbers to be
> dropped in.)_ Empirically, Emberly ships as a **~[BINARY SIZE]** static
> binary with a **~[RSS] resident footprint**, and its context-economy layer
> cuts token usage by roughly **[X]%** and API cost by roughly **[Y]%** on
> representative multi-tool sessions versus sending full context every turn.

---

## User guide

### Install

No prebuilt binaries yet — build from source (needs a recent stable Rust
toolchain; **no C toolchain required**):

```sh
git clone <this-repo> && cd emberly-code
cargo build --release
# the binary is target/release/emberly — put it on your PATH, e.g.:
install -m 0755 target/release/emberly ~/.local/bin/emberly
```

**Quick start:**

1. **Pick a provider and give it a key.** Emberly ships baked-in profiles —
   `anthropic`, `openai`, `zai` (the Z.ai coding plan), and `local` (a keyless
   `http://localhost:11434/v1` for Ollama/vLLM). Selecting one is just its name
   plus a key:

   ```sh
   export EMBERLY_PROVIDER=anthropic
   export EMBERLY_MODEL=claude-sonnet-5
   export ANTHROPIC_API_KEY=sk-...
   ```

2. **Run it in your project directory:**

   ```sh
   cd ~/my-project
   emberly
   ```

3. Type your task and press **Enter**. Emberly asks before running commands or
   editing files; you approve or deny each action.

The first time you run Emberly in a folder it will ask whether you **trust** the
code there (the safe default is *decline*). Without any provider configured,
Emberly still starts in an **offline placeholder mode** so you can explore the
interface. To keep settings with a project instead of in your shell, run
`emberly init`.

### Configuration

Everything is optional — Emberly runs on built-in defaults. Settings resolve in
this order, later winning over earlier:

1. Built-in defaults
2. Global config — `~/.config/emberly/config.toml` (or `$XDG_CONFIG_HOME/emberly/`)
3. Project config — `.agents/config.toml` in your project
4. `EMBERLY_*` environment variables
5. `--provider` / `--model` command-line flags

Run **`emberly init`** to scaffold `.agents/` with a commented `config.toml`,
the default prompts (editable, under `.agents/prompts/`), a `permissions.toml`
template, and a `.gitignore` that keeps session transcripts out of version
control. Existing files are never overwritten. Run **`emberly config show`** to
see the resolved settings and where each value came from.

#### Providers, models, and keys

```toml
provider = "zai"                # a baked-in profile, or one you define below
model    = "glm-4.6"
```

**Adding a provider is configuration, not code** — a profile names a wire-format
`adapter` (`anthropic` or `openai`), an endpoint, and a key *reference*. The
easiest way in is the **guided setup wizard**: open the model picker
(`/model`) and pick the trailing "+ add new provider…" row — it walks you
through a name, adapter, endpoint, model id, and key (masked as you type),
then writes both files for you. Raw editing below remains the fallback for
anything the wizard's fixed field set doesn't cover (per-model metadata, a
non-standard auth scheme):

```toml
[providers.myserver]
adapter  = "openai"                          # or "anthropic"
base_url = "https://my-endpoint.example/v1"
auth     = { scheme = "bearer", key = "myserver" }   # → MYSERVER_API_KEY

# Optional per-model metadata → live cost estimate, /effort, and image/document input:
[providers.myserver.models."my-model"]
context_window = 128000
max_output     = 8192
pricing        = { input = 1.0, output = 2.0 }   # USD per million tokens
effort         = "medium"        # default reasoning level; enables /effort
effort_levels  = ["low", "medium", "high"]       # optional subset
vision         = true            # model accepts image input (P-11)
documents      = true            # model accepts document/PDF input (P-12)
```

Baked-in profiles can be tweaked the same way — set just the field you want to
change; the rest is kept. **API keys** are read from `<REF>_API_KEY` (e.g.
`ANTHROPIC_API_KEY`, `ZAI_API_KEY`) or from `~/.config/emberly/keys.toml` (a
flat `ref = "secret"` table, mode `0600`). Keys are never read from project
files, so they don't get committed.

**Project instructions:** an `AGENTS.md` (or `CLAUDE.md`) at your project root
is picked up automatically as standing context.

#### Config keys at a glance

| Key | Default | What it controls |
|---|---|---|
| `reasoning` | `collapsed` | Thinking-trail view: `collapsed` / `expanded` / `hidden` |
| `[ui] tool_explanations` | `true` | One-line captions under non-obvious tool calls |
| `[ui] mouse` | `true` | Wheel-scroll + click-to-select in the rich TUI |
| `[trust] trusted_dirs` | `[]` | Folders pre-approved for the trust gate (global config only) |
| `[loop] enabled` | `true` | Loop-breaking guardrail (+ `repeat_window`, `max_no_progress_turns`) |
| `[completion] enabled` | `true` | Completion gate: holds the loop to registered checks before "done" (+ `max_attempts`, `[[completion.check]]`) |
| `[context] window_turns` | _auto_ | Adaptive context window (+ `keep_recent_turns`) |
| `[context] auto_compact` | `true` | Auto-summarize older turns (+ `auto_compact_threshold`) |
| `[context] pin_task_list` | `true` | Keep the agent's task list pinned in context |
| `[truncate] reduce` | `true` | Salient reduction of tool output (+ `max_lines`/`max_bytes`/`head_lines`/`tail_lines`) |
| `[image] max_bytes` | _limit_ | Max size for an image read into the conversation |
| `[document] max_bytes` | _limit_ | Max size for a PDF read into the conversation |
| `[memory] enabled` | `true` | Persistent cross-session memory (+ `max_index_entries`) |
| `[skills] enabled` | `true` | The skill system |
| `[search] enabled` | `true` | Register the `web_search` tool (needs `adapter`/`endpoint`/`auth` to work; `max_results` caps results) |
| `[stream] first_chunk_secs` | `300` | Seconds to wait for a completion stream's first chunk (+ `idle_secs`, default `90`, for the gap between later chunks) — raise both for a slow local/cloud inference backend |
| `[sandbox] require` | `false` | Refuse to start without active OS confinement |

**Environment variables:** `EMBERLY_PROVIDER`, `EMBERLY_MODEL`. For display,
`NO_COLOR` or `TERM=dumb` force plain mode and `EMBERLY_MOTION=0` disables
animation.

#### Editing config and prompts from a session

You don't have to leave Emberly to tune it. **`/config`** opens
`.agents/config.toml` in your `$EDITOR`; **`/prompt [system|compact]`** opens a
prompt file (seeded from the baked-in default). Edits write to the **project
tier** only, and Emberly tells you whether you're editing an existing value or
creating a new override. On save the change is **applied to the running
session**; startup-only settings are named as needing a restart. **`/reload`**
re-reads everything on demand. In plain mode there's no editor handoff — the
commands print the path to edit yourself, and `/reload` applies it.

Adding a provider has its own guided path: from `/model`, the trailing "+ add
new provider…" row opens a step-by-step wizard (name → adapter → endpoint →
model id → key), ending in a summary screen with the key redacted to its last
4 characters before it writes anything. It writes the profile into the same
project `.agents/config.toml` and the key into `~/.config/emberly/keys.toml`,
then reloads exactly like a manual edit would — no separate success message,
no auto-switch (pick the new profile from `/model` afterward). Not available
in plain mode, which falls back to the same print-the-path-and-`/reload`
pattern as `/config`/`/prompt` above (no `$EDITOR` handoff either way in
plain mode).

### Commands & features

The screen is a conversation timeline — your messages and the agent's, tool
calls, and diffs — with a sidebar showing the model, context usage, cost, the
agent's task list, and changed files.

**Input & keybindings**

| Key | Action |
|---|---|
| `Enter` | Send your message |
| `Shift+Enter` / `Alt+Enter` | Newline (compose a multi-line message) |
| `Esc` | Cancel the current turn / dismiss an overlay |
| `Ctrl-C` | Cancel the current turn; quits when idle and the input line is empty |
| `↑` `↓` `PgUp` `PgDn` / mouse wheel | Scroll the conversation or an overlay |
| `Ctrl-R` | Toggle the reasoning trail open/closed |
| `Ctrl-P` | Open the command palette |
| `Ctrl-L` | Force a full repaint (clears display corruption without resizing) |
| Mouse click | Select an interactive row / affordance (never approves a permission) |

**Commands** — open the palette with **`Ctrl-P`**, or type any `/name`:

| Command | Key | What it does |
|---|---|---|
| `/new` (`/clear`) | | Start a fresh session (the current one is saved) |
| `/session` | | List saved sessions and switch to one |
| `/compact` | | Summarize older turns to reclaim context space |
| `/model` | | Switch the active provider/model (`/model <profile>` direct) |
| `/mode` | `Shift-Tab` | Pick a permission mode (`Shift-Tab` cycles) |
| `/effort` | | Set reasoning effort (`/effort low\|medium\|high\|max`) |
| `/view` | | View the last assistant message in full |
| `/diff` | `Ctrl-O` | Open the latest file's diff |
| `/files` | | List files changed this session |
| `/sidebar` | `Ctrl-B` | Toggle the sidebar |
| `/cancel` | | Cancel the in-flight turn |
| `/config` | | Edit `.agents/config.toml` in `$EDITOR` |
| `/prompt` | | Edit a prompt file (`/prompt system\|compact`) |
| `/reload` | | Re-read config & prompts from disk and apply them |
| `/memory` | | Inspect, edit, and delete stored memory |
| `/skills` | | List available skills and inspect a skill's instructions |
| `/help` | `Ctrl-P` | List commands and keybindings |
| `/quit` | `Ctrl-D` | Exit |

#### Safety: permissions, sandbox, and trust

Emberly is built around auditability and bounded action, in two layers — a
**rule engine** (when to ask) beneath an **OS sandbox** (what is possible):

- **Permission modes.** `Shift-Tab` (or `/mode`) cycles how much the agent may
  do without asking: **normal** (ask per the rules), **auto-accept edits** (file
  writes inside the project auto-apply; commands still ask), and **auto** (all
  tools auto-run inside the project root). The two auto tiers **require active OS
  confinement** — without a kernel fence they aren't offered, and the app tells
  you why.
- **Permission rule engine.** Every tool call resolves to allow / ask / deny,
  most-specific rule first, layering built-in defaults → global config → project
  `.agents/permissions.toml` → in-session grants. Read-only commands run without
  nagging; anything that writes, or anything outside the project root, asks. A
  denial is fed back to the agent as data, not an error.
- **OS sandbox.** Spawned commands run under kernel-level confinement — **Linux
  Landlock** (a self-exec shim, no `unsafe`) and **macOS Seatbelt** (via
  `sandbox-exec`, no C FFI). Writes are confined to the project root, `.git/` is
  read-only except for genuine `git`, and outside-root access is refused. The
  harness process itself is never confined — only its children. Where a platform
  has no backend, Emberly runs an honest, clearly-labeled **degraded mode**: the
  allowlist is suspended (every command asks) and the auto modes are locked.
- **Workspace trust.** The first time you run Emberly in an unfamiliar folder it
  asks — before reading any project file — whether you trust the code there.
  Trust is remembered per folder, extends to its subtree, and is stored globally
  (`~/.config/emberly/trust.toml`, `0600`) so a repository can never pre-declare
  itself trusted. Manage grants with `emberly trust list` / `emberly trust
  revoke <path>`, or pre-approve via `[trust] trusted_dirs` in global config.
- **Loop-breaking guardrail.** If the agent starts spinning without making
  progress, Emberly halts it and hands the decision back to you — **keep going**,
  **stop**, or **say something** to steer — rather than burning tokens.
- **Completion gate.** The sibling guardrail: if you register pass/fail checks
  (`[[completion.check]]` — e.g. a test suite), the agent can't declare a task
  done while they fail. A failing check re-opens the loop as feedback; after
  repeated failures Emberly halts and asks you to keep going, steer, stop, or
  **finish anyway** (an explicit override, never presented as passing).
  Inert until you register a check.

#### Reasoning effort & the thinking trail

For models that expose a reasoning control, you own the latency/cost/quality
trade per task. Enable it per model with an `effort` default (see config above).
**`/effort`** sets the level for your next message; the engine maps it to the
provider's native knob (Anthropic's thinking budget, OpenAI's
`reasoning_effort`). When a provider streams its **reasoning** distinctly, it
renders as a quiet, collapsed `▸ reasoning (N lines)` trail; **Ctrl-R** toggles
it open. The `reasoning` config key sets the default view (`hidden` still
records the trace to the transcript).

#### Persistent memory & skills

- **Memory (`/memory`).** Emberly keeps a small, progressive-disclosure memory
  store the agent can write to and recall across sessions — durable facts about
  you and the project. The `/memory` overlay lets you inspect, edit, and delete
  entries grouped by scope. Nothing is hidden: what the agent remembers is always
  yours to review.
- **Skills (`/skills`).** A skill is a folder with a `SKILL.md` the model can
  invoke on demand (user-global or project-local, project winning on a name
  clash). `/skills` lists what's available and lets you **read a skill's full
  instructions before it ever runs** — "what could this tell the model to do" is
  always inspectable.

#### Web search & image/document input

- **Web search.** The `web_search` tool is registered by default but does
  nothing until you point it at a search service — set `[search]` `adapter`
  (`brave` / `tavily` / `searxng` / `json`), `endpoint`, and `auth`. The agent
  then searches through your harness-owned endpoint (capped by `max_results`,
  default 5). Set `enabled = false` to remove the tool entirely.
- **Image input.** For vision-capable models (`vision = true`), the agent can
  read an image file inside your project into the conversation via the
  `read_image` tool — point it at a screenshot or diagram and ask about it.
- **Document input.** For document-capable models (`documents = true`), the
  agent can read a PDF file inside your project into the conversation via the
  `read_document` tool — the same pattern as image input, applied to
  documents. The harness never parses the PDF; it just forwards the bytes, so
  there's no page count or extracted text, only size and format.

#### Sessions & long conversations

Every session is written to `.agents/sessions/<id>.jsonl` as it happens
(durably, line by line). If Emberly crashes or is killed, the next launch offers
to resume. You can also:

- `emberly resume` — continue the most recent session
- `emberly resume <id>` — continue a specific one
- `emberly sessions` — list them, newest first, each with its resume command
- `/session` (in-app) — pick one from a menu and switch without leaving

For long sessions the context-economy layer works automatically; use `/compact`
to manually summarize the older part when the window fills. Recent messages are
kept verbatim and the summary is recorded in the transcript, so resuming works.

Each session also gets a disposable scratch directory
(`.agents/scratch/<id>/`) the agent can stash temporary files in — a script,
intermediate output, a working note — via the `scratch_write` tool. It's
harness-managed (the model supplies content, never a path) so it costs no
permission prompt, and it's gitignored so it never lands in your tracked
project. Nothing auto-deletes it; reclaim the disk space with `emberly clean`.

#### Command-line reference

```
emberly                     Start (or offer to resume) an interactive session
emberly --plain             Run in plain line mode (no full-screen TUI)
emberly resume [id]         Resume the latest session, or one by id
emberly sessions            List saved sessions in this project
emberly init                Scaffold .agents/ (config, prompts, permissions)
emberly config show         Show the resolved configuration and its sources
emberly trust list          List trusted folders
emberly trust revoke <path> Revoke trust for a folder
emberly clean [<id>]        Reclaim scratch-space disk usage (all, or one session)
emberly --version           Print the version

  --provider <name>         Select the active provider profile for this run
  --model <name>            Override the model for this run
```

Plain mode is selected automatically when output isn't a terminal or
`NO_COLOR`/`TERM=dumb` are set — so Emberly degrades gracefully over pipes and
minimal terminals.

---

## Developer guide

### Project status

Feature-complete for the **v0.4.4** milestone, installable from source.
The interactive TUI, live providers, session persistence, the permission rule
engine, auto-accept modes, and OS confinement (Linux Landlock, macOS Seatbelt)
all work today, alongside the full 0.2–0.4.4 stack described below. **Not yet
shipped:** prebuilt binaries and Windows support (no Landlock/Seatbelt
equivalent).

### Release history

Each release stacks a new layer onto the last — so the nicknames follow a fire
as it grows. _(Affectionate, not official.)_

| Version | Milestones | Nickname | What it added |
|---|---|---|---|
| **v0.1** | M1–M5 | 🪵 _Kindling_ | The harness itself — the six crates, the agent loop, both live providers, the full TUI, sessions / resume / compaction, the permission rule engine, and OS confinement on Linux + macOS. |
| **v0.2** | M6 | 🏡 _Hearth_ | Configurable and safe to live with — endpoint-configurable provider profiles (incl. Z.ai), in-app config/prompt editing, reasoning effort + thinking trail, the ask-you-a-question tool, tool-call explanations, workspace trust, and the loop-breaking guardrail. |
| **v0.3** | M7 | 🔥 _Slow Burn_ | The economy layer — salient tool-result reduction, the adaptive context window + `recall`, automatic + manual compaction, and the derived resume cache. Longer, cheaper sessions from the same fuel. |
| **v0.4** | M8 | 🌲🔥 _Wildfire_ | New capability surface — planning (task list), sight (image input), durable memory, extensible skills, live web search, and pointer interaction. |
| **v0.4.1** | M9 | 🌲🔥 _Wildfire_ | A completion gate that holds the loop to registered pass/fail checks before it may declare a task done, and document (PDF) input — the same pattern as image input, applied to documents. |
| **v0.4.2** | M10 | 🌲🔥 _Wildfire_ | Guided provider/model setup — a step-by-step wizard, reachable from the model picker, that writes a new provider profile and its key without hand-editing config. Plus a round of post-ship hardening: compaction, cancellation, permission previews, and parallel tool-call handling. |
| **v0.4.3** | M11 | 🌲🔥 _Wildfire_ | Three externally-reported bug fixes (a stalled SSE stream could hang forever; an interrupted turn could commit a message no provider adapter accepts; the token estimate badly undercounted Thai/CJK text), plus session scratch space — a disposable per-session working directory (`scratch_write`, `emberly clean`). |
| **v0.4.4** | — | 🌲🔥 _Wildfire_ | Correctness and internals, no new features. Three defects in the permission rule engine (a saved `always allow` could write a `permissions.toml` that no longer parsed, silently dropping every project rule including `deny`s; a `tool = "*"` rule could override a named-tool `deny`; `match = "*"` matched nothing instead of everything), three in the provider streaming seam, and a slow first token no longer trips the idle timeout (#15). Internally: the `Engine` and `App` god objects split by topic and their flat field lists grouped, one generic gate with a single fail-closed rule, and the unused outside-root grant path removed from the sandbox (Tech Spec v0.12). |

Prior as-built plans live under `docs/version-0-1/`, `docs/version-0-2/`,
`docs/version-0-3/`, `docs/version-0-4/`, `docs/version-0-4-1/`, and
`docs/version-0-4-2/`.

### Architecture

Cargo workspace, six crates, strictly one-way dependency flow
(`emberly` → {`tui`, `core`}; `tui` → `core`; `core` → {`providers`, `tools`,
`sandbox`}):

| Crate | Responsibility |
|---|---|
| `emberly-core` | Engine: agent loop, event model, sessions, context management, memory, skills |
| `emberly-providers` | `Provider` trait + Anthropic and OpenAI-compatible clients |
| `emberly-tools` | `Tool` trait + built-in tool suite (files, bash, search, image, recall, …) |
| `emberly-sandbox` | Permission rule engine + OS confinement (security-critical) |
| `emberly-tui` | `ratatui` terminal frontend (rich + plain line mode) |
| `emberly` | Thin binary: CLI, wiring, supervisor |

**Guarantees enforced in code and CI**

- **Safe Rust in first-party code** (HC-1): every crate carries
  `#![forbid(unsafe_code)]`.
- **Panic-free core** (HC-3): `core`, `providers`, `tools`, and `sandbox`
  additionally `#![deny(clippy::unwrap_used, clippy::expect_used)]`.
- **No C dependencies in the default build** (HC-2): pure-Rust tree, `rustls`
  over OpenSSL, git CLI over `libgit2`, fully static `musl` targets; `cargo
  deny` bans the canonical C offenders and CI proves the build graph is C-free.

**Working on it**

```sh
cargo build                                              # whole workspace
cargo test --workspace --all-features                    # test suite
RUSTFLAGS="-D warnings" \
  cargo clippy --workspace --all-targets --all-features  # lint gate (HC-1/HC-3)
cargo fmt --all -- --check                               # formatting
```

> The CI `clippy` gate runs with `-D warnings` and `--all-targets`, so
> `unwrap`/`expect` (including in tests) and `too_many_arguments` are hard
> errors. Follow the repo convention in tests: `panic!`/`let-else`, never
> `.unwrap()`/`.expect()`.

Release targets (v1): `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, `aarch64-apple-darwin` (best-effort
`x86_64-apple-darwin`). Windows is deferred (no Landlock/Seatbelt equivalent).

**Documents** (the SFD standard — Requirements → Design → Tech Spec):

- [`docs/emberly-code-requirements.md`](docs/emberly-code-requirements.md) — WHAT and WHY (v0.10)
- [`docs/emberly-code-design-guideline.md`](docs/emberly-code-design-guideline.md) — how it looks, feels, speaks (v0.10)
- [`docs/emberly-code-tech-spec.md`](docs/emberly-code-tech-spec.md) — HOW it is built (v0.12)
- [`docs/version-0-4-3/IMPLEMENTATION_PLAN.md`](docs/version-0-4-3/IMPLEMENTATION_PLAN.md) — phased build plan (+ per-phase `PHASE*_TODO.md`)

## License

Apache-2.0.
