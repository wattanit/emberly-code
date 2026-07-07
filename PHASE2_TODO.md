# Phase 2 — Safety Story — TODO & Progress

**Milestone:** M2 (Tech Spec §15) — *Landlock confinement + rule engine +
degradation policy + escape tests.*
**Goal:** Prove the safety story. OS-level containment on Linux, the full
rule engine, the honest degradation policy, and the escape-test suite that
gives HC-4/HC-5 regression teeth.

**Depends on:** Phase 1 complete (engine, tools, gate, line frontend).

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 0. Prerequisites & dependencies | [x] | deps signed off + added; `SandboxStatus`/`Mode` moved to sandbox, re-exported from core |
| 1. Rule engine | [x] | `Decision`/`Rule`/`RuleEngine` in `emberly-sandbox`; 16 unit tests |
| 2. Sandbox probe, status & degradation | [~] | status type, degradation policy, one-time notice, `sandbox.require` all done + tested; **Linux Landlock ABI detection deferred to group 3** (inseparable from the confinement path per the `landlock` crate's design) |
| 3. Landlock child confinement | [ ] | **post-checkpoint**; needs shim spike |
| 4. Genuine-git resolution & `.git` profile | [ ] | post-checkpoint (Linux/CI) |
| 5. Process-group tree-kill (P1 deferral) | [x] | `rustix` group SIGKILL via drop-guard; `/proc` liveness test |
| 6. Wire rules + sandbox into gate/engine | [~] | rule half done (gate consults engine; grants; matched-rule reason). Sandbox-handle half is group 3 |
| 7. Modes | [x] | `Mode::resolve` gates auto tiers on confinement; `SetMode` wired; ModeChanged event+transcript |
| 8. Glob + grep tools | [x] | `globset`/`ignore`/`grep-searcher`; root-confined, skip `.git/`, honor `.gitignore`; tested |
| 9. Escape-test suite & exit criterion | [ ] | post-checkpoint (Linux/CI) |

**Overall Phase 2: foundation complete (groups 0–2, 5, 6-rules, 7, 8).**
Remaining: the Landlock spike (group 3), genuine-git `.git` profile (group 4),
and the escape-test suite (group 9) — all gated on the checkpoint review.

### Foundation checkpoint state (this session)

Built on Linux (kernel 6.17, Landlock active in the LSM list), so the real
confinement path is achievable next — but at this checkpoint the binary still
runs the **honest degraded path**: `probe()` reports `Unavailable` because
children are not yet confined (group 3), so the bash allowlist is suspended and
auto modes are locked. All of that machinery is built and tested; group 3 flips
the probe to the confined status without touching the degradation policy.

- **Deps added (owner-signed-off, HC-2 clean):** `landlock` (Linux-gated in
  `emberly-sandbox`), `rustix` (`process` feature, unix-gated in
  `emberly-tools`), `ignore` + `globset` + `grep-searcher` + `grep-regex`. All
  transitive additions are pure-Rust; the C-dep ban stays green.
- **Tests:** +6 sandbox rule/mode/status unit tests, +6 engine integration
  tests (silent allow, auto-deny, mode gating, session grant), +1 tree-kill
  test, +5 glob/grep tests. Full workspace suite green; clippy clean.
- **Mode semantics (owner-decided):** the allowlist is non-destructive bash +
  read-only tools, so it auto-runs in **Normal** too. **Auto-accept edits** adds
  the file write/edit tools. **Auto** auto-runs *all* tools inside the project
  root — safe because Auto requires active confinement, so the kernel enforces
  root-confinement and the `.git` line the rule layer stops asking about.
  Outside-root always asks (HC-4) and `Deny` rules always deny, in every mode.
  The whole tier policy is one function: `rules::mode_auto_allow`.

---

## Platform note (read first)

The dev machine is **macOS (aarch64-apple-darwin)**, where **Landlock does not
exist** (macOS confinement via Seatbelt is Phase 5). So in Phase 2, on this
machine the sandbox probe returns **Unavailable** and the harness runs the
**degraded path**. Practical consequences:

- **Testable locally (macOS):** the rule engine (group 1), the degradation
  policy (group 2), process-group kill (group 5), modes gating (group 7),
  glob/grep (group 8), and the degraded-environment escape assertions.
- **Linux/CI only:** actual Landlock confinement (groups 3–4) and the
  positive escape tests (write-outside-root fails, `.git` write fails, genuine
  git succeeds). CI must run on a **Landlock-enabled kernel** with the LSM
  active — verify on the GitHub `ubuntu-latest` runner and, if the LSM is off,
  add a self-hosted or container step (group 9).

Write the Linux-only code behind clean `#[cfg(target_os = "linux")]` seams so
the crate still builds and the degraded path still runs everywhere.

---

## 0. Prerequisites & dependencies  *(§10 policy; Tech Spec §1, §12)*

- [ ] Add `landlock` (Linux, pure-Rust syscall wrapper — HC-2 clean) to
      `emberly-sandbox`, gated `#[cfg(target_os = "linux")]`. **Requires owner
      sign-off** (sandbox deps are frozen harder, Tech Spec §1).
- [ ] Add a syscall crate for process-group signaling (group 5) — prefer
      `rustix` (pure-Rust `linux_raw` backend, no libc → HC-2 clean) over
      `nix` (libc). Lands in `emberly-tools` (bash) or `emberly-sandbox`;
      decide placement in group 5. Requires dependency acceptance (§10).
- [ ] Add `ignore` + `globset` (and reuse `grep-searcher` path) for glob/grep
      (group 8) — pure-Rust ripgrep libraries.
- [ ] Record all additions in `cargo vet`/`deny`; keep the C-dep ban green.
- [ ] **Type relocation:** move `SandboxStatus` from `emberly-core::types` to
      `emberly-sandbox` (the probe produces it; core → sandbox) and re-export
      from core, mirroring the `ToolCallId`/`TokenUsage` pattern. Update
      `UiEvent`/`TranscriptEvent` references (should be transparent via
      re-export).

## 1. Rule engine  *(§6.1, §6.2, §6.5, §6.6; Tech Spec §6.1)*

- [ ] `emberly-sandbox`: `Decision { Allow, Ask, Deny }` and a `Rule =
      (tool, matcher) -> Decision`, evaluated **most-specific-first**.
- [ ] Matchers: bash **prefix** matchers on the command string (explicitly
      convenience-tier, no shell parsing — §6.5); path/tool matchers for
      read/write/edit.
- [ ] Precedence, in order: built-in defaults → global config
      (`~/.config/emberly/`) → project `.agents/permissions.toml` →
      session grants (in-memory).
- [ ] Default bash allowlist (Tech Spec §6.1): `ls cat head tail wc rg find
      pwd echo which git-status git-diff git-log git-show git-branch
      cargo-check cargo-tree cargo-metadata`.
- [ ] Default file rules: reads in-root **allow**; writes/edits in-root
      **ask**; anything outside root **ask, always** (never allow via rule).
- [ ] Session grants ("allow for this session") held in memory; "always allow
      in project" writes a line to `permissions.toml` (and returns the written
      line to show the user, §6.6).
