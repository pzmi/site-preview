---
name: static-html-preview
description: >
  Use when creating or editing a static HTML, CSS and JavaScript site and you need to see it
  render: start a local live-reload preview with site-preview, inspect it in a browser at
  mobile, tablet and desktop widths, confirm that edits reload the page, and always stop the
  server afterwards.
---

# Static HTML preview

Verify a static site by looking at it, not by reading its source. This skill drives
`site-preview` (a local live-reload server for static sites, one binary, no Node at runtime).
It never modifies the site: the reload script is added to served responses only.

Install this directory as a skill in your agent (for Claude Code, copy it to
`.claude/skills/static-html-preview/` in your own project or to your user skills directory).
It needs `site-preview` on `PATH` and a browser tool such as Playwright MCP.

## Steps

1. Find the root: the directory with `index.html`.
2. Start the server as a background process and keep its stdout:
   `site-preview serve --json --port 0 <root>`.
3. Read the first stdout line, `{"event":"ready","url":"http://127.0.0.1:PORT/","pid":N,"watching":true}`.
   Keep `url` and `pid`. If the process exits first, read stderr (warnings and errors are text
   there), fix the cause and start again.
4. Fetch `url`; it must succeed. If `watching` is `false` or stdout later shows
   `{"event":"watcher","state":"degraded",...}`, edits will not reload: reload by hand after
   each edit, or restart.
5. Open `url` in the browser. Look at screenshots at 390, 768 and 1440 px wide, a full-page
   screenshot, the accessibility snapshot, console errors and failed requests. Use the
   controls, links and responsive layout the artifact has.
6. Edit the source. Confirm the reload by reading `url` + `__site_preview/events` until a
   `reload` event arrives, or by polling the page until `data-revision` on
   `script[src*="__site_preview/client.js"]` grows. Look again.
7. Repeat until it looks and behaves right.
8. Stop the server with `kill -TERM <pid>` when finished or when you give up. Exit status 0
   means a clean stop.

## Rules

- Never bind to `0.0.0.0` or pass `--allow-remote` to fix reachability; there is no
  authentication. A browser in another container or VM cannot reach your loopback address:
  run the browser beside the server or forward the port with an access-controlled tunnel.
- One preview per site. `--port 0` avoids collisions with other previews.
- Do not rely on the text on stderr for control flow; stdout is the machine interface.
