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
| 0. Prerequisites & dependencies | [x] | reqwest C-free; **crypto deferred to g7** |
| 1. First-party SSE parser | [x] | 8 tests (split frames, multibyte, crlf) |
| 2. Anthropic Messages API client | [x] | 3 mock tests |
| 3. OpenAI-compatible client | [x] | 2 mock tests; covers Ollama/vLLM (P-2) |
| 4. Retry & failure policy | [x] | 3 retry unit + 3 engine tests |
| 5. Token & cost accounting | [ ] | wires real ContextUsage/CostEstimate |
| 6. Secrets & configuration | [~] | env config done; keys.toml/config.toml/redaction remain |
| 7. Binary wiring & provider selection | [x] | pure-Rust TLS; replaces placeholder |
| 8. Tests & live smoke (exit criterion) | [ ] | unit+mock done; live keyed remains |

**Overall Phase 3: groups 0–4 + 7 done, 6 partial (2026-07-06); 67 tests green.**
**TLS decision RESOLVED (owner): pure-Rust `rustls` + `rustls-rustcrypto`**
(alpha, tracked for v1). Verified the actual build graph is C-crypto-free (no
`ring`/`aws-lc`); a CI step guards HC-2. `emberly` now selects a live provider
from `EMBERLY_PROVIDER`/`EMBERLY_MODEL` + API key and talks real HTTPS.
**Remaining: group 5 (accounting), rest of 6 (keys.toml/config file/redaction),
group 8 (live keyed smoke — needs your API key).**

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
      `reqwest`'s `rustls-tls` pulls `ring`/`aws-lc-rs` (C/assembly), conflicting
      with HC-2. **Decision made: DEFERRED to group 7.** Provider clients take an
      **injected `reqwest::Client`**, so the TLS/crypto backend is chosen only
      at binary-wiring time — the client library and all tests run over plain
      HTTP and stay C-free. The pure-Rust-vs-`ring` (unaudited-vs-C) call is
      still an **owner decision** to make in group 7. **Real HTTPS use is
      blocked until then.**
- [x] Added `reqwest` (default features **off**; `json`, `stream`; no TLS
      feature). Tree confirmed C-free (`grep` for openssl/ring/aws-lc = none).
- [x] **SSE:** first-party parser (group 1), not `eventsource-stream`.
- [x] Jitter: `fastrand` (tiny, pure-Rust) — used by `RetryPolicy` (group 4).
- [x] `wiremock` dev-dependency for keyless client/retry HTTP tests.
- [x] Deps added to `emberly-providers`; C-dep ban stays green. `cargo
      vet`/`deny` run in CI.

## 1. First-party SSE parser  *(P-5; Tech Spec §4.3, §16)*

- [x] `sse.rs`: byte-buffered parser → `SseEvent { event, data }`; multi-line
      data joined, comments/`[DONE]` handled, `\n\n` and `\r\n\r\n` terminators.
- [x] Streaming-friendly: buffers raw bytes so multibyte UTF-8 split across
      chunks is never decoded mid-character; `finish()` flushes a trailing frame.
- [x] 8 unit tests (split frames, multibyte split, crlf, comments, `[DONE]`).

## 2. Anthropic Messages API client  *(P-1, P-4, P-5; Tech Spec §4.2)*

- [x] `anthropic.rs`: thin `reqwest` client, injected `Client`, no SDK (P-4).
- [x] Request mapping (`build_body`): system top-level, messages → content
      blocks; `Role::Tool` → user-role `tool_result` blocks; tools mapped.
- [x] SSE → `StreamEvent` (`AnthropicMapper`): text_delta → `TextDelta`;
      tool_use `content_block_*`/`input_json_delta` → `ToolCall{Start,Delta,End}`;
      `message_delta`/`message_stop` → `Usage` + `Done`; `error` → `Err`. (P-1)
- [x] Authoritative `usage` (input from `message_start`, output from
      `message_delta`) surfaced as `Usage`.
- [x] `x-api-key` + `anthropic-version`; status → `ProviderError` (`wire.rs`).
      → 3 mock tests (text+usage, tool call, auth error).

## 3. OpenAI-compatible client  *(P-1, P-2, P-4, P-5; Tech Spec §4.2)*

