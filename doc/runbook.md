# Runbook: troubleshooting site-preview

Each entry is a symptom, its cause and the fix. Flags and warning codes are defined in the
[README](../README.md); this page says what to do when something goes wrong. Warnings are lines
on stderr shaped `site-preview: warning[<code>]: <message>`; the server keeps serving after
each one. Start-up failures are lines shaped `site-preview: error: <message>` and the process
exits with a nonzero code (1 for a refused start, 2 for a usage error).

## First checks

1. Read the `Watching:` line (or the `watching` field of the `ready` event with `--json`).
   `N eligible directories` means live reload is on; anything else is a degraded watcher, see
   [The watcher is degraded or unavailable](#the-watcher-is-degraded-or-unavailable).
2. Read stderr for `warning[...]` lines. Page warnings are printed once per request path.
3. Open the browser developer console: CSP and mixed-content problems appear only there.
4. `curl -i <url>__site_preview/client.js` should answer `200` with JavaScript, and
   `curl -N <url>__site_preview/events` should print an `event: state` block. If those work,
   the server side is fine and the problem is in the page or the watcher.

## Start-up refusals

### `cannot listen on ...: Address already in use` (exit 1)

Another process holds the port you passed with `--port`. Pick another port, or omit `--port`
so the system chooses a free one (the chosen URL is on the `Local:` line and in the `ready`
event). `--port` is exact: site-preview never falls back to another port. Several
site-preview processes can run at once as long as their ports differ.

### `refusing to bind <address> without --allow-remote` (exit 2)

A `--host` outside loopback (including `0.0.0.0` and `::`) needs `--allow-remote`. There is
no authentication: anyone who can reach the address can read the site. Prefer the default
loopback address, and use `--allow-remote` only on a network you trust. A `remote-binding`
warning is printed once the server is listening.

### `--host` rejected as invalid (exit 2)

`--host` takes a numeric IPv4 or IPv6 address. `localhost`, `127.1` and `[::1]` (brackets) are
refused; write `127.0.0.1` or `::1`.

### `invalid value ... for '--debounce'` (exit 2)

Write a whole number with an `ms` or `s` unit between `10ms` and `10s`: `150ms`, `1s`.
`0`, `1.5s`, `2m` and a bare `150` are refused.

### Site root errors (exit 1)

- `the site root does not exist` or `is not a directory`: pass the directory that holds
  `index.html`. With no argument the current directory is served.
- The root cannot be read: check its permissions.
- `the site root contains __site_preview ... a name reserved`: the site has a file or
  directory named `__site_preview` (any letter case). That name belongs to the server's
  internal endpoints and is never served from disk. Rename or remove it.

### `--open` did nothing (`warning[open-failed]`)

The launcher (`xdg-open` on Linux, `open` on macOS) could not start, typically because it is
not installed or there is no desktop session, as on a server or in a container. The server is
unaffected and the exit code stays 0; open the URL from the message yourself.

## Requests refused

### 421 `Host not allowed`

Every request is checked first against the server's own address, as a defense against DNS
rebinding. Accepted `Host` values:

- on a loopback listener (the default): the bound address and `localhost`, with the bound port;
- with `--allow-remote` on a wildcard address (`0.0.0.0`, `::`): any numeric IP address with the
  bound port, but never a DNS name and never `localhost`;
- with `--allow-remote` on a specific address: that address only.

Common causes and fixes:

- Browsing to a name such as `mysite.test` or a tunnel hostname: use the printed URL.
- A port forward or container publish that changes the port (`-p 8080:4173`): the port in the
  `Host` header must equal the port the server bound. Publish the same port number and use
  the numeric address.
- A reverse proxy that rewrites `Host`: not supported; point the browser at the server directly.
- Another computer: start with `--host <its address>` or `0.0.0.0` plus `--allow-remote`, and
  browse to the numeric address.

### 403 on `/__site_preview/events`

The event stream refuses a request whose `Origin` is not exactly this server's origin
(`http://` plus the same authority and port that passed the Host check) or that carries
`Sec-Fetch-Site: cross-site`. `localhost` and `127.0.0.1` are different origins: a page
loaded from one must stream from the same one, which is automatic for the injected client.
A 403 usually means a page from another site or local port is trying to read the stream, or a
proxy is adding an `Origin`. Plain `curl` sends neither header and is accepted.

### 503 on `/__site_preview/events` (too many streams)

At most 64 event streams are open at a time (one per open page or frame); the 65th gets `503`
with `Retry-After: 2`. Close stale tabs, or note that a tab that vanished without closing its
connection holds its slot until the connection ends. The injected client retries with a
growing delay (1 s up to 30 s), so a freed slot is picked up without reloading.

A cross-site page can also hold slots by opening streams without credentials when the page
you browse has no Fetch Metadata (a LAN address under `--allow-remote`); stop sharing the
address with untrusted pages.

### 403 on a file, or a file that "exists" but is not served

Denied by design, with a fixed `403` body:

- any path component starting with `.` (`.env`, `.git`, `.well-known`);
- directories named `node_modules`, `vendor`, `.venv`, `__pycache__`;
- editor leftovers: `.*.swp`, `.*.swo`, `#*#`, `*.tmp`, `*.bak`, `*.orig`, `*.rej`, `4913`,
  names ending in `~`;
- **symlinks**: a link anywhere below the root, to a file or a directory, even one that
  points inside the site. Replace it with a copy or serve the link's target as the root
  (`site-preview serve path/to/target`; only the root argument itself may be a link);
- names containing invisible Unicode characters that some filesystems ignore when comparing
  names.

The match is on whole path components, so `my-vendor-art.png` is fine. A `404` instead means
the file is missing, the directory has no `index.html` (there is no directory listing), or
the URL is malformed (bad `%` escapes, NUL, backslash, `..`, encoded slash). Compare the
request path with the file name's exact case.

### 405 Method Not Allowed

Only `GET` and `HEAD` are served; the server has no routes that change anything. A form
`POST` in your site needs a different server.

## A page does not reload

Work through these in order.

1. **Is the page served by site-preview?** A page opened from `file://` or another server has
   no reload script. View source: the document should end with
   `<script defer src="/__site_preview/client.js" data-instance=... data-revision=...>`.
2. **Is the watcher on?** `Watching:` must read `N eligible directories`, and no
   `watch-unavailable` or `watch-failed` warning may have appeared. See the next section.
3. **Was the page left unchanged on purpose?** Look for these warnings, each naming the path:
   - `html-too-large`: the page is over 8 MiB. Split it up.
   - `html-not-utf8`: the file, or its `<meta charset>`, is not UTF-8. Save it as UTF-8.
   - `html-rewrite-failed`: the script could not be inserted safely. Fix the markup the
     message points to.
   - `csp-blocks-live-reload`: see the next entry.
   Other pages are intact but cannot reload without a warning: a frameset document, a
   page loaded as a `<frame>`, a `</body>` inside SVG `<style>` text, an unclosed
   `<template>`. HTML fetched by script (`fetch`) is never rewritten.
4. **Did the edit count?** Changes inside hidden directories, `node_modules`, `vendor`,
   and files that match editor temporary names never advance the revision. Edit a file with a
   normal name, and note that a new file only matters once its final name appears.
5. **Is the edit inside the debounce window?** Many saves in a burst produce one reload after
   the site has been quiet for `--debounce` (150 ms by default). Continuous writes (a build
   tool rewriting files every few milliseconds) postpone the reload until they stop; raise or
   lower `--debounce` to suit.
6. **Does the stream connect?** In the developer console's network tab, `events` should stay
   open as `text/event-stream`. A failed stream (421, 403, 503) is retried with a backoff of
   1 s to 30 s, and a reload happens once it reconnects if the site changed meanwhile.
7. **Hot reload took over.** A stylesheet or image change is applied in place; the page does
   not reload. If the change shows no effect, the file is probably referenced in a way the
   client cannot refresh (a stylesheet fetched by script, an image set by script or inline
   style), in which case the client reloads fully; if neither happens, check for a
   `window.__sitePreview.accept` handler that returns a promise which never settles: after
   5 seconds the page reloads.

### `csp-blocks-live-reload`

The page's `<meta http-equiv="Content-Security-Policy">` forbids the reload script
(`script-src`, or `default-src` as fallback) or the reload connection (`connect-src`, or
`default-src`). The server never edits a policy and serves the page as written, so it will
not live reload until the policy allows `'self'` for both (a policy that lists only `https:`
sources does not match this plain-HTTP server). For development, add `'self'` to
`script-src` and `connect-src`, or use a looser policy locally. A `Content-Security-Policy`
sent as an HTTP header by something in front of the server is not inspected; its violations
show only in the browser console.

### Reload loops

The client reloads once when the server instance or revision differs from its page baseline,
including after a server restart. A page that reloads repeatedly usually has its own script
that triggers file writes (a build step touching served files, a service worker caching
old pages) so each reload causes a new change. Stop the writer, or keep generated files
outside the served root. A saved copy of a served page contains the preview tag; it
adopts the current instance and revision instead of looping.

## The watcher is degraded or unavailable

Live reload needs the operating system's file-change notifications. While the watcher is
degraded the server still serves pages, and it never claims otherwise: the `Watching:` line,
the warnings and, with `--json`, the `watcher` events all say so.

| Signal | Meaning |
|:--|:--|
| `Watching: N eligible directories (degraded: ...)` and `warning[watch-registration-failed]` | Some directories could not be watched; the others work. The warning names each directory (relative to the root, at most ten, then a count) and the reason. Registration is retried after later changes. |
| `Watching: unavailable (...); live reload disabled` and `warning[watch-unavailable]` | The native watcher could not start, or no directory could be registered. |
| `warning[watch-failed]` | The watcher failed while running; changes may be missed until restart. Reported once until it recovers. |
| `warning[reload-exhausted]` | The revision counter ran out (after 2^64 - 1 reloads). Restart. |
| `warning[root-removed]` | The site root was deleted, moved away or replaced. Live reload is no longer claimed, for good; see "The site root was removed" below. |
| `{"event":"watcher","state":"degraded","reason":"registration-failed"}` | The JSON counterpart of a registration failure. The other reasons are `runtime-failure`, `revision-exhausted` and `root-removed`. |
| `{"event":"watcher","state":"watching"}` | The watcher recovered. |

An agent reading `--json` should treat `"watching": false` in `ready`, or any `degraded` event,
as "edits will not reload the page": reload manually or restart the server.

### The site root was removed (`warning[root-removed]`)

site-preview opens the root once at start-up and serves through that open directory. When
the root is deleted (`rm -r`), moved away, or replaced by another directory at the same path
(a tool that rebuilds `dist/` by deleting and recreating it, a `git checkout` that swaps the
directory), the watcher finds the path no longer names the served directory. On Linux it
learns of it from a watch on the root's parent directory (the root's own watch is told nothing
while site-preview holds the root open, ADR 0073); when that directory cannot be watched (no
permission to read it), the next change below the root finds it instead:

