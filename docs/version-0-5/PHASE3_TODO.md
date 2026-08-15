# Phase 3 — TUI (`emberly-tui`) — TODO & Progress

**Milestone:** M12 phase 3 (Tech Spec §15) — the 0.5 feature set.
**Satisfies:** Design Guideline §4.13/§8.9/§5/§3.1 over FR-9. Pinned to
**Req v0.11 / Design v0.11 / Spec v0.13** (all `approved`). Depends on
Phase 2 (the events this phase renders).

## Group 1 — Sidebar Agents section (Design §3.1/§4.13)

- [x] New sidebar section, present only while at least one subagent is
      alive (the established no-empty-stub rule — Tasks/Memory/Skills/
      completion gate). `render_sidebar` in `render.rs`; hidden whenever
      `App.agents` is empty (tested:
      `agents_section_is_absent_while_no_subagent_is_alive`).
- [x] Renders from `SubagentSpawned`/`SubagentEnded`: one line per alive
      subagent (`name (id)`), sourced from `App.agents` filtered to
      `!ended` (`render_sidebar`, tested:
      `ended_agent_drops_from_the_sidebar_but_the_last_one_ending_hides_the_section`).
      Deviation: `SubagentStatus` (running/idle/timed-out) is **not**
      rendered on the line — the event carries no such field yet and none
      was added; the line is name+id only, same shape as Skills.
- [x] Selecting an entry sends `Command::InspectAgent` (click via
      `ClickTarget::AgentRow`/`OpenAgentsInspector`, key via Enter in the
      inspector; tested: `agents_enter_issues_inspect_for_the_selected_agent`).

## Group 2 — Per-agent inspector overlay (Design §4.13)

- [x] New overlay variant (`OverlayContent::AgentList`, `RowKind::Agent`)
      reusing the existing §4.2 overlay machinery (scrollable, Esc to
      dismiss; tested: `agents_inspector_esc_dismisses`).
- [x] Renders `UiEvent::AgentActivity` as a read-only text overlay on top
      of the list (`apply_agent_activity` → `open_text_overlay`, mirroring
      `SkillBody`; tested: `agent_activity_opens_a_read_only_overlay`).
- [ ] **Known deviation from Design §4.13:** the overlay is a **snapshot
      fetched once on Enter** (reads the subagent's transcript as of its
      last flush — Tech Spec §8.4's documented choice from Phase 2), not
      the "live-updating... as it happens" overlay the Design Guideline
      describes. True live streaming would need a second per-subagent
      event subscription into the open overlay; not built this phase.
      Flagged for the owner: accept as a v0.5 simplification (record as
      upstream feedback against Design §4.13) or schedule the live variant
      before release.
- [x] **Closed:** an ended subagent's inspector stays reachable for the
      rest of the session (Design §4.13). `AgentSummary` gained an `ended`
      flag; `SubagentEnded` now marks the matching entry instead of
      removing it (`app/events.rs`, `line.rs`'s run loop), so it survives
      in `App.agents` / `LineState.agents`. The **sidebar section** still
      filters to `!ended` only (§3.1's "currently alive" + no-empty-stub
      rule), but the `/agents` **inspector** list — both the rich overlay
      (`AgentList`) and plain-mode `/agents` — shows the full catalog, with
      an ended entry marked (`(id, ended)`) so it's never confused with a
      live one, and Enter/`{name}` still issues `Command::InspectAgent`
      for it exactly as for a live one. Tested:
      `agents_enter_on_an_ended_entry_still_inspects_it`,
      `subagent_spawned_and_ended_events_mark_ended_rather_than_remove`,
      `agent_list_marks_an_ended_entry`, `resolve_agent_still_resolves_an_ended_entry`,
      `agents_slash_still_lists_and_resolves_an_ended_agent`.
- [x] Confirmed the main pane never receives a subagent's raw streaming
      text — `AgentActivity` is the only subagent-activity event a
      frontend applies, and it only ever opens on an explicit `InspectAgent`
      reply, never unsolicited.

## Group 3 — Tool-activity lines

- [x] `spawn_agents`/`message_agent`/`end_agent`/`list_agents` render
      through the existing generic `ToolOutcome.summary` path — no new
      rendering code needed (confirmed by re-reading `render.rs`'s tool-call
      rendering, which is generic over any tool name/summary; this was
      already true after Phase 1's tool implementations set `summary`
      correctly).

## Group 4 — Permission-prompt provenance line (Design §5)

- [x] Prompt renderer reads `PermissionRendering.on_behalf_of` and adds one
      dimmed line naming the subagent when present; unchanged when `None`
      (`render.rs::render_permission`, tested:
      `permission_prompt_names_the_subagent_it_is_on_behalf_of`). Committed
      in `aed4ee1`.
- [x] Confirmed every other permission-prompt guarantee is untouched: Deny
      default, full content, no timeout-to-approve, no batching (existing
      tests `permission_prompt_shows_full_content_and_deny_default` etc.
      still pass unchanged).

## Group 5 — Degraded mode (Design §7)

- [x] Sidebar Agents section has no degraded-mode analog (line mode has no
      sidebar at all — same as Tasks/Memory/Skills), but its two reachable
      parts do: `/agents` lists the cached alive set inline
      (`render_agent_list`, tested: `agent_list_renders_name_id_and_empty`,
      `agents_slash_lists_then_resolves_a_name`), `/agents <name-or-id>`
      resolves and fetches activity (`resolve_agent`, tested:
      `resolve_agent_matches_by_name_then_id`), and `AgentActivity` prints
      inline read-only (tested: `agent_activity_renders_inline_read_only`).
- [x] The provenance line renders in degraded mode too (`line.rs`'s
      `render_permission`, tested:
      `permission_prompt_names_the_subagent_in_degraded_mode`). Committed
      in `aed4ee1`.
- [x] No reliance on color, motion, or the mouse for anything above — all
      new degraded output is plain ASCII, checked by the existing
      `degraded_output_has_no_ansi_escapes` sweep test.

**Status:** functionally complete and tested except one remaining flagged
item in Group 2 (the live-updating overlay), a real gap against Design
§4.13 as written, not an oversight — surfaced there for an owner decision
rather than silently built around. The ended-agent-reachability gap is
now closed.

`cargo build --workspace`, `cargo test --workspace`, `cargo clippy
--workspace --all-targets` (`-D warnings`), and `cargo fmt -p emberly-tui
--check` all pass. Not yet done: a live manual verification pass in a real
terminal session (spawn a batch of subagents, watch the sidebar/inspector,
converse via `message_agent`, trigger a permission prompt from a subagent's
own tool call).
