# Phase 5 — Web search (T-14) — TODO & Progress

**Milestone:** M8 phase 5 (Tech Spec §15) — the 0.4 capability-parity set. Fifth phase;
independent of Phases 1–4. It gives the agent **live web reach** through a
**harness-owned, config-driven, provider-agnostic** search backend — permission-gated,
its results treated as untrusted content.
**Satisfies:** T-14; Tech Spec §5.5, §6.1, §12, §8; Design §5.2, §4.10; Requirements §1
(provider-agnostic), §6; HC-2. Pinned to **Req v0.7 / Design v0.7 / Spec v0.8** (all
`approved`).
**Goal:** A `web_search` tool that talks to a **configured** search service through a
thin first-party `reqwest` client, exactly mirroring the provider-profile pattern
(§4.5, P-8) so a new search service is config, not code. It is **permission-gated**
(default `web_search → ask`, allowlistable like a bash command), its network egress is
the **harness process** (not a sandboxed child, so governed by the permission layer, not
sandbox confinement), and its `{title, url, snippet}` results are rendered as **untrusted
web content** with visible source URLs — never harness or assistant voice.

**Independent of Phases 1–4.** Per the plan's dependency summary, Phases 1/2/5 are
mutually independent; Phase 5 builds only on the shipped product (M1–M7 + the 0.4 phases
merged so far) and the provider/permission machinery. It can be implemented in any order
relative to Phase 6. Overlap with other phases is confined to the additive sites
(`ConfigFile`, the registry build, `builtin_defaults`, the TUI tool-render branch).

**Depends on:** the shipped product. The seams it mirrors:
- **Provider reqwest client (the HTTP precedent)** — injected `reqwest::Client` on the
  provider struct (`crates/emberly-providers/src/anthropic.rs:29`, `new` :40), POST +
  auth apply + send (:80), `Auth::apply` (`crates/emberly-providers/src/auth.rs:24`),
  non-2xx mapping `check_response` (`crates/emberly-providers/src/wire.rs:19`). The
  binary builds the HTTPS client once (`crates/emberly/src/provider_setup.rs:249`,
  `build_https_client`) and selects the adapter purely from config (:108).
- **Provider profile + auth + key resolution (the `[search]` precedent, P-8)** —
  `ProfileFile` (`crates/emberly/src/config.rs:154`), `AuthFile` (:171), field-merge
  (:201), adapter chosen at wiring (`provider_setup.rs:85`/:110), `resolve_auth`
  (`provider_setup.rs:216`), `api_key` env→keys.toml (`config.rs:1049`), keys.toml 0600
  enforcement (:1142).
- **Permission-gating (the `bash` precedent)** — `BashTool` `spec`/`execute`/authorize
  with `outside_root: false` (`crates/emberly-tools/src/builtin/bash.rs:150`, :182),
  `ctx.authorize` (`crates/emberly-tools/src/ctx.rs:167`), engine gate
  (`crates/emberly-core/src/gate.rs:34`), rule engine `Decision`
  (`crates/emberly-sandbox/src/rules.rs:26`), **built-in defaults**
  (`builtin_defaults` :338), fall-through `Ask` (:257), generic per-tool grant
  `tool_session_grant` (:434), permissions.toml load (`config.rs:840`), session/project
  grant (`engine.rs:1975`/:1981), engine consults rules `on_permission_ask` (:2006).
- **PermissionRequest / the reserved outside-root band to AVOID** — `PermissionRequest`
  (`crates/emberly-tools/src/permission.rs:17`, `outside_root` :28), `PermissionRendering`
  (`crates/emberly-core/src/types.rs:22`), the loud outside-root TUI styling to stay
  clear of (`crates/emberly-tui/src/render.rs:906`/:929, `line.rs:200`, banner
  `strings.rs:22`).
- **Tool result → size backstop (no tool change needed)** — reduce→truncate at
  ingestion (`crates/emberly-core/src/engine.rs:2067`), `reduce_output` dispatch
  (`crates/emberly-tools/src/reduce.rs:64`, web_search falls through to passthrough),
  `truncate_output` (`crates/emberly-tools/src/truncate.rs:33`).
