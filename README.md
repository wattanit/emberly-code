# Emberly Code

> **Currently on v0.5.3** — feature-complete for the milestone, install via Homebrew, `cargo install`, or a prebuilt binary.

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

📖 **Full documentation:** https://wattanit.github.io/emberly-code/ — install,
configuration, the interface & command reference, safety model, and more.

---

## Contents

- [Why Emberly Code](#why-emberly-code)
- [Documentation](#documentation)
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
skill system, web search, multi-agent delegation, and mouse support. All
keyboard-first; the mouse only ever adds convenience, never new authority.

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

## Documentation

The full user guide now lives on the docs site — install methods, every
configuration option (providers, MCP servers, the complete config-key
reference), the interface and command/keybinding reference, the safety model
(permissions, sandbox, trust), reasoning effort, memory & skills, multi-agent
delegation, sessions, and the CLI reference:

**https://wattanit.github.io/emberly-code/**

Its source lives under [`docs-site/`](docs-site/) (built with
[mdBook](https://rust-lang.github.io/mdBook/)). Run it locally:

```sh
cargo install mdbook
cd docs-site && mdbook serve -o
```

To get going right away:

```sh
brew install wattanit/emberly/emberly     # or: cargo install emberly
export EMBERLY_PROVIDER=anthropic
export EMBERLY_MODEL=claude-sonnet-5
export ANTHROPIC_API_KEY=sk-...
cd ~/my-project && emberly
```

---

## Developer guide

### Project status

Feature-complete for the **v0.5.3** milestone, installable from source.
The interactive TUI, live providers, session persistence, the permission rule
engine, auto-accept modes, and OS confinement (Linux Landlock, macOS Seatbelt)
all work today, alongside the full 0.2–0.5.3 stack described below. **Not yet
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
| **v0.4.5** | — | 🌲🔥 _Wildfire_ | Another correctness pass. A session switch left the derived view cache stale, so returning to a session reported "0 in / 0 out" at $0.00 despite full history (#18). A tool call whose backend streamed no usable arguments (empty, the literal text `null`, or garbage) surfaced as a bare `invalid type: null` and could stall a session with no explanation — arguments now default to `{}`, and the model is told plainly which call was bad and to retry. Provider request failures and retries are now written to the session transcript instead of only flashing in the UI. Added: `--version` reports a build timestamp; the completion stream's first-chunk/idle timeouts are configurable (`[stream]`, closing the #15 config deferral) for slower local/cloud inference backends; `Ctrl-L` forces a full repaint as an interim escape hatch for display corruption reported on independent terminals (Ghostty, Termius, Termux), cause unconfirmed. |
| **v0.5.0** | M12 | 🔥 _Bonfire_ | Multi-agent delegation — the primary agent can spawn, message, list, and end subagents, each a real nested engine running under the exact same permission, sandbox, and workspace-trust posture as the primary agent, with its own selectable provider profile and a tool set that's never a superset of the primary agent's own. Concurrent by default, bounded to one level of depth (no recursive spawning), a configurable concurrency ceiling and idle reap, cost roll-up into the session total, and a per-agent inspector (sidebar Agents section, `/agents` command, permission-prompt provenance line) so no subagent is a silent background process. |
| **v0.5.1** | M13 | 🔥 _Bonfire_ | Three independent slices: **MCP client support** (`[mcp.servers.*]`, `/mcp`, a first-party stdio JSON-RPC client — no vendor SDK — with discovered tools permission-gated and provenance-labeled exactly like a built-in tool's); **user-attached images** (`/attach <path>`, reusing the existing vision content-block path `read_image` already produces); and **session export** (`/export <path>` / `emberly export`, a read-only, self-contained HTML render of a session and any subagent it spawned). |
| **v0.5.2** | — | 🔥 _Bonfire_ | A usability pass on the terminal and the CLI, no new capability. The chat and input panels keep only their top/bottom rule now, so a terminal-selected transcript copies cleanly instead of picking up border glyphs; a typed `/command` is colored live as you type it (recognized, still-ambiguous, or unresolvable) and Tab-completes when exactly one command matches; a fresh session opens with a short orientation instead of a blank pane; `/init` brings the CLI's `.agents/` scaffolding in-session; the `.agents/config.toml` template is now a clearly divided, fully-commented section per built-in provider instead of one flat block; a denied tool call now tells the model to ask you rather than spend the turn hunting for a workaround; and `emberly --help`/`-h`, plus per-subcommand help (`init`, `sessions`, `resume`, `config`, `trust`, `clean`, `export`), finally document the CLI's own surface. |
| **v0.5.3** | M14 | 🔥 _Bonfire_ | Drag-to-select-and-copy: dragging over the conversation pane with no modifier held highlights the text under the pointer and copies it to the clipboard on release via an OSC 52 escape sequence, with a "Copied N characters." notice — replacing the fragile Shift-drag-then-Ctrl+C flow (native Shift-drag still works as the documented fallback). The status bar's hint line now also mentions Shift-Tab's existing mode-cycle binding. Plus a fix: a session's auto-generated title now reaches the sidebar live from the first message, instead of showing "untitled session" until the next `/resume`. |

Prior as-built plans live under `docs/version-0-1/`, `docs/version-0-2/`,
`docs/version-0-3/`, `docs/version-0-4/`, `docs/version-0-4-1/`,
`docs/version-0-4-2/`, `docs/version-0-4-3/`, `docs/version-0-5/`, and
`docs/version-0-5-1/`.

### Architecture

Cargo workspace, six crates, strictly one-way dependency flow
(`emberly` → {`tui`, `core`, `providers`, `tools`, `sandbox`} — the
composition root wires the live providers/tools and drives the self-exec
sandbox shim, so it depends on all five directly; `tui` → `core` only;
`core` → {`providers`, `tools`, `sandbox`}):

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

- [`docs/emberly-code-requirements.md`](docs/emberly-code-requirements.md) — WHAT and WHY (v0.13)
- [`docs/emberly-code-design-guideline.md`](docs/emberly-code-design-guideline.md) — how it looks, feels, speaks (v0.13)
- [`docs/emberly-code-tech-spec.md`](docs/emberly-code-tech-spec.md) — HOW it is built (v0.17)
- [`docs/version-0-5-1/IMPLEMENTATION_PLAN.md`](docs/version-0-5-1/IMPLEMENTATION_PLAN.md) — v0.5.1's phased build plan (v0.5.2 was a smaller, informal polish pass with no foundation-document revision; v0.5.3 bumped the foundation documents for drag-to-select-and-copy but likewise shipped without a dedicated Implementation Plan)
- [`docs/distribution/IMPLEMENTATION_PLAN.md`](docs/distribution/IMPLEMENTATION_PLAN.md) — the license change and distribution-channel plan below

## License

**AGPL-3.0-or-later.** Free to use, modify, and redistribute — including as
part of a commercial product — as long as your own project stays under a
compatible copyleft license, source included. Running a modified version as
a network service counts as distribution under AGPL: if you do, your users
are entitled to that version's source too.

**Commercial licensing.** If AGPL's terms don't work for your use case —
you want to embed Emberly Code in a closed-source product without those
obligations — a separate commercial license is available. Open an issue to
start that conversation.
