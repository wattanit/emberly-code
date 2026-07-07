You are compacting a coding session. Your summary will REPLACE the
older portion of this conversation; the most recent messages are kept
verbatim by the harness. Whatever you do not capture here is lost to
the agent that continues the work — write for that agent, not for a
human reader.
 
Produce exactly these sections, in this order:
 
## Task
The user's original request, preserved faithfully (near-verbatim), plus
any later refinements, corrections, or scope changes the user made.
 
## Current state
What has been completed and verified (and how it was verified: which
build/test/command passed), what is in progress, what has not been
started.
 
## Files
Every file created, modified, or deleted: path, what changed, and why.
One line per file where possible.
 
## Key decisions
Choices made and their reasons, including approaches that were tried
and abandoned — and why — so they are not retried.
 
## User directives
Standing instructions, preferences, and constraints the user stated
during the session that remain binding (style choices, things to avoid,
approval boundaries). Preserve their wording where it matters.
 
## Active problems
Unresolved errors and blockers. Include exact error messages, exact
failing commands, and current hypotheses.
 
## Next steps
The remaining work, in order, specific enough to act on immediately.
 
Rules:
- Factual only. Include nothing you cannot ground in the conversation;
  never invent paths, names, or results.
- Preserve exact identifiers: file paths, function and symbol names,
  commands, flags, version numbers, error text. Approximations here
  cause the continuing agent to fail.
- Omit pleasantries, routine tool chatter, and exploration that led
  nowhere and taught nothing.
- If a section is empty, write "None."
- As short as full fidelity allows — compress prose, never facts.
- Output the sections only: no preamble, no closing remarks.