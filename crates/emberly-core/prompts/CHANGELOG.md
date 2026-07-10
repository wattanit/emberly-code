# Emberly default prompts — changelog

These are the baked-in default prompts, embedded at build time (`src/prompts.rs`
via `include_str!`) and overridable per project under `.agents/prompts/`. They
are versioned **independently of the crate**: `prompts::VERSION` is stamped into
every session's `session_start` transcript record, so a session (or a resume)
records which prompt set produced it. Bump `VERSION` whenever a prompt changes
and add an entry here.

## v3 — 2026-07-11
- Added `tool_explanation.md` (T-9, Tech Spec §5.4): the instruction telling the
  model to fill the optional `explanation` field on a tool call, briefly and
  only for non-obvious calls. It is **not** part of `system.md`; it is appended
  to the system prompt only when `ui.tool_explanations` is on, so with the
  feature off neither the instruction nor the schema property is sent (no tokens
  spent). `system.md`/`compact.md` unchanged.

## v2 — 2026-07-07
- Replaced the initial placeholder prompts with the first real working set
  (owner-authored, commit `cda9bb4`):
  - `system.md` — Emberly Code identity + working method (understand before
    acting; small verifiable steps; solve only the asked task; footprint).
  - `compact.md` — structured compaction into fixed sections, written for the
    agent that continues the work rather than a human reader.

## v1 — 2026-07-07
- Initial extraction of the built-in prompts to files:
  - `system.md` — the agent system prompt (was a const in `emberly`'s config).
  - `compact.md` — the `/compact` summarization prompt (was a const in the engine).
