# Persistent Memory

Emberly keeps a small, progressive-disclosure memory store the agent can
write to and recall across sessions — durable facts about you and the
project. The **`/memory`** overlay lets you inspect, edit, and delete entries
grouped by scope. Nothing is hidden: what the agent remembers is always yours
to review.

Config: `[memory] enabled` (default `true`), `[memory] max_index_entries`
(default `50`, a soft warn threshold, not a hard limit) — see the
[Config Keys Reference](../configuration/reference.md).

See also: [Skills](./skills.md), the other progressive-disclosure system —
standing capabilities the model invokes on demand, rather than durable facts
it recalls.
