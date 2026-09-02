# Emberly Code — Implementation Plan (distribution & licensing)

**Status:** 🚧 Phase 0 closed and approved; Phases 1–4 not started.
**Date:** 2026-09-02
**Owner:** Wattanit
**Source documents** (G-14 pin):
- Requirements Document v0.12 (`docs/emberly-code-requirements.md`) —
  unchanged by this scope, pinned for reference only.
- Design Guideline v0.12 (`docs/emberly-code-design-guideline.md`) —
  unchanged by this scope, pinned for reference only.
- Technical Specification v0.15 (`docs/emberly-code-tech-spec.md`) — HOW —
  **Status: approved** (2026-09-02) — covers §13 Build & Release (license,
  crates.io, Homebrew, cargo-dist) and the v0.15 entry in §16 Open Items.

This plan expands Technical Specification §13 with the project's first
public distribution channels (crates.io, a Homebrew tap) and a license
change (Apache-2.0 metadata → AGPL-3.0-or-later, dual-licensed). It is
**not a new milestone** — §16's v0.15 entry records why: this scope
graduates no FR-n/HC-n/P-n/T-n requirement, changes no product behavior,
and so doesn't fit the M1–M13 pattern of "a feature set that proves a
scope." For the same reason this plan is filed at `docs/distribution/`
rather than `docs/version-X-Y/` — it isn't tied to an app feature-release
version bump. Both calls were made explicitly by the owner (2026-09-02),
recorded here rather than left implicit.

---

## Guiding principles (apply to every phase)

- **The friction is deliberate, not accidental.** AGPL was chosen
  specifically so that embedding this code — by cloning the repo, by
  `cargo add`-ing a published crate, or by running the Homebrew-installed
  binary — carries the same copyleft obligation everywhere. No channel in
  this plan is built to be a quieter or lower-friction path around it.
- **No name-squatting, on any crate.** Every crate published in Phase 2 is
  the real, functioning code. A placeholder/reservation-only stub was
  considered and explicitly rejected — occupying a name with no real
  content behind it isn't something this project does even for its own
  benefit.
- **Publish is a one-way door.** crates.io has no delete, only yank; a
  `--dry-run` precedes every real `cargo publish`, and each publish is
  shown before it runs, not fired silently.
- **Homebrew ships binaries, never triggers a source build.** This matches
  Requirements' pure-Rust, no-C-toolchain story (HC-2) instead of
  undercutting it for the one distribution channel most likely to reach a
  non-Rust-developer audience.
- **Nothing here touches engine, tool, or sandbox behavior.** This plan is
  packaging, publishing, and licensing only. `emberly-sandbox` and every
  other crate's actual code is out of scope.

---

## Phase 0 — Foundation document updates (prerequisite, not engineering) — ✅ closed 2026-09-02

**Goal:** Get the Technical Specification to reflect the license and
distribution-channel decisions before any execution starts.

**Closed.** Tech Spec bumped 0.14 → 0.15: §13 Build & Release expanded
with the license (AGPL-3.0-or-later, dual-licensed), the crates.io
publish-order constraint, and the Homebrew tap/cargo-dist decision; §16
Open Items gained the v0.15 entry. Requirements and Design Guideline
companion-version pins refreshed to v0.15 (G-16) — neither document's own
content changed, since nothing about WHAT the harness does or how it
looks/feels/speaks was decided here.

**Done when:** Tech Spec v0.15 exists with the §13/§16 content — ✅ done.
Owner approved v0.15 on 2026-09-02 with no requested changes — ✅ done.
Phase 1 may begin.

---

## Phase 1 — License fixes

**Goal:** Make the license real. A `LICENSE` file has never existed for
this project despite `Cargo.toml` claiming Apache-2.0.

**Gated on:** Phase 0's Tech Spec v0.15 approval — ✅ cleared 2026-09-02.

