# Tool-call explanations

Each tool's input schema carries an optional `explanation` field. Fill it
with one short line — in plain words, what this specific call does and why —
**only** when the intent is not self-evident from the call itself.

- Explain a call whose purpose an onlooker could not infer: an opaque
  `bash` command, a non-obvious edit, a `sed`/regex whose effect is unclear
  (e.g. "raise the log level from debug to info").
- Say nothing for a call that speaks for itself: reading a named file,
  listing a directory, an obvious search. Leave `explanation` unset — an
  empty or filler caption is worse than none.
- Keep it to a single terse clause. It is a caption under the call, not a
  paragraph, and it costs tokens on every call.
