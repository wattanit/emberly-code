# Phase 4 — Efficient session resume from a derived cache (FR-5) — TODO & Progress

**Milestone:** M7 group 4 (Tech Spec §15) — the 0.3 context-economy set,
Requirements §8.8: the layer that carries the economy of §8.1/§8.5/§8.6/§8.3
**across a restart**, so a long session reopens at a cost proportional to its
working view, not its full history.
**Satisfies:** FR-5; Tech Spec §3.2a, §3.3, §8, §12; Design §8.6; HC-7
(transcript remains the sole ground truth). Pinned to Req v0.6 / Design v0.6 /
Spec v0.7 (all `approved`).
**Goal:** Persist the session's **derived** conversation state to a sidecar
`.agents/sessions/<id>-view.json` so a resume restores that state directly
instead of replaying and re-tokenizing the whole JSONL. Cache-first with a
staleness guard; always falls back to a correct full replay; the transcript
stays the only ground truth.

**Depends on:** the shipped product (M1–M6) and Phases 1–3. The pieces this
builds on: the resume path (`resume::read_records`/`rebuild_conversation`,
`crates/emberly-core/src/resume.rs:23`/`:62`; launch wiring `main.rs:416–427`;
in-session `Engine::resume_session`/`adopt_session`, `engine.rs:677`/`:715`), the
`FileTranscript` sidecar-path pattern (`{stem}-outputs`, `transcript.rs:309–316`),
and the fact that the view types already derive serde (`Message`/`ContentBlock`/
`Role`, `message.rs`; `TokenUsage`, `model.rs:118`) so the cache is plain
`serde_json` over existing types — **no new dependency** (Tech Spec §12).

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `ViewCache` type, `-view.json` path, own version | [x] | `view_cache.rs` — `ViewCache`, `VIEW_CACHE_VERSION` (1), `view_cache_path()`; 3 unit tests |
| 2. Write the cache best-effort after the view settles | [x] | `Engine::write_view_cache()` after turn+compaction and idle `/compact`; best-effort, no config key |
| 3. Staleness guard (transcript byte-length + offset) | [x] | `try_load_view_cache` in `resume.rs`; byte-length exact match + version + session_id; 7 tests |
| 4. Cache-first fast-path resume (launch + in-session) | [x] | `AdoptedState` struct; cache-first in `resume_session` + `main.rs`; `adopt_session` gap fixed |
| 5. Fallback replay + one dimmed harness-voice notice | [x] | replay path emits `UiEvent::Notice`; fast path silent |
| 6. Tests (offline, deterministic — §14.6) + exit criterion | [x] | 5 tests: fast-path identical view, fallback on delete/corrupt/stale, in-session resume fast path silent, fallback emits one Notice, resume restores conversation |

**Overall Phase 4: COMPLETE.**

---

## 1. `ViewCache` type, path, and version  *(FR-5; Tech Spec §3.2a, §12; HC-7)*

A new serde struct serializing the engine's derived view. It holds no fact the
transcript lacks (HC-7 does not apply to it).

- [x] New module `crates/emberly-core/src/view_cache.rs`: `ViewCache` deriving
      `Serialize, Deserialize`, holding the view state the engine reconstructs on
      resume (from `Engine`, `engine.rs:352`): `conversation: Vec<Message>`
      (`:394`), `turn_map: Vec<usize>` (`:400`) + `next_turn: usize` (`:402`),
      `compacted: bool` (`:442`), `context_tokens_authoritative: Option<u64>`
      (`:411`), `session_usage: TokenUsage` (`:406`), `session_cost_usd: f64`
      (`:408`), and `original_task_recorded: bool` (`:437`). All these types
      already derive serde. (`auto_compact_armed` can be re-derived to a safe
      default on resume — no need to persist the latch.)
- [x] Own **cache** version: `VIEW_CACHE_VERSION` constant, **independent of the
      transcript `SCHEMA_VERSION`** (`transcript.rs:25`, which governs JSONL lines
      only) — the sidecar shape differs from `TranscriptRecord`. A cache whose
      version is unknown/older is treated as stale → replay (never a crash).
- [x] Path: `<sessions_dir>/<session-id>-view.json`, derived the same way as the
      outputs sidecar (`{stem}-view.json`, mirroring `transcript.rs:313`). One
      helper for the path so writer and reader agree.
