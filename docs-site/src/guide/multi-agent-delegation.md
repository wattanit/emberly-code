# Multi-Agent Delegation

The agent can delegate a self-contained subtask to a subagent — a nested
Emberly session with its own conversation, context window, and completion
gate, running the exact same code as the primary agent rather than a second
implementation. Four tools drive it: `spawn_agents` (start one or more, in
parallel), `message_agent` (send a further prompt to one that's still alive),
`list_agents` (situational awareness when an id has been forgotten), and
`end_agent`. A subagent can never itself spawn, message, list, or end
subagents — that ceiling is structural (the tool is simply not in its own
registry), not a depth counter that could be gotten wrong.

Delegation never lowers the bar on anything: a subagent's tool calls are
decided by the *same* permission rules and sandbox as the primary agent's —
proxied to the one session-wide decision state, never an independent copy —
so a permission prompt raised on a subagent's behalf carries a dimmed
"on behalf of subagent `<name>`" line rather than appearing unattributed, and
its token usage/cost rolls into your session's own displayed total, never a
hidden side channel. Everything a subagent does shows up as quiet, ordinary
tool activity in the main conversation — never a second live-streamed voice —
and the sidebar's **Agents** section (present only while at least one is
alive) lets you open a read-only, live-updating inspector on any of them, past
or present, via **`/agents`**.
