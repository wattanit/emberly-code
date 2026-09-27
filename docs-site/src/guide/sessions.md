# Sessions & Long Conversations

Every session is written to `.agents/sessions/<id>.jsonl` as it happens
(durably, line by line). If Emberly crashes or is killed, the next launch offers
to resume. You can also:

- `emberly resume` — continue the most recent session
- `emberly resume <id>` — continue a specific one
- `emberly sessions` — list them, newest first, each with its resume command
- `/session` (in-app) — pick one from a menu and switch without leaving

For long sessions the context-economy layer works automatically; use `/compact`
to manually summarize the older part when the window fills. Recent messages are
kept verbatim and the summary is recorded in the transcript, so resuming works.

## Exporting a session

**`/export <path>`** (or `emberly export <path>` from
the shell, on any saved session) renders the full conversation — including
any subagent it spawned — to one self-contained HTML file: read-only, never
mutating the source transcript. It carries everything the transcript does,
so review before sharing it — export never redacts.

Each session also gets a disposable scratch directory
(`.agents/scratch/<id>/`) the agent can stash temporary files in — a script,
intermediate output, a working note — via the `scratch_write` tool. It's
harness-managed (the model supplies content, never a path) so it costs no
permission prompt, and it's gitignored so it never lands in your tracked
project. Nothing auto-deletes it; reclaim the disk space with `emberly clean`.