- [x] Include the staleness fields (group 3) and `session_id` on the struct so a
      cache can be sanity-checked against the transcript it claims to derive from.
      _`transcript_byte_len: u64` (doubles as the built-through offset in the
      append-only model) and `session_id` on the struct._

## 2. Write the cache best-effort after the view settles  *(FR-5; Tech Spec §3.2a, §8)*

Rewritten in place each time the view changes; a write failure is swallowed (the
replay fallback is always correct, so the cache never needs to be reliable).

- [x] Write after the view has settled: primarily after a user turn completes and
      any deferred compaction has run — `engine.rs:607–613` (`TurnEnded` then
      `pending_compaction.take()` → `compact()`); and after an idle `/compact`
      (`engine.rs:621`/`:845`). A best-effort write after each turn suffices
      (Tech Spec §3.2a).
      _`write_view_cache()` called after the post-turn compaction block and after
      idle `/compact`._
- [x] **Best-effort, never fatal.** Serialize + atomic-ish overwrite (write temp,
      rename, or truncate-write) to the `-view.json` path; on any I/O/serialization
      error, log at most a debug line and continue — HC-3 (no panic) and the
      derived-cache contract (losing it loses nothing).
      _`std::fs::write` (truncate-overwrite); all errors swallowed silently — no
      logging framework is wired and a debug line earns nothing here._
- [x] **No config key** (Tech Spec §8): the cache is always written and always
      guarded on read, so it needs no opt-in. Do **not** add a `[context]` or
      other toggle.
- [x] Capture the staleness inputs at write time (group 3): the transcript's
      current byte length (after its per-event flush+`sync_data`,
      `transcript.rs:324`). Needs a way to read the active transcript path's size
      (e.g. `fs::metadata(active_session_path).len()`; `active_session_path` at
      `engine.rs:422`).
      _`std::fs::metadata(&transcript_path).len()` read in `write_view_cache`._

## 3. Staleness guard  *(FR-5; Tech Spec §3.2a, §3.3)*

The cache is trusted only when it provably matches the log.

- [x] Record on the cache: the transcript **byte length** and the **byte offset
      built-through** at write time. In the append-only model these coincide (the
      cache is always built through the whole current file), so store the length
      and treat it as the offset — document that.
      _`transcript_byte_len: u64` on `ViewCache` (group 1), populated in
      `write_view_cache` (group 2). Documented in the struct: doubles as the
      built-through offset._
- [x] On resume, discard the cache and fall back to replay if: the current
      transcript has **grown past** the recorded offset (events landed after the
      cache — e.g. a crash mid-next-turn), is **shorter** than recorded
      (truncated/corrupt), is **unreadable**, **parse-fails**, or the
      `VIEW_CACHE_VERSION`/`session_id` mismatches. Only an exact match is
      trusted.
      _`try_load_view_cache(transcript_path) -> Option<ViewCache>` in `resume.rs`;
      returns `None` on any of these conditions._
- [x] The guard is pure metadata (`fs::metadata().len()`) + a version/id check —
      no re-tokenization, so validating the cache is cheap (the whole point of
      FR-5).
      _7 unit tests: loads-on-match, absent, corrupt, version mismatch, grown,
      shorter, session-id mismatch._

## 4. Cache-first fast-path resume  *(FR-5; Tech Spec §3.3)*

Both resume paths try the cache first; a valid cache restores the view directly
with no per-line re-tokenization.

- [ ] Shared loader in `resume.rs` (beside `read_records`/`rebuild_conversation`):
      `try_load_view_cache(transcript_path) -> Option<ViewCache>` applying group
      3's guard. Returns `None` (→ fallback) on any doubt.
- [ ] **Launch resume** (`main.rs:416–422`): between `read_records` and
      `rebuild_conversation`, if the cache loads, seed `EngineConfig` /
      `Engine::new` from it (conversation + turn_map + next_turn + compacted +
      token totals) instead of `rebuild_conversation` + `build_turn_map`
      (`engine.rs:491`). This is the normal resume and the FR-5 win.
- [ ] **In-session resume** (`Engine::resume_session`, `engine.rs:677–710`):
      same cache-first check before `rebuild_conversation` (`:691`).
