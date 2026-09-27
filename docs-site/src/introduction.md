# Introduction

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

## Where to go next

- New to Emberly? Start with [Installation](./installation.md) and the
  [Quick Start](./quickstart.md).
- Setting up providers, keys, or MCP servers? See [Configuration](./configuration/index.md).
- Want the full command and keybinding reference? See
  [Interface & Commands](./guide/interface-and-commands.md) and the
  [CLI Reference](./cli-reference.md).