- **TUI tool render (untrusted styling is greenfield)** — `ConvItem::Tool` render
  (`crates/emberly-tui/src/render.rs:415`), plain `ToolFinished` (`line.rs:84`),
  `result_preview` (`engine.rs:2803`), `ToolFinished` UiEvent (`event.rs:54`).
- **Registry + config-driven wiring** — `default_registry()` (unconditional, 11 tools,
  `crates/emberly-tools/src/builtin/mod.rs:34`), called without config
  (`crates/emberly/src/main.rs:516`), `registry.specs()` advertises only registered tools
  (`crates/emberly-tools/src/registry.rs:38`), `Resolved` (`config.rs:386`), `load`/merge
  (`config.rs:461`).
- **Deps** — workspace `reqwest` (`Cargo.toml:41`, no default features; `rustls-tls`,
  `json`, `stream`), `emberly-tools/Cargo.toml` has **no reqwest yet** (:10), wiremock
  dev-dep precedent (`emberly-providers/Cargo.toml:23`).
- **Plain-HTTP tests** — wiremock over plain HTTP with an injected client
  (`crates/emberly-providers/tests/live_clients.rs:1`, `http_client` :17, mock :96),
  config-only adapter unit test (`provider_setup.rs:274`); tool gate test doubles
  `AllowGate`/`DenyGate` (`crates/emberly-tools/tests/builtin_tools.rs:61`); engine
  round-trip + `PermissionRequest` assertion (`engine_loop.rs:3528`).

**Key structural facts (from the codebase):**
1. **web_search is the first *config-conditionally-registered* tool.** Memory and skills
   always register and disable their *backend* (`build_memory_store`/`build_skill_catalog`
   return `None`); but Tech Spec §5.5 requires that when `search.enabled = false` the tool
   is **not registered at all** — the model must not even see it in `registry.specs()`.
   `default_registry()` is unconditional today (`builtin/mod.rs:34`), so this needs a new
   config-driven registration step in the binary (do **not** push config into
   `default_registry()` — keep it pure for tests). New pattern; resolve in group 4.
2. **Network egress is the harness process, not `ctx.sandbox()`.** Unlike `bash` (which
   spawns a confined child via `ctx.sandbox().bash_invocation`), `web_search` makes a
   first-party `reqwest` call from the engine process, exactly like a provider call (Tech
   Spec §5.5, §2). §6.2/§6.3 child confinement does **not** apply; the permission layer is
   what governs it. The tool never touches the sandbox.
3. **The reserved outside-root safety band must NOT be used.** `web_search` sets
   `PermissionRequest.outside_root = false` (like `bash`, `bash.rs:187`). The loud band
   (`render.rs:929`, `OUTSIDE_ROOT_BANNER`) is reserved for filesystem escape; a web
   search is an ordinary, lower-stakes gated action (Design §5.2). Getting this wrong
   dilutes the one signal that must never be ignored — assert `outside_root == false` in a
   test.
4. **The tool holds its own client + resolved config (mirror providers), not a
   `ToolCtx` gate.** Providers keep `{client, auth, base_url}` on the struct; likewise
   `WebSearchTool { client, adapter, endpoint, auth, max_results }` is built by the binary
   composition root and registered. Endpoints/secrets live on the tool, not threaded
   through `ToolCtx`. There is **no engine round-trip gate** (unlike memory/skills) — only
   the ordinary `ctx.authorize` permission call.
5. **Reuse the P-8 profile/auth/key machinery; the `query` auth scheme is new.** `[search]`
   mirrors `ProfileFile`/`AuthFile`, and key resolution reuses `config::api_key`
   (env→keys.toml, never inline). But the provider `Auth` enum
   (`emberly-providers/src/auth.rs`) has only `bearer`/`x-api-key`/`header`/`none` — the
   spec's `query` scheme (append `?key=…`) is **new** and, since `emberly-tools` does not
   depend on `emberly-providers`, lives search-side in `emberly-tools`. Key *resolution*
   stays in the binary; only the resolved secret is handed to the tool.
