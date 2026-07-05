# Phase 3 — Live Providers — TODO & Progress

**Milestone:** M3 (Tech Spec §15) — *Anthropic + OpenAI-compat live providers,
streaming, retries, token/cost accounting.*
**Goal:** Prove P-1..P-6. Two live provider backends behind the abstraction,
real SSE streaming, a retry/failure policy, and token/cost accounting — turning
the Phase 1 engine from a placeholder demo into something that actually codes.

**Depends on:** Phase 1 complete (the `Provider` trait, engine loop, and
`FakeProvider` all exist and are proven). **Reordered ahead of Phase 2** — see
`PHASE2_TODO.md` and the phase-order note.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 0. Prerequisites & dependencies | [ ] | **HC-2 crypto decision** lives here |
| 1. First-party SSE parser | [ ] | pure, unit-tested |
| 2. Anthropic Messages API client | [ ] | |
| 3. OpenAI-compatible client | [ ] | covers Ollama/vLLM/OpenRouter (P-2) |
| 4. Retry & failure policy | [ ] | + engine whole-turn retry on drop |
| 5. Token & cost accounting | [ ] | wires real ContextUsage/CostEstimate |
| 6. Secrets & configuration | [ ] | env-first, keys.toml 0600, redaction |
| 7. Binary wiring & provider selection | [ ] | replaces PlaceholderProvider |
| 8. Tests & live smoke (exit criterion) | [ ] | unit+mock in CI; live keyed |

**Overall Phase 3: not started.**

---

## Platform note (read first)

Phase 3 is **fully developable and testable on macOS** — it is network/HTTP
code, no OS sandbox involved. Everything except the *live* round trip is
unit- or mock-tested with no API key and runs in CI:

- **In CI (no keys):** request/response JSON mapping, the SSE parser,
  retry/backoff logic (injected clock), and full client paths against a **mock
  HTTP server**.
- **Manual / nightly (keyed, never in the merge path — Tech Spec §14.4):** one
  real tool-use round trip per backend. This is the exit criterion's live half.

---

## 0. Prerequisites & dependencies  *(§10 policy; HC-2; Tech Spec §12, §16)*

