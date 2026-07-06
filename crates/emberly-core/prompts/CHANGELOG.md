# Emberly default prompts — changelog

These are the baked-in default prompts, embedded at build time (`src/prompts.rs`
via `include_str!`) and overridable per project under `.agents/prompts/`. They
are versioned **independently of the crate**: `prompts::VERSION` is stamped into
every session's `session_start` transcript record, so a session (or a resume)
records which prompt set produced it. Bump `VERSION` whenever a prompt changes
and add an entry here.

## v1 — 2026-07-07
- Initial extraction of the built-in prompts to files:
  - `system.md` — the agent system prompt (was a const in `emberly`'s config).
  - `compact.md` — the `/compact` summarization prompt (was a const in the engine).