6. **`web_search`/`SearchConfig`/search adapters are greenfield** — none exist in `.rs`
   (confirmed); only the docs describe them.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Dep (`reqwest` into tools) + search types + adapters/parsers + thin client + auth apply (`emberly-tools`) | [x] | `SearchResult{title,url,snippet}`; brave/tavily/searxng/json parsers; `query` auth scheme new |
| 2. `web_search` tool: authorize (outside_root=false) → HTTP → parse → cap → ToolOutcome (`emberly-tools`) | [x] | first-party HTTP, never `ctx.sandbox`; results tagged untrusted; cap at `max_results` |
| 3. Permission rule `web_search → ask` + ordinary treatment + allowlist (`emberly-sandbox`) | [x] | add to `builtin_defaults`; generic `tool_session_grant`/permissions.toml already works |
| 4. Config `[search]` profile + key resolution + conditional registration (`emberly`) | [x] | mirror `ProfileFile`/`resolve_auth`; register only when `enabled` + endpoint set |
| 5. Untrusted-content TUI render (§4.10) + permission prompt copy (§5.2) + degraded (`emberly-tui`) | [x] | title/URL/snippet as quoted web text; visible source URLs; ASCII in degraded mode |
| 6. Tests (offline, wiremock plain-HTTP — §14.7) + exit criterion | [x] | parser tests; gate fires + allowlist suppresses; disabled→unregistered; cap; untrusted render |

**Overall Phase 5: COMPLETE.**

---

## 1. Dependency + search types, adapters, thin client, auth  *(T-14; Tech Spec §5.5, §12, HC-2)*

New `crates/emberly-tools/src/search.rs` — the client, the normalized types, and the
per-adapter response parsers. Pure over an injected `reqwest::Client` so tests run over
plain HTTP (mirror the provider clients).

- [ ] **Dependency (Tech Spec §12; `cargo vet` before merge — Requirements §10).** Add
      `reqwest.workspace = true` to `crates/emberly-tools/Cargo.toml` (`:10`) — **not a new
      external crate** (Providers already locks it), a new crate→crate edge; same feature
      set (no default features; `rustls-tls`, `json`, `stream`). Add `wiremock` +
      `rustls-rustcrypto` dev-deps mirroring `emberly-providers/Cargo.toml:23` for the
      plain-HTTP tests. Confirm the edge in `cargo vet`/`cargo deny` (no duplicate-version
      split).
- [ ] Types: `struct SearchResult { title: String, url: String, snippet: String }`
      (`Clone, Debug, Serialize, Deserialize, PartialEq`) and a `SearchError` (map connect
      / non-2xx / parse). Re-export from `emberly-tools/src/lib.rs`.
- [ ] **Search auth (search-side, `query` scheme is new).** `enum SearchAuth { None,
      Bearer(String), Header{name,value}, Query{param,value} }` with an `apply(builder,
      url) -> RequestBuilder` mirroring `Auth::apply` (`auth.rs:24`) — `Query` appends
      `?param=<key>` to the URL (the new scheme, Tech Spec §5.5). The secret is already
      resolved (binary, group 4); this only *applies* it. **Record the `query`-scheme
      addition in the notes log** (G-24 candidate if the Spec §4.5 `Auth` wording should
      absorb `query` provider-side too).
- [ ] **Response-shape parsers (the search analogue of §4.2 wire parsers).**
      `fn parse(adapter: &str, body: &Value, max: usize) -> Result<Vec<SearchResult>,
      SearchError>` dispatching to `brave`/`tavily`/`searxng`/`json`. Each normalizes a
      service's JSON into `Vec<SearchResult>`; `json` is a generic JSON-path-configurable
      parser for uncovered services (Tech Spec §5.5). Cap to `max` here. A genuinely new
      shape is a new parser; a new service is just a profile (P-8, the property to
      preserve). Unit-test each parser against a captured sample body (like the
      `anthropic.rs` mapper tests).