- [ ] **HC-2 crypto-provider decision (do first — it gates everything).**
      `reqwest`'s `rustls-tls` pulls a crypto backend that is **not pure Rust**
      (`ring`/`aws-lc-rs` have C/assembly), which conflicts with HC-2
      ("pure-Rust crypto") and the memory-safety priority. Plan: use `rustls`
      with a **pure-Rust provider** (RustCrypto via `rustls`'s provider API),
      built into a `ClientConfig` and handed to reqwest via
      `.use_preconfigured_tls(...)`, with reqwest's own TLS features **off**.
      Validate this compiles static-musl and connects, before building clients.
      Fallback to record if pure-Rust is unworkable: an explicit, owner-approved
      HC-2 exception for a vetted `ring`/`aws-lc-rs`.
- [ ] Add `reqwest` (default features **off**; `json`, `stream`; TLS via the
      preconfigured pure-Rust rustls above — **not** the `rustls-tls` feature
      if that forces `ring`). Confirm the tree stays C-free (`cargo deny`).
- [ ] **SSE decision (Tech Spec §16, bias first-party):** implement a
      first-party SSE frame parser (group 1) rather than `eventsource-stream`;
      record the call.
- [ ] Jitter source for backoff (group 4): prefer a tiny pure-Rust RNG
      (`fastrand`) or derive jitter from elapsed nanos — no C, no unsafe in
      first-party. Decide here.
- [ ] Add a **mock HTTP server** dev-dependency (e.g. `wiremock`) for
      client/retry integration tests without a key.
- [ ] `cargo vet`/`deny` acceptance for every addition; C-dep ban stays green.

## 1. First-party SSE parser  *(P-5; Tech Spec §4.3, §16)*

- [ ] Parse an SSE byte stream into events: accumulate `data:` lines until a
      blank line, handle multi-line data, comments (`:`), and `[DONE]`
      sentinels. ~100 lines, no dependency.
- [ ] Streaming-friendly: works over a `bytes` stream with partial frames
      across chunk boundaries.
- [ ] Pure + unit-tested with canned byte slices (including split frames).
      Feeds both provider clients.

## 2. Anthropic Messages API client  *(P-1, P-4, P-5; Tech Spec §4.2)*

- [ ] Thin first-party `reqwest` client (no vendor SDK — P-4).
- [ ] Request mapping: normalized `CompletionRequest` → Anthropic Messages
      body (system, messages, `content` blocks, `tools`, `max_tokens`).
      `ContentBlock::{Text,ToolUse,ToolResult}` ↔ Anthropic content blocks.
- [ ] SSE response → normalized `StreamEvent`: `content_block_delta` (text) →
      `TextDelta`; `content_block_start/delta/stop` for `tool_use` →
      `ToolCall{Start,Delta,End}`; `message_delta`/`message_stop` → `Usage` +
      `Done{stop_reason}`. No wire type crosses the boundary (P-1).
- [ ] Authoritative `usage` (input/output tokens) surfaced as `Usage`.
- [ ] Auth header (`x-api-key`, `anthropic-version`); errors → `ProviderError`.

## 3. OpenAI-compatible client  *(P-1, P-2, P-4, P-5; Tech Spec §4.2)*

- [ ] Thin `reqwest` client; **configurable base URL** → transitively covers
      Ollama, vLLM, OpenRouter, private deployments (P-2).
- [ ] Request mapping: `/v1/chat/completions`, `messages`, `tools`
      (function schema), `stream: true`. Normalized ↔ OpenAI `tool_calls`.
- [ ] SSE `delta` chunks → `TextDelta`; streamed `tool_calls` (index-keyed
      fragments) → `ToolCall{Start,Delta,End}`; `finish_reason` → `Done`;
      `usage` (when the endpoint sends it) → `Usage`.
- [ ] Bearer auth; base URL + model from config (group 6).

## 4. Retry & failure policy  *(S-3; Tech Spec §4.3)*

- [ ] Client-side retry for **pre-stream** failures: exponential backoff +
      jitter on connect errors / 429 / 5xx, honoring `Retry-After`; **max 3**
      attempts. Uses `ProviderError::is_retryable()`/`retry_after()` from
      Phase 1.
- [ ] Every retry surfaced as a dimmed harness-voice line (never silent) —
      an event the frontend renders.
- [ ] **Mid-stream drop** (Phase 1 `StreamEnd::Dropped`): keep the partial
      assistant text visible + marked interrupted, then **retry the whole
      turn** (partial turns are not stitched). This updates the engine's
      `run_turn` drop handling — bounded retry count, then a HarnessError.
- [ ] Post-retry failure → harness-world error; **session stays live and
      resumable** (S-3). Injectable clock/sleep so tests are deterministic.

## 5. Token & cost accounting  *(P-6; Tech Spec §4.4; Design §3.1)*

- [ ] Prefer authoritative `Usage` from responses; between responses estimate
      with `count_tokens` (chars/4). Accumulate per session.
- [ ] Per-model **pricing table** in config (`[pricing."model-id"] input=…,
      output=…` per MTok); `Pricing::estimate_usd` (added Phase 1) drives the
      estimate.
- [ ] Emit real `ContextUsage` (against the true model window) and
      `CostEstimate` events; the line frontend can show them (still quiet by
      default — the rich display is Phase 4). Cost always labeled **"est."**

## 6. Secrets & configuration  *(Tech Spec §8)*

- [ ] API keys: **env vars first** (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
      etc.), else `~/.config/emberly/keys.toml` with **`0600` enforced**
      (warn + refuse on group/world-readable).
- [ ] Provider/model/base-URL config: global `~/.config/emberly/config.toml`
      + project `.agents/config.toml` (project wins per key); `EMBERLY_*`
      env overrides. (Full config/provenance system is Phase 5; Phase 3 reads
      only what it needs.)
- [ ] Keys **never** in project config, never logged; requests logged with
      auth headers **redacted** (matters once transcripts land in Phase 5 —
      keep the redaction seam now).

## 7. Binary wiring & provider selection  *(P-2, P-3)*

- [ ] Replace `PlaceholderProvider` selection: build the real provider from
      config/flags — `--provider`, `--model`, base URL, key.
- [ ] Clear failure when no provider/key is configured (harness voice, next
      step) — not a panic; keep the placeholder available behind an explicit
      offline flag if useful for demos.
- [ ] `emberly` now runs a **real** coding session in line mode (rich TUI is
      Phase 4; kernel sandbox is Phase 2 — until then it runs tool-layer
      confined + degraded, honestly labeled).

## 8. Tests & live smoke (exit criterion)  *(P-3; Tech Spec §14.1, §14.4)*

- [ ] Unit: request serialization (both wire formats), SSE parsing (incl.
      split frames + tool-call fragments), retry/backoff schedule (injected
      clock), error classification.
- [ ] Mock-server integration: full client path incl. a streamed tool-use
      round trip and a 429→retry→success, with **no key**, in CI.
- [ ] `FakeProvider` engine tests from Phase 1 still green (abstraction
      unchanged).
- [ ] **Live smoke (manual/nightly, keyed, not in merge path):** one real
      tool-use round trip against **each** backend; streaming + a forced retry
      observable; context/cost driven by real usage.

---

## Phase 3 exit criterion (from IMPLEMENTATION_PLAN.md)

> Both live backends complete a one-tool-use round trip in the manual/nightly
> smoke suite, streaming and retries observable, context-usage and cost figures
> driven by real usage data.

- [ ] **Exit criterion met.** (P-3: the abstraction is proven by two live
      implementations before v1.)

---

## Notes / decisions log

*(Pre-seeded with decisions to confirm during Phase 3; add outcomes as work
proceeds.)*

- **HC-2 crypto provider (biggest decision):** rustls' default backends
  (`ring`, `aws-lc-rs`) are not pure Rust. To honor HC-2 + the memory-safety
  priority, target a **pure-Rust rustls provider (RustCrypto)** via a
  preconfigured `ClientConfig`, with reqwest's built-in TLS features off.
  Validate static-musl build + a real TLS handshake in group 0 before
  committing. If genuinely unworkable, surface a scoped HC-2 exception for an
  owner decision — do not silently pull `ring`.
- **First-party SSE** over `eventsource-stream` (Tech Spec §16 bias); ~100
  lines, fewer deps, exact control.
- **Retry split:** pre-stream failures retry inside the provider client;
  mid-stream drops retry the **whole turn** in the engine (updates Phase 1's
  `StreamEnd::Dropped` path). Bounded, surfaced, never silent (S-3).
- **Jitter:** tiny pure-Rust RNG (`fastrand`) or elapsed-nanos derivation — no
  C, no first-party unsafe.
- **Deterministic retry tests:** inject the clock/sleep so backoff is testable
  without real waits.
- **Config scope:** Phase 3 reads only the provider/model/keys it needs; the
  full two-tier config + `--show-config` provenance system is Phase 5.
- **Placeholder provider:** retire from the default path once real selection
  works; optionally keep behind an explicit offline/demo flag.
