# Phase 1 — Tool-result salient reduction (FR-2) — TODO & Progress

**Milestone:** M7 group 1 (Tech Spec §15) — the 0.3 context-economy set, **layer
2** of Requirements §8.4 (salient reduction, sitting on top of the §8.1 size
truncation that already ships).
**Satisfies:** FR-2; Tech Spec §5.3, §3.2 (`full_output_ref`), §8
(`truncate.reduce`); Design §4.3, §8.6; HC-7 (transcript untouched). Pinned to
Req v0.6 / Design v0.6 / Spec v0.7 (all `approved`).
**Goal:** Reduce a tool result to its salient content *by meaning* before it
enters the model context — deterministically, with **no model call** — keeping
the full output in the sidecar and one `/view` away. `truncate.reduce = false`
turns the layer off.

**Depends on:** the shipped product (M1–M6). The pieces this phase builds *on*
already exist: `truncate_output`/`TruncateConfig`
(`crates/emberly-tools/src/truncate.rs`, `.../ctx.rs`), the ingestion point
`Engine::ingest_tool_result` (`crates/emberly-core/src/engine.rs:1556`), and the
sidecar + `full_output_ref` machinery (`FileTranscript::sidecar`,
`crates/emberly-core/src/transcript.rs:326`, writing under
`.agents/sessions/<id>-outputs/`). Phase 1 adds a reduction pass *before* the
existing size backstop and widens the sidecar trigger to cover reduction.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Reducer registry & signature (`emberly-tools`) | [x] | `reduce.rs` — `Reduction` type, `reduce_output` dispatch by tool name, passthrough, `reduction_marker` helper |
| 2. Per-tool reducers: `bash`, `grep`, `glob` (`read_file` none) | [ ] | |
| 3. Wire reduction into ingestion, before the size backstop | [ ] | |
| 4. Sidecar on reduce-*or*-truncate; transcript honesty (HC-7) | [ ] | |
| 5. Config: `truncate.reduce` + wire the `[truncate]` TOML section | [ ] | |
| 6. Tests (offline, deterministic — §14.6) + exit criterion | [ ] | |

**Overall Phase 1: IN PROGRESS (Group 1 done).**

---

## 1. Reducer registry & signature  *(FR-2; Tech Spec §5.3)*

A reduction layer that mirrors how `truncate_output` is structured — a pure,
deterministic function in `emberly-tools`, no I/O, no model call — so the engine
calls it at ingestion exactly as it already calls `truncate_output`.

- [x] New module `crates/emberly-tools/src/reduce.rs`: a `Reduction` result type
      (`content: String`, `reduced: bool`, and enough to build the marker — e.g.
      what/how-much was withheld) and a `reduce_output(tool_name: &str, raw:
      &str) -> Reduction` entry point. Pure function, `#[must_use]`, no
      `unwrap`/`expect` (HC-3). Re-export from `lib.rs` beside `truncate_output`.
- [x] Registry keyed by **tool name** (the engine has the invoked tool's name at
      the `ingest_tool_result` call site via `call`/`PendingToolCall`, so keying
      by name avoids threading `ToolSpec` through the engine — §5.3's
      `(&ToolSpec, &raw)` shape realized as name-dispatch). A tool with no
      registered reducer **falls straight through** (returns `reduced: false`,
      content unchanged) to the size backstop — decision to record in the notes
      log if the signature diverges from the Spec's wording.
- [x] Reducers are **pure and deterministic** (Requirements §8.5: no model
      round-trip, no per-call latency). Collapsing "runs of near-identical
      lines" is a deterministic string transform, not a heuristic call.

## 2. Per-tool reducers  *(FR-2; Tech Spec §5.3)*

The initial reducer set (initial; tune with use — Requirements §13, Tech Spec
§16). Salient content is defined per tool by the Spec:

- [ ] **`bash`** — keep the exit status, **all** of stderr, and head+tail of
      stdout; deterministically collapse runs of near-identical
      progress/percentage lines. *Example (non-normative, §5.3):* a 500-line
      `cargo build` collapses to its warnings/errors + final summary. Reducer
      lives in / near `crates/emberly-tools/src/builtin/bash.rs` output shape.
- [ ] **`grep`** — keep every match line and its `file:line` header; drop
      nothing that is a hit (hits are the point, §5.3). Note the tool already
      caps at `MAX_MATCHES` (`builtin/grep.rs`) — the reducer must not double-cap
      or hide hits.
- [ ] **`glob`** — keep the path list; if it exceeds the count backstop, keep
      head+tail with the elision marker (§5.3).
- [ ] **`read_file`** — **no semantic reducer** (§5.3): already bounded by the
      optional `start_line`/`end_line` and the size backstop; reducing by meaning
      would risk hiding code the model asked for. Verify it falls through the
      registry untouched.
- [ ] Each reducer that withholds anything produces a marker **naming what was
      withheld** and offering `/view` (Design §4.3/§8.6) — e.g. progress lines
      collapsed — distinct in wording from the size-truncation marker
      (`truncate.rs:109`) so the model can tell *reduced-by-meaning* from
      *trimmed-by-size*.

## 3. Wire reduction into ingestion (reduction first, then size backstop)  *(FR-2; Tech Spec §5.3; Requirements §8.4 layering)*

`Engine::ingest_tool_result` (`crates/emberly-core/src/engine.rs:1556`) currently
does `truncate_output(&outcome.content, &self.truncate)`. Insert the reduction
pass ahead of it so the order is **salient reduction → size backstop** (§5.3).

