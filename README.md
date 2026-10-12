# site-preview

Zero-configuration local static-site preview server with live reload, as a single Rust binary.

> **Status: pre-alpha.** The server, live reload and `--json` interface work from a source build; there is no release yet, so no install packages (an install section arrives with the first release).

## What it is

A globally installable development server that makes editing a static HTML site feel like editing Markdown. Point it at a directory containing `index.html` and it serves the site, watches for changes, and reloads the browser. No Node.js, npm, per-site configuration, build step, or changes to your source files are needed. Live-reload code is injected only into served HTML responses.

## What it is not

- Not a static site generator, bundler, or publishing service.
- Not a Node.js tool; the installed binary has no Node, npm, or Python runtime requirement.
- Not a browser security sandbox: it is a developer preview.

## Quick start

```text
$ site-preview serve .
Site Preview
Serving:  /home/me/site
Local:    http://127.0.0.1:49152/
Watching: 3 eligible directories
Ready.
```

Open the `Local:` URL, edit a file, save, and the page updates. Press Ctrl-C to stop. The port
is chosen by the system unless you pass `--port`. Two or more servers can run side by side;
they share no lock or runtime file. A directory without an `index.html` is served too (its
`/` is a 404, other files still load); there is never a directory listing.

`Watching:` counts watched directories, not files. It reads `N eligible directories` when
live reload is fully on, adds `(degraded: ...)` when some directories could not be watched,
and says `unavailable (...); live reload disabled` when none could (see
[doc/runbook.md](doc/runbook.md)).

## CLI reference

```text
site-preview serve [OPTIONS] [DIRECTORY]
site-preview --version
site-preview --help
```

Options may come before or after `DIRECTORY`.

| Argument | Default | Meaning |
|:--|:--|:--|
| `DIRECTORY` | `.` | The site root: an existing, readable directory. Resolved once at start-up. If the root itself is a symlink it is followed once; links below it are never followed. |
| `--host <IP>` | `127.0.0.1` | Address to listen on, a numeric IPv4 or IPv6 address (names such as `localhost` are refused). Outside loopback it needs `--allow-remote`. |
| `--port <N>` | `0` | Exact port; `0` lets the system choose a free one. An unavailable port is an error, never a fallback. |
| `--allow-remote` | off | Permit a non-loopback `--host` (including `0.0.0.0` and `::`). There is no authentication. Prints the `remote-binding` warning. |
| `--debounce <DUR>` | `150ms` | How long the site must stay quiet before the browser reloads, one window for the whole site. A whole number with an `ms` or `s` unit, from `10ms` to `10s`. |
| `--json` | off | Print start-up and watcher events as JSON lines on stdout (see "Machine output"). |
| `--open` | off | Open the `Local:` URL with `xdg-open` (Linux) or `open` (macOS) once ready. The launcher is run without a shell. A failure is only an `open-failed` warning on stderr, after the `Local:` line; the server keeps running and the exit code is unaffected. |
| `-h`, `--help` | | Print help. |
| `-V`, `--version` | | Print the version (top level only). |

When the server listens on a wildcard address (`0.0.0.0`, `::`), the printed URL uses the
loopback address of that family, because a wildcard is never a valid destination.

### Exit codes

| Code | Meaning |
|:--|:--|
| `0` | Clean stop after SIGINT or SIGTERM, or `--help` / `--version`. |
| `1` | The server could not start: the root is missing, not a directory or unreadable, the root holds the reserved name `__site_preview`, the port is in use or the address cannot be bound, or another fatal start-up error. The message on stderr says what to do. |
| `2` | Usage error (unknown flag, invalid `--debounce`, non-numeric `--host`, no subcommand), or a non-loopback `--host` without `--allow-remote`. |
| `130` / `143` | A second SIGINT / SIGTERM cut the shutdown short (128 plus the signal number). |

### Stopping

SIGINT (Ctrl-C) and SIGTERM stop the server: the listener closes first, open event streams
end, the watcher stops, and remaining responses get at most 3 seconds to finish. If that
takes longer than half a second, a notice on stderr says it is waiting and that pressing
Ctrl-C again stops at once; an ordinary stop is silent, so do not rely on the notice. In
`--json` mode the notice is text on stderr, never on stdout.