- [x] `openai.rs`: thin `reqwest` client; **configurable base URL** (covers
      Ollama/vLLM/OpenRouter/private — P-2).
- [x] Request mapping (`build_body`): `/chat/completions`, `messages` (system
      prepended, assistant `tool_calls`, `tool` role results), `tools` as
      functions, `stream:true`, `stream_options.include_usage`.
- [x] SSE → `StreamEvent` (`OpenAiMapper`): `delta.content` → `TextDelta`;
      index-keyed `tool_calls` fragments → `ToolCall{Start,Delta,End}`;
      `finish_reason` → `Done`; `usage` → `Usage`; `[DONE]` ignored.
- [x] Bearer auth (when key non-empty). → 2 mock tests (text+usage, split
      tool call across chunks).

## 4. Retry & failure policy  *(S-3; Tech Spec §4.3)*

- [x] `retry.rs`: `RetryPolicy` (max_attempts 3, base 500ms, cap 8s) with
      exponential backoff + **full jitter** (`fastrand`); `delay_for`/`may_retry`.
      3 pure unit tests (bounds, exponential, max-attempts).
- [x] **Retry loop lives in the engine** (it owns the event channel; policy is
      provider data) — `open_stream_with_retry` retries retryable pre-stream
      failures honoring `Retry-After`; every retry emits `UiEvent::Retrying`
      (never silent), rendered by the line frontend.
- [x] **Mid-stream drop** → engine retries the **whole turn** (`StreamEnd::
      Dropped`); partial text shown live but **not stitched** into the retry
      (message commit moved out of `consume_stream`); bounded, then HarnessError.
- [x] Post-retry failure → HarnessError; session stays live (S-3). → 3 engine
      tests (pre-stream retry, non-retryable not retried, whole-turn drop retry).

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

## 7. Binary wiring & provider selection  *(P-2, P-3)* — DONE

- [x] **Pure-Rust TLS (owner decision):** reqwest `rustls-tls-webpki-roots-
      no-provider` (rustls **without** ring) + `rustls-rustcrypto` provider
      installed at startup (`provider_setup::build_https_client`). Build graph
      verified C-crypto-free; CI HC-2 guard added. Alpha crate, tracked for v1.
- [x] `provider_setup::select_provider`: builds Anthropic / OpenAI-compat from
      `EMBERLY_PROVIDER` + `EMBERLY_MODEL` + key (+ `EMBERLY_BASE_URL`,
      `EMBERLY_CONTEXT_WINDOW`, `EMBERLY_MAX_OUTPUT`). Unset → offline
      placeholder. Verified end-to-end (real client builds, retries, fails
      gracefully against a bogus endpoint — no panic).
- [x] Clear behavior when unconfigured: offline placeholder with a hint line.

### Historic (superseded — TLS resolved here, not deferred)
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

### Groups 0–4 outcomes (2026-07-06)

- **TLS/crypto — DEFERRED to group 7, not decided.** Rather than resolve the
  pure-Rust-vs-`ring` question up front, provider clients take an **injected
  `reqwest::Client`**. reqwest is built with **no TLS feature** → the whole
  provider layer + tests are C-free and run over plain HTTP (`wiremock`). This
  cleanly unblocked groups 0–4 without silently pulling `ring`. **Owner
  decision still needed in group 7** (pure-Rust RustCrypto = memory-safe but
  unaudited, vs `ring` = vetted but C/asm). **Real HTTPS is blocked on it.**
- **Retry loop is in the engine, not the client** (refines §4.3's "client-side"
  wording): the engine owns the event channel and must surface each retry, and
  whole-turn drop-retry is inherently engine-level. `RetryPolicy` (data +
  jitter) stays in `emberly-providers`; clients are single-attempt.
- **Deterministic retry tests without a clock:** the engine test harness uses a
  `RetryPolicy` with 1–2ms delays, so retry paths run fast without injecting a
  clock (simpler than `tokio` time control here).
- **`consume_stream` refactor:** message-commit moved from `consume_stream` to
  `run_turn`, so a dropped turn's partial is shown live but not committed/
  stitched into the retry. Phase 1 engine tests unaffected.

### Pre-seeded plans (some superseded above)

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