- [ ] Run `reduce_output(tool_name, &outcome.content)` first (gated on the
      `truncate.reduce` flag, group 5), then `truncate_output(reduced.content,
      …)`. The model-visible content is the output of both passes, in that
      order.
- [ ] The reduction adds no model call and no await that blocks the loop — it is
      a synchronous pure transform on the already-collected `outcome.content`
      (Requirements §8.5). Keep the S-5 loop-signature accumulation
      (`engine.rs:1564`) on the **full** `outcome.content` (unchanged), so
      reduction never weakens no-progress detection.
- [ ] Leave `result_preview` (`engine.rs:1909`) on the full content unless the
      preview visibly regresses — the UI caption is not the context payload.

## 4. Sidecar on reduce-*or*-truncate; transcript honesty  *(FR-2; HC-7; Tech Spec §3.2)*

Today the sidecar/`full_output_ref` is written **only when size-truncated**
(`engine.rs:1570–1574`). Reduction also withholds content, so the full output
must be preserved whenever *either* pass fired.

- [ ] Write the sidecar (`self.transcript.sidecar(&call.id, &outcome.content)` —
      always the **full** pre-reduction, pre-truncation content) whenever
      `reduced || truncated`, and set `full_output_ref` accordingly. The
      complete output is the durable record the marker points at; FR-2's
      "recoverable" is guaranteed by construction (the sidecar holds everything).
- [ ] Decide and record: the transcript `ToolResult.truncated` flag
      (`transcript.rs:114`) now means "the recorded `output` is not the full
      output; `full_output_ref` has the whole thing" and is set on
      reduce-or-truncate — **or** add an additive `reduced: bool` field
      (serde-default, older readers warn-skip, no `SCHEMA_VERSION` bump, §3.2) if
      we want to distinguish the two in the audit trail. Pick one in the notes
      log; default to reusing `truncated` unless the distinction earns its keep.
- [ ] Confirm resume (`resume.rs:95` `rebuild_conversation`) still rebuilds from
      `ToolResult.output` (the model-visible content) and is unaffected — the
      view it restores is the reduced/truncated one, matching the live session
      (HC-7: the log is untouched; the sidecar holds the full output).

## 5. Config: `truncate.reduce` + wire the `[truncate]` section  *(FR-2; Tech Spec §8)*

`truncate.reduce` does not exist and `[truncate]` is not wired to TOML at all
(production hardcodes `TruncateConfig::default()` at `crates/emberly/src/config.rs`
/ `main.rs:509`). Adding the toggle means giving `[truncate]` a config home.

- [ ] Add `reduce: bool` (**default `true`**, Tech Spec §8) to `TruncateConfig`
      (`ctx.rs:14`) so the flag rides with the truncation config already carried
      into the engine (`EngineConfig.truncate`) and `ToolCtx`. `#[derive(Copy)]`
      stays valid.
- [ ] Wire a `[truncate]` section into `ConfigFile`
      (`crates/emberly/src/config.rs`) → build the engine's `TruncateConfig` from
      it instead of the hardcoded default (`main.rs:509`). Expose at least
      `reduce`; exposing `max_lines`/`max_bytes`/`head_lines`/`tail_lines` here
      too is natural and retires part of the Tech Spec §16 "`truncate.*`
      defaults" open item — keep them optional with the current defaults.
- [ ] `truncate.reduce = false` skips group 3's reduction pass entirely (raw
      results, then only the size backstop) — for users who want unreduced
      output. Provenance/`config show` (C-3) reports the tier as for any key.

## 6. Tests + exit criterion  *(Tech Spec §14.6 offline, deterministic)*

- [ ] **Per-tool reducer unit tests** (`reduce.rs`): a noisy `bash` output
      collapses its progress runs to warnings/errors + summary; `grep` keeps all
      hits + headers; `glob` head/tails an over-count list; `read_file` passes
      through untouched. Deterministic, no provider.
- [ ] **Full output recoverable:** an engine ingestion test asserts that oversized
      / noisy output is reduced in the context message **and** that the sidecar
      holds the complete output with a `full_output_ref` set (closes the
      end-to-end gap: no current test proves ingestion-time truncation writes a
      sidecar). Assert the model-visible marker offers `/view`.
- [ ] **Toggle:** `truncate.reduce = false` passes output through the reduction
      layer unchanged (only the size backstop may act).
- [ ] **HC-7:** the `tool_result` transcript record + sidecar together preserve
      the full result; nothing rewrites a prior transcript line.
- [ ] **Exit criterion (Phase 1 done when):** per-tool reducers reduce
      `bash`/`grep`/`glob` to salient content with the full output recoverable
      from the sidecar; `truncate.reduce = false` passes through; the transcript
      records the full result (via sidecar) untouched (Tech Spec §14.6, FR-2,
      HC-7). Workspace clippy-clean under the §1 lint policy; offline suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1/v0.2 phase docs' logs).

- Reducer dispatch is keyed by **tool name**, not by threading `&ToolSpec`
  through the engine — the Spec's `(&ToolSpec, &raw)` shape (§5.3) realized as
  name-dispatch at the `ingest_tool_result` call site. _Confirm at
  implementation; record here if it diverges from the Spec wording (feedback per
  G-24 if the Spec should absorb it)._
- `truncated` vs. a new `reduced` transcript field (group 4) — _decide at
  implementation._ Default: reuse `truncated` to mean "recorded output ≠ full
  output; see `full_output_ref`"; add `reduced` only if the audit trail needs to
  tell size-trim from meaning-reduce apart.
- Reduction runs **before** size truncation (§5.3, §8.4 layering); the size
  backstop always applies after, reducer or not.
