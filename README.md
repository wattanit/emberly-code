# Emberly Code

An interactive AI coding agent for the terminal — the coding-agent sibling of
Emberly. The user states tasks in natural language; the agent reads, edits,
and creates files, runs commands, and iterates until the task is done or the
user intervenes.

Emberly Code exists for three reasons, in priority order: **full source
control** (the entire harness is owned, auditable, and modifiable in-house),
**safety and transparency** (every action is bounded by an enforceable
permission and sandbox model and recorded in a complete audit trail), and
**stability** (reliable on standard and non-standard stacks, including
musl/static deployments). It is provider-agnostic by design.

> **Status:** pre-v1, under active development. Not yet interactive.

## Documents

- [`docs/emberly-code-requirements-v0.3.md`](docs/emberly-code-requirements-v0.3.md) — WHAT and WHY
- [`docs/emberly-code-design-guideline-v0.3.md`](docs/emberly-code-design-guideline-v0.3.md) — how it looks, feels, speaks
- [`docs/emberly-code-tech-spec-v0.1.md`](docs/emberly-code-tech-spec-v0.1.md) — HOW it is built
- [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) — phased build plan
- [`PHASE1_TODO.md`](PHASE1_TODO.md) — current phase breakdown and progress

## Workspace layout

Cargo workspace, six crates, strictly one-way dependency flow
(`emberly` → {`tui`, `core`}; `tui` → `core`; `core` → {`providers`,
`tools`, `sandbox`}):

| Crate | Responsibility |
|---|---|
| `emberly-core` | Engine: agent loop, event model, session, context management |
| `emberly-providers` | `Provider` trait + Anthropic and OpenAI-compatible clients |
| `emberly-tools` | `Tool` trait + built-in tool suite |
| `emberly-sandbox` | Permission rule engine + OS confinement (security-critical) |
| `emberly-tui` | `ratatui` terminal frontend |
| `emberly` | Thin binary: CLI, wiring, supervisor |

## Guarantees enforced in code and CI

- **Safe Rust in first-party code** (HC-1): every crate carries
  `#![forbid(unsafe_code)]`.
- **Panic-free core** (HC-3): `core`, `providers`, `tools`, and `sandbox`
  additionally `#![deny(clippy::unwrap_used, clippy::expect_used)]`.
- **No C dependencies in the default build** (HC-2): pure-Rust tree, `rustls`
  over OpenSSL, git CLI over `libgit2`; fully static musl targets.
  `cargo deny` bans the canonical C offenders.

## Building

```sh
cargo build            # debug build of the whole workspace
cargo clippy --all-targets --all-features  # lint gate (HC-1/HC-3)
cargo test             # test suite
```

Release targets (v1): `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`, `aarch64-apple-darwin` (best-effort
`x86_64-apple-darwin`). Windows is deferred (no Landlock/Seatbelt equivalent).

## License

Apache-2.0.
