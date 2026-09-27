# Reasoning Effort & the Thinking Trail

For models that expose a reasoning control, you own the latency/cost/quality
trade per task. Enable it per model with an `effort` default (see
[Providers, Models & Keys](../configuration/providers.md)).
**`/effort`** sets the level for your next message; the engine maps it to the
provider's native knob (Anthropic's thinking budget, OpenAI's
`reasoning_effort`). When a provider streams its **reasoning** distinctly, it
renders as a quiet, collapsed `▸ reasoning (N lines)` trail; **Ctrl-R** toggles
it open. The `reasoning` config key sets the default view (`hidden` still
records the trace to the transcript).