- [ ] Thin client: `struct SearchClient { client: reqwest::Client, endpoint: String,
      adapter: String, auth: SearchAuth, max_results: usize }` with `async fn search(&self,
      query: &str, count: Option<usize>) -> Result<Vec<SearchResult>, SearchError>` — build
      the request (GET/POST per adapter — confirm per service, note in log), apply auth,
      send, `check_response`-style non-2xx mapping, parse. Injected client (no TLS choice
      here). Only the configured `endpoint` is ever reached (Tech Spec §5.5).

## 2. The `web_search` tool  *(T-14; Tech Spec §5.2, §6.1)*

New `crates/emberly-tools/src/builtin/web_search.rs`, modeled on `builtin/bash.rs` (the
permission-gated precedent) — but HTTP from the harness, never a sandbox child.

- [ ] `struct WebSearchTool { client: SearchClient }` (holds the resolved client/config,
      like a provider struct — structural fact 4). `WebSearchTool::new(client)` built by
      the binary (group 4). `spec()`: name `"web_search"`, description (C-4, overridable
      C-1), schema `{query: string (required), count: integer (optional, ≤ max_results)}`.
      `describe()` → `format!("search: {query}")`.
- [ ] `execute()`: **authorize first** —
      `ctx.authorize(PermissionRequest { tool: "web_search", summary: format!("search: {query}"),
      detail: <query + backend endpoint name (not the key) + "reaches the internet; results
      are untrusted">, affected_paths: vec![], outside_root: false }).await` — return
      `ToolOutcome::denied("search the web")` if not allowed. **`outside_root` MUST be
      `false`** (structural fact 3). Never call `ctx.sandbox()`.
- [ ] On allow: call `self.client.search(query, count)`; on `Err`, return a clean HC-6
      `ToolOutcome::failure` (connect/non-2xx/parse — the model can react), never a panic.
      On success, format results as the model-facing content (per-result `title`, `url`,
      `snippet`) capped at `max_results`, and set a summary `searched: "<query>" (N
      results)`. **Tag the outcome as untrusted web content** for the TUI (group 5) — decide
      the tag mechanism in the notes log (a `web_search` tool-name key vs. an explicit
      marker on the outcome/event; leaning to an explicit marker so §4.10 styling is
      unmistakable and reusable).
- [ ] Long snippets/large result sets ride the existing §5.3 size backstop at ingestion
      (`engine.rs:2067`) — **no tool-side truncation beyond `max_results`**; confirm the
      backstop covers it (a passthrough reducer is fine, `reduce.rs:64`).

## 3. Permission rule + ordinary treatment + allowlist  *(T-14; Tech Spec §6.1, Design §5.2)*

All in `crates/emberly-sandbox/src/rules.rs` (+ confirm the generic grant path).

- [ ] Add a built-in default rule `web_search → ask` to `builtin_defaults()` (`rules.rs:338`,
      beside the read/bash defaults): `Rule { tool: ToolSelector::Named("web_search"),
      matcher: Matcher::Any, action: Decision::Ask, source: RuleSource::Builtin }`. (Even
      without it the fall-through is `Ask` at `:257`, but an explicit rule is the documented
      §6.1 behavior and makes `config show` honest.)
- [ ] Confirm allowlisting works with **no new code**: the generic `tool_session_grant`
      (`rules.rs:434`) + `grant_rule` non-bash branch (`engine.rs:2905`) already produce a
      per-tool `web_search → allow` for "allow this session" and
      `.agents/permissions.toml` for "always allow in this project" (Requirements §6.6).
      Add a `web_search` allow example to the rules tests.
- [ ] Verify the ordinary-vs-outside-root split: the request from group 2 has
      `outside_root: false`, so `on_permission_ask` (`engine.rs:2006`) renders the plain
      prompt, never the reserved band (Design §5.2). This is a *refinement* of T-14
      recorded in Design §5.2 — no requirement change, so nothing flows upstream.

## 4. Config `[search]` + key resolution + conditional registration  *(T-14; Tech Spec §8, §5.5)*

- [ ] `crates/emberly/src/config.rs`: add `pub search: SearchConfigFile` to `ConfigFile`
      (:19 region, beside `memory`/`skills`) — `struct SearchConfigFile { enabled:
      Option<bool>, adapter: Option<String>, endpoint: Option<String>, auth:
      Option<AuthFile>, max_results: Option<usize> }` (reuse `AuthFile` :171). Field-merge
      project-over-global in `ConfigFile::merge` (mirror `ProfileFile::merge` :201 for the
      nested `auth`).