## Machine output (`--json`)

With `--json`, stdout carries JSON lines and nothing else; warnings and the shutdown notice
stay text on stderr. The first line is the `ready` event, written once and flushed as soon as
the server accepts connections:

```json
{"event":"ready","url":"http://127.0.0.1:49152/","pid":12345,"watching":true}
```

`watching` is false when no file watcher could start (pages are still served). Later lines
are `watcher` events, written when live reload degrades or recovers, never repeated for the
same state:

```json
{"event":"watcher","state":"degraded","reason":"runtime-failure"}
{"event":"watcher","state":"watching"}
```

`reason` is one of `registration-failed`, `runtime-failure` or `revision-exhausted`. A server
whose watcher could not start follows `ready` with a `degraded` event naming
`registration-failed`. New fields may be added to either event; do not rely on their order.

AI coding agents: [AGENTS.md](AGENTS.md) is a ready-made workflow built on this output (start,
read `ready`, inspect in a browser, edit, observe the reload, stop), and
[examples/claude-skill/static-html-preview/](examples/claude-skill/static-html-preview/SKILL.md)
is the same as an example skill.

## Warnings

Warnings go to stderr, one per line, as `site-preview: warning[<code>]: <message>`. The
server keeps serving after each one. Page warnings are printed once per request path (`/` and `/index.html` count separately).

| Code | Meaning |
|:--|:--|
| `remote-binding` | Listening outside loopback with no authentication (`--allow-remote`). |
| `open-failed` | `--open` could not start the platform launcher (`xdg-open`, `open`); the URL is in the message and the server keeps serving. |
| `html-too-large` | An HTML page over 8 MiB is served unchanged, without live reload. |
| `html-not-utf8` | An HTML page that is not UTF-8, or whose `<meta>` charset names another encoding, is served unchanged without live reload. |
| `html-rewrite-failed` | The reload script could not be inserted safely; the page is served unchanged. |
| `csp-blocks-live-reload` | The page's `<meta http-equiv="Content-Security-Policy">` would block the reload script or the reload connection: the governing `script-src` or `connect-src` (or their `default-src` fallback) allows none of `'self'`, `*`, `http:` or a host-source (an `https:`-only list warns, since this server speaks plain `http`; `'strict-dynamic'` makes a script list block). The page is still served with the script and its policy untouched, but it will not live reload; edit the policy for development. Only a meta tag is checked; other violations show only in the browser console. |
| `watch-registration-failed` | A directory could not be watched (named relative to the root, with the reason), at start-up or when it appeared later; changes in it do not live reload. It is retried after later changes. At most ten are named at a time, then the rest are counted. |
| `watch-unavailable` | Nothing can be watched (the native watcher could not start, or no directory could be registered): live reload is disabled and `Watching:` says so; pages are still served. |
| `watch-failed` | The native watcher failed while running: live reload may miss changes until site-preview is restarted; pages are still served. Reported once until the watcher recovers. |
| `reload-exhausted` | The revision counter ran out (after 2^64 - 1 reloads): live reload stops; pages are still served. |

## HTTP contract

Every response carries `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`. No CORS
header is ever sent.

| Request | Response |
|:--|:--|
| `GET /`, `GET /dir/` | `index.html` of that directory, with the reload script injected. |
| `GET /dir` for a directory | `307` to `/dir/` (query kept), so relative URLs resolve. |
| `GET /file.ext` | The file's bytes with its MIME type, streamed. `.gpx`, `.geojson`, `.kml` and `.wasm` have explicit types; HTML is `text/html; charset=utf-8`. |
| `HEAD` | The status and headers of the `GET`, no body. For HTML the `Content-Length` includes the injected script. |
| `Range` header | Ignored: the full resource is returned with `200`. |
| Denied names | `403`: any component starting with `.`, `node_modules`, `vendor`, `.venv`, `__pycache__`, editor swap/backup/temp names (`.*.swp`, `#*#`, `*.tmp`, `*.bak`, `*.orig`, `*.rej`, `4913`, names ending in `~`), any symlink, and any spelling of `__site_preview` outside the internal routes. Matching is per path component, never a substring, and case-insensitive. |
| Missing, a directory without `index.html`, malformed or traversing paths | `404`. There is never a directory listing. |
| Any method other than GET and HEAD | `405` with `Allow: GET, HEAD`. |
| A `Host` that is not this server | `421` (see Security posture). |
| `GET /__site_preview/client.js` | The embedded live-reload client. |
| `GET /__site_preview/events` | The reload event stream (below). `403` for a foreign `Origin` or `Sec-Fetch-Site: cross-site`; `503` with `Retry-After: 2` when 64 streams are already open. |
| Any other `/__site_preview/...` | `404`. |