- [ ] Pure logic, no OS calls → unit-tested on any platform.

## 2. Sandbox probe, status & degradation  *(§6.7; Tech Spec §6.5, §6.2 probe)*

- [ ] `SandboxStatus` (relocated here): `Confined{backend}` /
      `Partial{backend,missing}` / `Unavailable{reason}`.
- [ ] Startup probe: Linux → query Landlock ABI (cheap syscall), record
      granted-vs-requested; non-Linux → `Unavailable{"no landlock on <os>"}`.
- [ ] Emit `SandboxStatus` (UiEvent) + write into `session_start`; a **one-time
      plain-language notice** on absence/partial (Design §8.2), warning styling.
- [ ] Degradation policy (§6.5), principle *make risk clear, tighten
      convenience, stay usable*: when unavailable/partial-missing-core →
      **suspend the bash allowlist** (every bash asks), **lock auto modes**
      (group 7), keep per-action prompting normal, label tool-layer protection
      as **policy-level** not kernel-level.
- [ ] `sandbox.require = true` config → refuse to start without kernel
      confinement (default `false`).
- [ ] Never crash on unavailability; confinement applies to children only.

## 3. Landlock child confinement  *(HC-4, HC-5, S-1; Tech Spec §6.2)* — Linux/CI

- [ ] **Design spike first:** apply Landlock to children **without `unsafe`
      pre_exec** (HC-1). Leading approach: a **self-exec sandbox shim** — a
      hidden `emberly` subcommand that calls `landlock` `restrict_self()`
      (safe) then `CommandExt::exec()` (safe; Landlock is inherited across
      `execve`). Validate before building the rest of the group.
- [ ] Build the ruleset (best-effort ABI, record what was granted): project
      root **read+write**; `.git/` under root **no write**; system paths
      (`/usr /lib /etc`, toolchain) **read+exec**; everything else **no
      access**.
- [ ] Approved outside-root paths (HC-4 ask) added to that **single
      invocation's** ruleset only.
- [ ] The harness process is **never** confined — only spawned children.
- [ ] `emberly-sandbox` exposes a confined-spawn API the bash tool uses via a
      `tools`-side trait (group 6), so `tools` stays independent of `sandbox`.

## 4. Genuine-git resolution & `.git`-write profile  *(HC-5, §6.3; Tech Spec §6.4)* — Linux/CI

- [ ] Record the canonical git binary at session start (`which git`,
      canonicalized).
- [ ] A command qualifies for the `.git/`-writable profile **only if**: first
      token is `git` AND PATH-resolves to a canonical binary **outside** the
      project root that **matches** the recorded git binary.