- [ ] Resolve into `emberly_core`-free runtime search config used by the binary: defaults
      `enabled = true`, `max_results = 5` (Tech Spec §5.5). Add a `search` field to
      `Resolved` (`config.rs:386`). Resolve the `auth` → a `SearchAuth` reusing
      `config::api_key` for the key reference (mirror `resolve_auth` `provider_setup.rs:216`;
      extend it or add a `resolve_search_auth` that also handles the new `query` scheme). A
      configured `enabled=true` with **no endpoint** is a clear startup error (like a
      profile with no adapter, `provider_setup.rs:85`).
- [ ] **Conditional registration (new pattern, structural fact 1).** In the binary, after
      `default_registry()` and after building the HTTPS client (`build_https_client`,
      reuse or a `search_setup.rs` mirroring `provider_setup.rs`): when
      `resolved.search.enabled` and an endpoint is set, construct the `SearchClient` +
      `WebSearchTool` and `registry.register(Arc::new(web_search_tool))`; otherwise leave
      it unregistered so `registry.specs()` never advertises it. Keep `default_registry()`
      config-free (tests rely on it). **Decide the exact shape in the notes log** (a
      `build_registry(resolved, client)` helper in the binary vs. mutate-after-default).
- [ ] `emberly config show` (C-3): add a `[search]` block printing adapter, endpoint, and
      key **status** (`set`/`unset` via `key_status` :1041) — **never the key** (mirror the
      provider profile block :982). Record provenance for non-default `search.*` fields
      (`record` :901).

## 5. Untrusted-content TUI render + prompt copy + degraded  *(T-14; Design §4.10, §5.2, §7)*

- [ ] **Untrusted results render (Design §4.10, greenfield).** Render a `web_search`
      result as fetched web content — per hit: title, **visible source URL**, and the
      snippet styled as *quoted web text*, never harness or assistant voice, so a hostile
      "ignore your instructions" snippet reads visibly as web data. Extend the
      `ConvItem::Tool` render (`render.rs:415`) keyed on the group-2 untrusted tag (add a
      `ConvItem` variant or a marker field on the tool item — mirror how the tool result
      already carries `summary`/`preview`). Result count is already capped (`max_results`);
      long snippets already pass the size backstop.
- [ ] Plain/degraded frontend (`line.rs:84`): the same result list ASCII-only — title,
      `url`, snippet as indented quoted lines, no ANSI, source URLs visible (Design §7 —
      the "untrusted web content" meaning must survive without color). Keep
      `degraded_output_has_no_ansi_escapes` green.
- [ ] **Permission-prompt copy (Design §5.2).** Confirm the group-2 `detail` renders the
      three facts — the query, the backend endpoint name (dimmed, not the key), and the one
      plain line "reaches the internet; results are untrusted" — through the ordinary
      permission prompt (`render.rs` non-outside-root branch :906, `line.rs`), with Deny the
      default keypress and the standard allow-once / session / project choices. **No
      capitalized outside-root banner.**

## 6. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Parser unit tests:** each adapter (`brave`/`tavily`/`searxng`/`json`) normalizes a
      captured sample JSON body into `Vec<SearchResult>` and caps at `max_results`; a
      malformed body is a clean `SearchError`, not a panic.
- [ ] **Client over plain HTTP (wiremock, mirror `live_clients.rs`):** a `SearchClient`
      pointed at a `MockServer` returns parsed results; auth (`bearer`/`header`/`query`) is
      applied to the outgoing request (assert the header/query param); a 401/500 maps to a
      clean error.
- [ ] **Config-only / provider-agnostic (mirror `provider_setup.rs:274`):** switching
      `adapter`/`endpoint` in config selects the parser and endpoint with no code change —
      proves search is config, not vendor code (P-8, Requirements §1).
- [ ] **Permission gate fires + allowlist suppresses (engine round-trip):** a scripted
      `web_search` call raises a `PermissionRequest` with `outside_root == false`; a
      session `allow` grant suppresses the second call's prompt (mirror the bash gate
      tests, `engine_loop.rs` `PermissionRequest` assertion :3528).