Error bodies are fixed text (`404 Not Found`, `Host not allowed`, ...); nothing from the request
or the disk is echoed. The server keeps at most 128 connections open, drops a connection that
sends no complete request head within 10 seconds, and refuses to start if the site root already
contains an entry named `__site_preview` in any letter case.

## Live reload

### Injection

For a successful `GET` of an HTML document the server inserts one tag before the real
`</body>` (or at the end of the document):

```html
<script defer src="/__site_preview/client.js" data-instance="..." data-revision="42"></script>
```

`data-instance` identifies this server run and `data-revision` is the site revision when the
page was read, so a save that lands between loading the page and opening the stream is still
noticed. Neither is a visitor identifier; the client keeps state in memory only (no cookies,
no web storage) and the files on disk are never modified. The tag is added only for
document and iframe navigations (`Sec-Fetch-Dest`), not for HTML fetched by script, and not
twice into a page that already contains it (its attributes are refreshed instead). A `HEAD`
reports the length of the rewritten body but sends none. A page that cannot be rewritten
safely (over 8 MiB, not UTF-8, a parse failure, a blocking CSP) is served unchanged and gets a
warning, see the table above.

### Event stream

`GET /__site_preview/events` is a server-sent-events stream (`text/event-stream`,
`X-Accel-Buffering: no`). A `:` keepalive comment is sent after 15 seconds of silence.
`Last-Event-ID` is ignored: every connection starts with the current state.

```text
event: state
id: 0
data: {"instance_id":"4911-1791813351209113305","revision":0}

event: reload
id: 1
data: {"instance_id":"4911-1791813351209113305","revision":1,"changes":[{"path":"/css/site.css","kind":"css"}],"full":false}

```

- `state` is sent first on every connection and is only a baseline.
- `reload` is sent once per settled burst of changes. Slow readers coalesce: they may see only
  the newest revision.
- `changes[].path` is the root-relative URL path, percent-encoded per component. `kind` is
  `css`, `js`, `html`, `asset` (images, icons, fonts, audio and video), or `other`, taken from
  the extension (`.mjs` is `js`).
- `full` is `true` when the page cannot update in place: an `html` or `other` file changed, a
  directory appeared or vanished, more than 64 paths changed, or the connection skipped a
  revision (then `changes` is empty). Fields may be added later; ignore unknown ones.
- Changes in hidden, excluded or editor-temporary paths never advance the revision.

The client reloads when a `state` or `reload` shows a different instance (a server restart)
or revision than its baseline, so a reconnect costs at most one reload and never a loop. If
the stream is refused (for example `503`), it retries with a backoff from 1 s up to 30 s.

### What swaps and what reloads

When a `reload` is not `full`, comes from the same instance and is exactly the next revision,
the client tries to apply it in place:

| Changed | Result |
|:--|:--|
| Stylesheets (`css`) | The `<link rel="stylesheet">` is replaced by a cache-busted copy; the old one is removed only after the new one loads, so the page is never unstyled. A stylesheet that is not linked from the page (or only reachable through something the client cannot read) causes a reload. |
| Images, icons, fonts, media (`asset`) | Cache-busted in place in `<img src>`, `<img srcset>` / `<source srcset>`, `<link rel~=icon>`, and stylesheets whose rules reference the file in `url(...)` (including through `@import`). An asset used in any other way (fetched by script, inline style) causes a reload. |
| Scripts (`js`) | The page reloads and the scroll position is restored. It is kept in `history.state.__sitePreview` for that one reload (never in the URL, storage or cookies) and removed afterwards. Unless the page accepted the change, below. |
| `html`, `other`, anything unclear, a load failure or a 5 s load timeout | Full reload. |