- the watcher degrades with `root-removed`, once, and never recovers, even when a directory
  appears at the path again;
- the server keeps serving the directory it opened: a deleted one is empty (every request is
  `404`), a moved one is still served under its new name.

Restart site-preview to serve the directory now at the path. For a build tool that recreates
its output directory, serve the parent directory, or configure the tool to empty the
directory instead of deleting it.

### Linux: inotify limits

Each watched directory uses one inotify watch, and each watcher uses one inotify instance
(site-preview uses one instance, with one watch per eligible directory, not per file, and on
Linux one more on the root's parent directory, ADR 0073).
The messages are:

- `the system's file watch limit is reached (on Linux raise fs.inotify.max_user_watches)`
- `the system's limit on file watcher instances or open files is reached (on Linux raise fs.inotify.max_user_instances)`

Both limits are per user and shared with editors, file syncers and other dev servers, so a
machine running several tools can exhaust them even for a small site. Check and raise them:

```text
sysctl fs.inotify.max_user_watches fs.inotify.max_user_instances
sudo sysctl fs.inotify.max_user_watches=524288
sudo sysctl fs.inotify.max_user_instances=512
```

To keep the change across reboots, put the same two `key=value` lines in a file under
`/etc/sysctl.d/` and run `sudo sysctl --system`. In containers these are host settings and
must be raised on the host. Hidden directories, `node_modules` and `vendor` are never
watched, so they do not count against the quota. Serving a smaller directory (for example
`dist` instead of the project root) also helps. After raising a limit, restart the server.

A descriptor limit (`ulimit -n`) can produce the second message as well.

### Network mounts, shared folders and overflow

- Network file systems (NFS, SMB, some FUSE and container bind mounts, virtual machine shared
  folders) often deliver no native change events, so the watcher starts without error but
  never fires. There is no polling fallback. Run the server where the files are local, or
  sync them into a local directory.
- Event queues can overflow during a very large change (a branch checkout, a bulk copy). The
  server then rescans the watched directories and publishes at least one reload rather than
  losing the change.
- Editors that save by writing a temporary file and renaming it over the original are
  handled; the reload follows the final name.
- A directory that appears after start-up is registered automatically, and one that is
  removed is dropped. If it was created inside a hidden or excluded directory it is ignored,
  by design.
- On macOS the watcher uses FSEvents; the inotify settings do not apply. See
  [macOS](#macos-fsevents) below.

### macOS (FSEvents)

CI runs the product tests on `macos-15` (arm64) and `macos-15-intel` (OPS-03, ADR 0074).
What differs from Linux, by design of the platform:

- **Latency and coalescing.** FSEvents batches events: a change arrives some tens of
  milliseconds late, and several quick changes to one file may reach the watcher as one
  event (a create and a write may be a single notification). The server publishes at least
  one reload per burst, never one per write; no test or promise depends on an exact latency.
- **One recursive stream.** FSEvents covers a whole tree, so on macOS the watcher starts one
  recursive watch of the root and never restarts it (ADR 0075); directories need no
  registration and a directory copied in is covered at once. Changes in ignored directories
  (`node_modules`, hidden ones) wake the process and are then dropped, so a build that rewrites
  `node_modules` costs some CPU but never causes a reload.
- **Names that are not UTF-8.** APFS refuses to create them. On other volumes the watcher's
  FSEvents backend cannot handle one (upstream limitation, ADR 0047); a root path that is not
  UTF-8 starts in degraded mode (`watch-unavailable`).
- **Case-insensitive volumes.** The default macOS volume ignores case, so `INDEX.HTML` is
  `index.html` there, and the reserved-name and eligibility rules already compare names that way.
- **Temporary paths** under `/var` are really `/private/var`. The server resolves the root
  once and prints the resolved path; compare against that, not the path you typed.
- **Descriptor limit.** A macOS process starts with 256 open files; raise it with
  `ulimit -n 4096` before serving a very busy site.

## Shutdown

Ctrl-C (SIGINT) and SIGTERM stop the server with exit code 0. Open event streams are ended,
so a browser tab never keeps the process alive. In-flight downloads get up to 3 seconds. If
that takes longer than half a second, stderr says
`site-preview: shutting down: waiting up to ... for open connections; press Ctrl-C again to
stop now`; a second signal then exits immediately with code 130 (SIGINT) or 143 (SIGTERM).
There is no notice on an ordinary quick stop. A stale process after a crashed terminal can be
found by the `pid` in the `ready` event (`--json`) and stopped with `kill <pid>`.

## What to include in a bug report

The command line, the `Watching:` line, every `warning[...]` line, the operating system and
whether the site is on a local or network file system. Do not paste private paths or file
contents; the server's logs never contain query strings, but your shell history might. For a
security problem, follow [SECURITY.md](../SECURITY.md) instead of opening a public issue.