- [ ] **`search.enabled = false` ⇒ unregistered:** the built registry's `specs()` does
      **not** include `web_search` (the model never sees it), and no `[search]` requirement
      of an endpoint is enforced when disabled.
- [ ] **Untrusted render:** the result block renders with visible source URLs in
      untrusted-content styling (rich) and ASCII-only with URLs (degraded); it is never in
      harness/assistant voice (Design §4.10/§7).
- [ ] **Cap honored:** more results than `max_results` are truncated to `max_results`
      before the model sees them; a very long snippet passes the §5.3 size backstop.
- [ ] **Exit criterion (Phase 5 done when):** a profile pointed at a fake search endpoint
      proves search is config-only and provider-agnostic; the `web_search → ask` gate fires
      and an allowlist grant suppresses it; results are tagged untrusted and capped at
      `max_results`; and results render in the untrusted-content style with source URLs
      (Tech Spec §14.7, T-14). Workspace clippy-clean under the §1 lint policy; `reqwest`
      edge `cargo vet`/`cargo deny`-clean (HC-2); offline suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.3, Phase 1–4 logs).

- **Tool holds the client, no engine gate.** Unlike memory/skills (engine-owned state
  reached via a gate), `web_search` is a self-contained tool holding its resolved
  `SearchClient` (mirror the provider structs), built by the binary composition root. Its
  only engine interaction is the ordinary `ctx.authorize` permission call. _If the Spec
  should absorb the tool-owns-client shape, it flows back per G-24, not an edit here._
- **Network egress is harness-process, permission-gated, not sandboxed.** First-party HTTP
  like a provider call (Tech Spec §5.5, §2); the tool never calls `ctx.sandbox()`. The
  permission layer is the right control for a harness-initiated egress with cost + an
  injection surface. `outside_root: false` — ordinary prompt, never the reserved band
  (Design §5.2).
- **Conditional registration is a new pattern.** Memory/skills always register and null
  their backend; §5.5 requires `web_search` absent from `registry.specs()` when disabled.
  Chosen: register in the binary (composition root) when `enabled` + endpoint set; keep
  `default_registry()` config-free for tests. _Decide `build_registry(resolved, client)`
  helper vs. mutate-after-default in group 4._
- **`query` auth scheme is new and lives search-side.** Provider `Auth`
  (`emberly-providers`) lacks `query`; `emberly-tools` can't depend on providers, so
  `SearchAuth` (with `Query{param,value}`) lives in `emberly-tools`. Key *resolution*
  stays in the binary (`config::api_key`, env→keys.toml, 0600); only the resolved secret
  reaches the tool — never printed by `config show`. _G-24 candidate if the Spec §4.5 auth
  set should gain `query` provider-side too._
- **Untrusted tagging mechanism.** Leaning to an explicit untrusted marker on the tool
  outcome/`ToolFinished` event (not just keying on the `"web_search"` name) so Design
  §4.10 styling is unmistakable and reusable by a future web-fetch source. _Decide in
  groups 2/5; if it needs a `ToolFinished` field, that is additive (`#[non_exhaustive]`)._
- **GET vs POST + request shape per adapter.** brave/tavily/searxng differ (query param
  vs. JSON body). _Confirm each against the real service in this phase and record; the
  `json` generic parser + a documented request shape is the escape hatch._
- **Size backstop, not a bespoke reducer.** Result flooding is bounded by `max_results` +
  the existing §5.3 backstop; no `web_search` entry in `reduce_output` initially
  (passthrough). _Add a salient reducer only if real result bodies prove noisy (Tech Spec
  §16, Requirements §13)._
- **Open items carried in (Tech Spec §16 v0.8, this phase's resolver):** the
  `brave`/`tavily`/`searxng`/`json` parsers and `bearer`/`header`/`query` auth are the
  initial set — **validate against the real services in this phase**; add a parser only for
  a genuinely new response shape. Web-search result density (hit count / snippet length
  that reads well — Design §10) tunes with use.