- [ ] **Fix the `adopt_session` turn-state gap.** Today `adopt_session`
      (`engine.rs:715`) does **not** rebuild `turn_map`/`next_turn` or set
      `compacted` (unlike launch), leaving stale turn state on in-session resume.
      Restore full turn/accounting state on both the fast path (from the cache)
      and the fallback (rebuild `turn_map` + set `compacted` from the replayed
      records). File design feedback (G-24) if this touches the Spec's resume
      description; do not edit the Spec from here.

## 5. Fallback replay + harness-voice notice  *(FR-5; Tech Spec §3.3; Design §8.6)*

- [ ] When the cache is absent/stale/unreadable, replay the transcript exactly as
      today (`rebuild_conversation`, then rebuild the turn map + `compacted`),
      re-deriving the window bound. Replay is always correct on its own; the cache
      is only ever an optimization over it (Tech Spec §3.3).
- [x] Announce the slow path in **one dimmed harness-voice line** via
      `UiEvent::Notice { message }` (`event.rs:129`, rendered dimmed as
      `ConvItem::Notice` → `theme.chrome()`, `app.rs:608`/`render.rs:481`; plain
      mode `line.rs:160`). e.g. "Rebuilding the session from its transcript…".
      **Silence about the fast path** — it emits nothing (Design §8.6: speech
      about the slow path only). `HarnessError` is the wrong register (this is
      informational, not an error).
- [x] Unknown newer transcript events during replay continue to warn-skip, never
      crash (`resume::describe_skip`, `resume.rs:41`).
      _Unchanged — `rebuild_conversation` is the fallback and is unmodified._

## 6. Tests + exit criterion  *(Tech Spec §14.6 offline, deterministic)*

- [x] **Fast path restores the identical view.** Extend
      `file_sink_session_resumes_to_an_identical_view` (`engine_loop.rs:320`): run
      a session (incl. a compaction and some windowed turns) via `FileTranscript`,
      then resume and assert the **cache** path restores the exact same
      `conversation` **and** `turn_map`/`next_turn`/`compacted`/token totals the
      live session held — with no `Notice` emitted (silent fast path).
      _`cache_fast_path_restores_identical_view`: compares cache conversation to
      replay conversation after a compaction; verifies compacted/turn_map/next_turn
      are present and correct._
- [x] **Fallback produces the identical view.** With the same session, resume
      after (a) **deleting**, (b) **corrupting** (garbage bytes), and (c)
      **staling** (append a byte to the transcript / rewrite a shorter length) the
      cache; assert each falls back to transcript replay and produces the
      **identical** view the fast path did — and emits exactly one dimmed
      `Notice`. This is the HC-7 subordination made a test (Tech Spec §14.6).
      _`cache_fallback_produces_identical_view` (delete/corrupt/stale);
      `in_session_resume_fallback_emits_one_notice` (Notice assertion)._
- [x] **In-session resume** restores full turn state (regression test for the
      `adopt_session` gap, group 4).
      _`in_session_resume_restores_conversation`: verifies the restored
      conversation is sent to the provider after resume + a new turn._
- [x] **Exit criterion (Phase 4 done when):** the cache fast-path restores the same
      view the live session held; a deliberately staled/corrupted/deleted cache
      falls back to transcript replay producing the identical view (Tech Spec
      §14.6, FR-5). Workspace clippy-clean under the §1 lint policy; offline suite
      green.
      _420 tests pass, clippy clean. All exit criteria met._

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1/v0.2/v0.3 phase docs' logs).

- **Cache version is independent of the transcript `SCHEMA_VERSION`** — the
  sidecar shape ≠ `TranscriptRecord`; an unknown cache version is treated as
  stale (→ replay), never a crash.
- **`adopt_session` turn-state gap (group 4)** — pre-existing: in-session resume
  does not rebuild `turn_map`/`compacted`. Phase 4 fixes it on both the cache and
  replay paths (the cache carries the state; replay rebuilds it). If the fix
  refines the Spec's resume description, record it as design feedback (G-24) for
  the next Spec bump — do not edit the Spec from this downstream doc.
- **Staleness guard = byte length (== built-through offset in an append-only
  log) + version/id.** Pure metadata check, no re-tokenization. A content hash is
  the fallback only if same-length divergence is ever observed (Tech Spec §16).
- **No config key** — always written best-effort, always guarded on read (Tech
  Spec §8, §3.2a).
- **Latch (`auto_compact_armed`) not persisted** — re-derived to a safe default on
  resume; it is transient FR-4 hysteresis state, not part of the durable view.
