# Phase 3 — TUI (`emberly-tui`) — TODO & Progress

**Milestone:** M12 phase 3 (Tech Spec §15) — the 0.5 feature set.
**Satisfies:** Design Guideline §4.13/§8.9/§5/§3.1 over FR-9. Pinned to
**Req v0.11 / Design v0.11 / Spec v0.13** (all `approved`). Depends on
Phase 2 (the events this phase renders).

## Group 1 — Sidebar Agents section (Design §3.1/§4.13)

- [ ] New sidebar section, present only while at least one subagent is
      alive (the established no-empty-stub rule — Tasks/Memory/Skills/
      completion gate).
- [ ] Renders from `SubagentSpawned`/`SubagentStatus`/`SubagentEnded`: one
      line per alive subagent (name + status).
- [ ] Selecting an entry sends `Command::InspectAgent`.

## Group 2 — Per-agent inspector overlay (Design §4.13)

- [ ] New overlay variant reusing the existing §4.2 overlay machinery
      (scrollable, Esc to dismiss).
- [ ] Renders `UiEvent::AgentActivity` — the subagent's own assistant text
      and tool-activity lines, live-updating while it runs.
- [ ] A subagent that has ended keeps its inspector reachable for the rest
      of the session (its activity is reviewable after the fact).
- [ ] Confirm the main pane never receives a subagent's raw streaming text
      (Design §4.13's "no raw concurrent streaming" — this is a rendering
      *omission* to verify, not a feature to build).

## Group 3 — Tool-activity lines

- [ ] `spawn_agents`/`message_agent`/`end_agent`/`list_agents` render
      through the existing generic `ToolOutcome.summary` path — confirm no
      new rendering code is actually needed (the 0.4.3 scratch-write
      precedent), and that the subagent name(s) appear in the line.

## Group 4 — Permission-prompt provenance line (Design §5)

- [ ] Prompt renderer reads `PermissionRendering.on_behalf_of` and adds one
      dimmed line naming the subagent when present; unchanged when `None`.
- [ ] Confirm every other permission-prompt guarantee is untouched: Deny
      default, full content, no timeout-to-approve, no batching.

## Group 5 — Degraded mode (Design §7)

- [ ] Sidebar Agents section, inspector fallback, tool-activity lines, and
      the provenance line all render in plain ASCII under `--plain`/
      `NO_COLOR`/`TERM=dumb` — no reliance on color, motion, or the mouse.

**Done when:** live-verified in a real terminal session — spawn a batch of
subagents, watch the sidebar and inspector update, converse with one via
`message_agent`, trigger a permission prompt from a subagent's own tool call
and confirm the provenance line appears, then verify degraded mode keeps
full parity — plus `cargo build/test/clippy(-D warnings)/fmt -p emberly-tui`
clean.