**Scope:**
- `[workspace.package] license` → `AGPL-3.0-or-later` — a single edit,
  since every crate inherits it via `license.workspace = true`.
- `[workspace.package] repository` (currently `""`) → the real GitHub URL.
- Add `LICENSE` at the repo root: full AGPLv3 text.
- README: a short "Commercial licensing" section describing how to reach
  the owner for a proprietary exception, alongside the existing install
  instructions.

**Done when:** `cargo metadata` reports `AGPL-3.0-or-later` for all six
crates; `LICENSE` present and textually correct; README changes reviewed
by the owner.

---

## Phase 2 — Registry-ready dependencies & crates.io publish

**Goal:** All six workspace crates published for real, so `cargo install
emberly` and `cargo add emberly-core` (etc.) work — and so the name
`emberly` is reserved by actual use, not a stub.

**Gated on:** Phase 1 complete (crates.io requires correct license
metadata before it will accept a publish).

**Scope:**
- Add `version = "0.5.2"` (or whatever the release version is at publish
  time) alongside every internal `path = "../x"` dependency — crates.io
  rejects bare `path` dependencies.
- Prerequisite: a crates.io account and API token, `cargo login` run
  locally once (token never committed).
- `cargo publish --dry-run -p <crate>` before every real publish.
- Publish order, forced by the internal dependency graph:
  1. `emberly-providers`, `emberly-sandbox`, `emberly-tools` (no internal
     dependency between them — any order)
  2. `emberly-core` (depends on all three above)
  3. `emberly-tui` (depends on `emberly-core`)
  4. `emberly` (depends on all five above)

**Done when:** all six crates resolve on crates.io at the published
version; `cargo install emberly` succeeds from a clean environment with no
local path overrides; a fresh `cargo add emberly-core` in a scratch
project resolves from the registry.

---

## Phase 3 — cargo-dist cross-platform binaries

**Goal:** Every tagged release produces prebuilt binaries for the §13
release targets as GitHub Release artifacts.

**Scope:**
- cargo-dist config targeting `x86_64-unknown-linux-musl`,
  `aarch64-unknown-linux-musl`, `aarch64-apple-darwin` (best-effort
  `x86_64-apple-darwin`) — the same target list §13 already commits to;
  this plan introduces no new target.
- A release-triggered CI workflow (tag push), kept separate from the
  existing PR-gated `ci.yml` checks.

**Done when:** a test tag produces a GitHub Release with binaries for
every listed target, each downloadable and passing `emberly --version`.

---

## Phase 4 — Homebrew tap

**Goal:** `brew install` works with no Rust toolchain required on the
user's machine.

**Gated on:** Phase 3 (needs real release artifacts to point at).

**Scope:**
- Create a personal tap repo (`homebrew-emberly`, same GitHub account) —
  not a `homebrew-core` submission; §13 already rules that out (review-gate
  timeline, notability bar) as unrealistic at this stage.
- cargo-dist generates/updates the tap's formula, `url`/`sha256` pointed at
  the Phase 3 release artifacts, kept in sync on every tagged release.

**Done when:** `brew tap <owner>/emberly && brew install emberly` succeeds
on a clean macOS machine (arm64 at minimum) and `emberly --version`
matches the tagged release.

---

## Deliberately out of scope for this plan

- **CLA tooling.** §13 makes a CLA a prerequisite before merging any
  external contribution, but no tooling (e.g. a `cla-assistant` gate) is
  built here — there are no external contributors yet.
- **Reserving the five sub-crate names beyond what Phase 2 does.** Phase 2
  publishes them for real anyway (they're real dependencies of `emberly`),
  so this is moot for those five specifically — noted in §16 only because
  the underlying question (reserve names with no real crate behind them)
  was considered and rejected as a general practice, not because these
  five needed a special decision.
- **Windows.** Still deferred per §13/Requirements §13 — no Landlock/
  Seatbelt equivalent; unrelated to this plan.