- [ ] `./git` in-repo, PATH shadowing inside the project, and `sh -c "git …"`
      wrappers do **not** qualify (safe-closed: they run without `.git` write).
- [ ] File tools keep their hard `.git` refusal (Phase 1) — belt and braces.

## 5. Process-group tree-kill  *(S-4; picks up the Phase 1 bash deferral)*

- [ ] Using the syscall crate from group 0, kill the **whole process group**
      (`kill(-pgid, …)`) on timeout/cancel, not just the leader — reaps
      grandchildren of `sh -c`.
- [ ] Keep the Phase 1 `process_group(0)` setup + `kill_on_drop` fallback.
- [ ] Update the bash timeout/cancel tests to assert descendants die (spawn a
      child that outlives its parent; confirm it is gone).

## 6. Wire rules + sandbox into the gate/engine  *(§6; Tech Spec §5.1, §6.6)*

- [ ] The engine gate (Phase 1 always-prompt) now consults the **rule engine**:
      `Allow` → run silently (no prompt, no event churn); `Ask` → prompt as
      today; `Deny` → auto-deny with the reason.
- [ ] `ToolCtx` gains the **sandbox handle** (Tech Spec §5.1) via a
      `tools`-side trait implemented by core/sandbox (mirrors the gate); bash
      spawns children **through it** (confined on Linux, plain when degraded).
- [ ] Session grants + `permissions.toml` writes wired through the gate; every
      request/decision/what-actually-ran is transcript-ready (persistence still
      Phase 5).
- [ ] `build_rendering` reason now reflects the **matched rule** (not the
      Phase 1 placeholder text).

## 7. Modes  *(§6.4; Tech Spec §6.6)*

- [ ] `Mode` transitions take `SandboxStatus` as a parameter so the **type
      system** forbids constructing an auto mode without confinement.
- [ ] `Command::SetMode` handling: Normal / AutoAcceptEdits / Auto. Auto tiers
      change what the gate auto-allows (edits; allowlisted + session-granted
      bash).
- [ ] Auto modes **unavailable** when degraded (group 2); the mode selector /
      status shows why. Mode change is a transcript event.

## 8. Glob + grep tools  *(T-5, T-6; Tech Spec §5.2)*

- [ ] `glob` (`globset`): root-confined, ignores `.git/`, honors `.gitignore`
      by default.
- [ ] `grep` (`grep-searcher`/`ignore`): first-party wrapper, root-confined.
- [ ] Register both in `builtin::default_registry`. Output truncated at
      ingestion like the others. → tests (any platform).

## 9. Escape-test suite & exit criterion  *(HC-4, HC-5; Tech Spec §14.3)*

- [ ] CI job on a **Landlock-enabled** kernel (verify `ubuntu-latest`; add a
      container/self-hosted step if the LSM is off).
- [ ] Positive escape tests (Linux): write **outside root** → fails; write
      `.git/` via file tool **and** via bash → fails; fake/aliased/in-repo
      `git` → does **not** get `.git` write; **genuine** `git commit` →
      succeeds.
- [ ] Degraded-environment test (Landlock-disabled container **and** macOS):
      status shown, auto modes locked, allowlist suspended, per-action
      prompting still works, hard lines labeled policy-level.
- [ ] `log`/surface any coverage a platform can't run (no silent gaps).

---

## Phase 2 exit criterion (from IMPLEMENTATION_PLAN.md)

> The escape-test suite passes on a Landlock-enabled CI kernel and produces the
> exact §6.5 behavior in a Landlock-disabled container.

- [ ] **Exit criterion met.**

---

## Notes / decisions log

*(Pre-seeded with decisions to confirm during Phase 2; add outcomes as work
proceeds.)*

- **Owner sign-off needed (Tech Spec §1):** adding `landlock` to
  `emberly-sandbox`. Frozen-harder dependency list — confirm before adding.
- **Landlock-without-`unsafe` (HC-1):** leading approach is a self-exec
  sandbox shim (`restrict_self()` + safe `exec()`), since `pre_exec` is
  `unsafe`. This is a spike in group 3 — validate the mechanism before
  building groups 4/6 on top of it. Fallback options if the shim is
  unworkable: accept a narrowly-scoped `unsafe` in a dedicated, separately
  audited module (would need an explicit HC-1 exception decision), or a
  small vetted helper crate.
- **Syscall dep for tree-kill:** prefer `rustix` (pure-Rust `linux_raw`, HC-2
  clean) over `nix` (libc). Confirm in group 0/5.
- **`SandboxStatus` relocation:** moves core → sandbox with a re-export, like
  `ToolCallId`/`TokenUsage` in Phase 1.
- **macOS = degraded during Phase 2:** Seatbelt is Phase 5, so local runs
  exercise the degraded path only; Landlock confinement is proven in Linux CI.
- **Tools stay independent of sandbox:** the confined-spawn capability reaches
  the bash tool via a `tools`-side trait implemented by core/sandbox — the
  same pattern as the permission gate (Phase 1 group 3).
