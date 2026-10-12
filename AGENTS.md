# Verifying a static site with site-preview

This file is for AI coding agents that use `site-preview` on a static HTML site. It is not a
guide to developing site-preview itself (see `CLAUDE.md` and `CONTRIBUTING.md` for that).
Copy it into the repository of the site you are working on, or adapt it into a skill (see
`examples/claude-skill/static-html-preview/SKILL.md`).

`site-preview` serves a directory with live reload. It never modifies your source files: the
reload script is added to served HTML responses only.

## Workflow

When you create or edit a static HTML artifact:

1. Find the site root: the directory that holds `index.html`.
2. Start the server as a managed, long-running background process and keep its stdout:

   ```sh
   site-preview serve --json --port 0 <root>
   ```

   `--port 0` lets the system pick a free port, so parallel previews never collide. Never use
   `--host 0.0.0.0` or `--allow-remote` to make a container or VM reachable (see the caveat
   below).
3. Read the first stdout line. It is JSON and nothing else is printed before it:

   ```json
   {"event":"ready","url":"http://127.0.0.1:49152/","pid":12345,"watching":true}
   ```

   Keep `url` and `pid`. Do not parse the human text on stderr; warnings go there as
   `site-preview: warning[<code>]: <message>`. If the process exits before printing `ready`
   (exit code 1: a bad root or an unavailable port, 2: a usage error), read stderr, fix the
   cause and start again. Do not wait for a line that will never come.
4. Check that the page loads and that live reload is on:

   ```sh
   curl -fsS <url>
   ```

   The response must be a success and `watching` must be `true`. If `watching` is `false`, or
   a later stdout line is `{"event":"watcher","state":"degraded","reason":...}`, edits will
   not reload the page: reload by hand after each edit, or restart the server. A later
   `{"event":"watcher","state":"watching"}` means it recovered.
5. Open `url` in your browser tool (for example Playwright or Playwright MCP) and look at the
   result:
   - screenshots at viewport widths 390, 768 and 1440 px;
   - a full-page screenshot, the accessibility snapshot, console errors and failed requests;
   - the controls, links, responsive layout and local assets this artifact relies on.
6. Edit the source files. Then confirm that the page reloaded: either watch the event stream
   until a `reload` event arrives,

   ```sh
   curl -sN <url>__site_preview/events
   ```

   (events are `state` on connect and `reload` after each settled burst of edits, each with
   `data: {"instance_id":...,"revision":N}`), or poll the served page and wait until
   `data-revision` on `script[src*="__site_preview/client.js"]` grows. The same element
   carries `data-instance`, which changes when the server restarts. Edits are batched: one
   reload follows a burst once the files have been quiet for the debounce window (150 ms by
   default).
7. Repeat steps 5 and 6 until the visual and interaction checks pass.
8. Stop the server when the task is done, and also when a check fails or you give up:

   ```sh
   kill -TERM <pid>
   ```

   SIGTERM (or SIGINT) is a clean stop and exits with status 0. Never leave the process
   running, and never stop it with SIGKILL unless it ignores SIGTERM.

Do not infer visual correctness from the HTML source or a DOM snapshot alone; look at the
rendered page.

## Isolation caveat

`127.0.0.1` means "this machine" to every process, so a browser running in another container
or VM cannot reach the preview just because both sides use loopback. Run the browser where
the server runs, or forward the port with an access-controlled mechanism (an SSH tunnel, a
container port mapping bound to loopback). There is no authentication: do not expose the
server on a shared network.

## Reference

The flags, exit codes, JSON events and warnings are documented in `README.md` ("CLI
reference", "Machine output"); troubleshooting is in `doc/runbook.md`.
