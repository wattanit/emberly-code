# Web Search & Image/Document Input

- **Web search.** The `web_search` tool only exists for the model to call once
  you've pointed `[search]` at a real backend — with no `endpoint` configured
  (the default) it is never registered at all, so the agent will say it has no
  search access rather than the tool silently failing. Nothing reaches the
  internet, and nothing costs you anything, until you add a block like:

  ```toml
  [search]
  adapter     = "brave"       # or "tavily" / "searxng" / "json"
  endpoint    = "https://api.search.brave.com/res/v1/web/search"
  auth        = { scheme = "header", header = "X-Subscription-Token", key = "brave" }   # reads BRAVE_API_KEY
  max_results = 5              # results sent to the model per search; default 5
  ```

  Brave and Tavily both need a paid/free-tier API key from their own service;
  a self-hosted SearXNG instance is often keyless (`auth` can be omitted). See
  the commented `[search]` examples `emberly init` scaffolds for all three.
  Set `enabled = false` to make the absence explicit even with an endpoint
  configured.
- **Image input, model-initiated.** For vision-capable models
  (`vision = true`), the agent can read an image file inside your project
  into the conversation via the `read_image` tool — point it at a screenshot
  or diagram and ask about it.
- **Image input, you-initiated.** You can also hand the agent an image
  directly with **`/attach <path>`** before sending your message — the same
  vision path `read_image` uses, staged onto the next message you send. On a
  model with no vision support, the image is never silently dropped; a plain
  note takes its place saying so.
- **Document input.** For document-capable models (`documents = true`), the
  agent can read a PDF file inside your project into the conversation via the
  `read_document` tool — the same pattern as image input, applied to
  documents. The harness never parses the PDF; it just forwards the bytes, so
  there's no page count or extracted text, only size and format.
