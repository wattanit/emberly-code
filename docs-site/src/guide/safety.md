# Safety: Permissions, Sandbox & Trust

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