The query parameter `__sp=<revision>` is what busts the cache; other query parameters and the
fragment are kept as they were.

### Accepting a script change without a reload

A page can opt in per script with `window.__sitePreview`:

```js
window.__sitePreview.version;                       // 1
window.__sitePreview.accept('/js/chart.js', async ({ path, url, revision }) => {
  const mod = await import(url);                    // url is cache-busted
  mod.render(document.querySelector('#chart'));
});
```

- `path` is root-relative (`'/js/chart.js'`) or any same-origin URL (`import.meta.url` works).
  A later call for the same path replaces its handler.
- When a change set holds only scripts and every changed script has a handler, no reload
  happens; each handler gets `{ path, url, revision }` and may return a promise.
- If any changed script has no handler, a handler throws, its promise rejects, or it does not
  settle within 5 seconds, the page reloads with scroll restore instead. The client moves to the
  new revision only after every handler succeeded.
- What re-running a module means (re-render, remove old listeners) is up to the handler; state
  of the old module copy is not carried over.
- The client is a deferred script at the end of the body, so register from a
  `DOMContentLoaded` listener when your own scripts run earlier. Each same-origin iframe
  document has its own client and its own API.

## Security posture

- Binds to loopback (`127.0.0.1`) by default; a non-loopback bind requires `--allow-remote` and prints the `remote-binding` warning.
- Every request passes a `Host` gate first (421 for a name that is not this server), which guards against DNS rebinding.
- The reload stream (`/__site_preview/events`) refuses a foreign `Origin` or a cross-site `Sec-Fetch-Site` with 403; no CORS headers are sent and no route changes state.
- Dotfiles and hidden directories are denied.
- Symlinks beneath the site root are rejected.
- Source files are only read; the reload script is added to served responses, never to files on disk.
- `--allow-remote` has no authentication; do not use it on untrusted networks.

See [SECURITY.md](SECURITY.md) for scope and reporting, and [doc/security.md](doc/security.md) for the threat model and enforced rules.

## Limitations

- A restrictive Content Security Policy can keep a page from live reloading; the policy is
  never changed. Only a `<meta>` policy is checked, so a policy sent as an HTTP header by
  another proxy shows up only in the browser console.
- Some pages are served intact but do not live reload, without a warning: a `</body>` inside
  SVG `<style>` text or an unclosed `<template>`, a frameset document, and a page loaded as a
  `<frame>`.
- The script is injected on document navigation, not into HTML fetched by script. Browsers
  without Fetch Metadata headers get a best-effort classification. Browsers send those headers
  only to potentially trustworthy origins, so a page opened at a LAN address under
  `--allow-remote` has none.
- Hidden files and directories are never served, including `.well-known`. Symlinks beneath the
  root are rejected even when they point inside it.
- There is no polling fallback. Network mounts and some container file shares emit no native
  change events, and OS event queues can overflow; see [doc/runbook.md](doc/runbook.md).
- Range requests get a full `200` response.
- Source files are never modified, but HTML responses are rewritten for preview.
- Your site's own scripts and assets may make external requests; site-preview itself makes
  none and collects nothing.
- Windows is not supported.

## Supported platforms

Linux and macOS (ARM64 and x86_64). Windows is not supported.

## Development

The toolchain is pinned in `rust-toolchain.toml`. From a checkout:

```text
cargo build -p preview-cli                # target/debug/site-preview
cargo xtask doctor                        # report missing dev tools
cargo xtask install-tools                 # install the pinned dev tools
cargo xtask quick                         # formatter, Clippy, fast tests (while working)
cargo xtask check                         # the full gate, run before every commit
```

`quick` is not `check`. `check` also runs the browser stage (`cargo xtask test --only browser`),
which needs Node 22, npm and the Chromium build 1194 that the pinned Playwright launches;
`install-tools` does not install it, so run `npx --prefix tests/e2e/browser playwright install chromium`
once. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Documentation

- [doc/runbook.md](doc/runbook.md) - troubleshooting
- [doc/plan.md](doc/plan.md) - implementation plan
- [doc/foundation.md](doc/foundation.md) - architecture, quality and delivery rules
- [doc/tasks.md](doc/tasks.md) - task breakdown
- [CONTRIBUTING.md](CONTRIBUTING.md) - how to contribute

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
