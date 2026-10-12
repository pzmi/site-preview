# Site Preview — Rust Implementation Plan

**Status:** Implementation-ready engineering plan  
**Date:** 2026-10-10  
**Priority:** P0 — minimal, secure, working CLI  
**Language:** Rust (2024 edition)  
**Product specification:** “Site Preview — Zero-Configuration Live-Reload Server” (provided separately)

## 1. Outcome and scope

Build a globally installable, single-binary development server that makes editing static HTML sites feel like editing Markdown. A directory containing `index.html` must work immediately, with no Node.js, npm, package installation, project configuration, build step, or modification to source files.

The P0 end-to-end loop is:

```text
Agent or human edits index.html / CSS / JS / assets
    → native filesystem notification
    → global trailing-edge debounce
    → latest site revision published via SSE
    → browser reloads
    → human or Playwright verifies the rendered page
```

Use Rust for the server only. A site can contain ordinary browser-side JavaScript; the tool never compiles, bundles, runs, or manages it. Playwright MCP is an optional external consumer, not a runtime dependency. Site Preview is **not** a static-site generator or publishing service.

**Definition of “zero configuration”:** the user installs one executable once, then runs `site-preview serve .` in any ordinary static directory. Configuration is via optional CLI flags, not required project files.

## 2. Research conclusions and design decisions

| Area | P0 decision | Rationale / alternative rejected |
|---|---|---|
| HTTP / SSE | `axum` 0.8.x + `tokio` 1.x | Mature HTTP routing and native SSE without implementing protocol plumbing. Do not hand-roll HTTP on raw TCP. |
| Watching | `notify` 8.2.x, platform native watcher | Uses Linux inotify and macOS FSEvents. `notify` also supports native recursive watching, unlike Go fsnotify. |
| Watch registration | `NonRecursive` watches on *eligible directories* | Deliberately avoids watching `.git`, `node_modules`, hidden dirs, etc. More code than `Recursive`, but enforces the privacy/resource requirement rather than merely filtering events. |
| Debounce | One small in-process trailing-edge timer | `notify-debouncer-mini` debounces **per path**; our contract needs **one reload per whole-site burst**. No additional debouncer dependency. |
| Broadcast | `tokio::sync::watch<SiteState>` | Stores one latest `(instance_id, revision)` value. No per-browser unbounded queues; slow readers coalesce revisions. |
| HTML injection | `lol_html` 3.x | Parser-aware insertion before the real `</body>`; unlike a case-insensitive string replace, does not mistake strings/comments for closing tags. |
| Secure file opens | `rustix` 1.x (`openat`, `O_NOFOLLOW`) | Traverse path components relative to an opened root directory fd. Protects against `..`, symlink escapes, and TOCTOU path-replacement races on the two supported Unix-like OSes. |
| CLI | `clap` 4.x | Handles subcommands, help, version and flags before **or after** the positional directory. Avoid Go flag-parser ordering limitation. |
| MIME | `mime_guess` 2.x + four overrides | Explicit GPX, GeoJSON, KML and WASM handling; avoid maintaining a full MIME table. |
| JSON | `serde_json` 1.x | A stable, escaped machine-readable ready event and SSE payloads. |
| Asset transfer | Stream non-HTML files; cap HTML rewrite size | No reading giant images/PDFs into memory and no large HTML rewrite allocations. |
| Opening browser | Explicit `--open`; invoke OS browser launcher without a shell | On macOS use `open`, on Linux `xdg-open`. Treat launcher failure as a warning, not a server failure. |
| Logging | Concise stderr diagnostics | No analytics, request-body logging, query-string logging, cookies, telemetry or remote logging. No structured logging framework needed in P0. |

### 2.1 Important difference from Go

The original Go plan says each nested directory must be registered because `fsnotify` is not recursive. That does **not** apply to Rust `notify`: `RecursiveMode::Recursive` covers new subdirectories too. Nevertheless, **choose non-recursive per-directory registration in P0** because the product explicitly forbids watching ignored subtrees. The implementation should be small: one traversal, one set of registered paths, and a rescan on directory changes.

An acceptable implementation simplification is to use a native recursive watch and filter ignored events **only if** the original requirement is relaxed to “do not react to ignored directories” rather than “do not watch them.” Do not claim those are equivalent.

### 2.2 Do not use `tower_http::services::ServeDir` blindly

`ServeDir` is an excellent general-purpose static-file service, but Site Preview imposes stricter rules: deny every dotfile, deny certain non-dot directories, reject all symlink components, prevent directory indexes, and inject only navigation HTML. Use a deliberately small custom GET/HEAD handler with secure file opening instead of wrapping an unrestricted file server and hoping a prefilter catches all cases.

### 2.3 Dependencies

Start with these dependency **families** and lock concrete patch versions in `Cargo.lock` when creating the project:

```toml
[package]
name = "site-preview"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "signal", "sync", "time", "fs"] }
notify = "8.2"
clap = { version = "4", features = ["derive"] }
lol_html = "3"
rustix = { version = "1", features = ["fs"] }
mime_guess = "2"
percent-encoding = "2"
serde_json = "1"
tokio-stream = { version = "0.1", features = ["sync"] }
tokio-util = { version = "0.7", features = ["io"] }

[dev-dependencies]
tempfile = "3"
```

The list is a starting point, not a claim that the manifest has been compiled. Remove a dependency if the final design doesn't use it; enable additional small Tokio features as required by the selected runtime APIs. Prefer the latest **stable** compatible release, not a `notify` v9 release candidate without an explicit reason. The distributed executable does not require Rust or Cargo.

## 3. CLI and machine interface

### Command

```text
site-preview serve [--host IP] [--port N] [--open] [--debounce 150ms]
                   [--json] [--allow-remote] [directory]
site-preview --version
site-preview --help
```

- Default `directory` = `.` (the current directory, **not** its parent).
- Default bind = `127.0.0.1`, port = `0` (OS-assigned); `--port 4173` means exactly 4173, otherwise exit nonzero if unavailable.
- Allow `site-preview serve ./site --port 4173` **and** `site-preview serve --port 4173 ./site`.
- Accept `--debounce` durations such as `150ms` and `1s`; reject zero or nonsensically large durations with a helpful error.
- Bind outside loopback (including `0.0.0.0` and `::`) only if `--allow-remote` is supplied. Print a security warning to stderr.
- Never automatically open a browser; `--open` is opt-in.
- Resolve the requested root once at startup. Require a readable directory.
- Support two or more independent server processes without shared locks or runtime files.

### Human startup output (example)

```text
Site Preview
Serving:  /Users/me/motorcycle-routes
Local:    http://127.0.0.1:49152/
Watching: 3 eligible directories
Ready.
```

Prefer counting **watched directories** rather than claiming to watch individual files; the OS watchers are directory-based.

### JSON startup protocol

After the listener has been bound, route state initialized, and watcher started or explicitly marked unavailable, flush one newline-delimited JSON event on stdout:

```json
{"event":"ready","url":"http://127.0.0.1:49152/","pid":12345,"watching":true}
```

- `--json` stdout is strictly JSONL, never human logs or progress bars.
- Errors and human-oriented warnings go to stderr.
- Emit a further JSONL status event if the watcher later changes state; document its schema. Do not emit a second `ready` event.
- Call `stdout().flush()` after readiness so agents reading a pipe can discover the URL promptly.
- Exit with code `0` for clean SIGINT/SIGTERM; nonzero for invalid CLI, inaccessible root, listener bind failure, or fatal server initialization errors. A watcher failure is **degraded mode** (HTTP still works), not a false success claim about live reload.

## 4. Recommended repository organization

```text
site-preview/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── AGENTS.md                         # sample agent workflow
├── src/
│   ├── main.rs                       # CLI, process lifetime, startup JSON
│   ├── serve.rs                      # routing, GET/HEAD, MIME, HTML injection
│   ├── paths.rs                      # path filtering and secure file opening
│   ├── watcher.rs                    # notify, watch-set, debounce, errors
│   ├── reload.rs                     # revision state, SSE, embedded client
│   └── client.js                     # embedded using include_str!()
├── tests/
│   ├── http.rs
│   ├── security.rs
│   ├── watcher.rs
│   └── lifecycle.rs
├── fixtures/
│   └── site/
│       ├── index.html
│       ├── styles.css
│       ├── app.js
│       ├── routes.gpx
│       └── images/sample.svg
└── .github/workflows/
    ├── ci.yml
    └── release.yml
```

Keep it a **single binary crate**. Split modules by concerns only where the code actually warrants it; no traits-for-everything, plugin model, database, generic message bus, configuration engine, or framework abstraction.

## 5. Serving files safely

### 5.1 HTTP contract

| Request | P0 response |
|---|---|
| `GET /` | `index.html` from root, optionally with reload script injected. |
| `GET /guide/` | `guide/index.html`, if present. |
| `GET /guide` when directory exists | Temporary redirect to `/guide/` to preserve relative asset URLs. |
| `GET /image.webp`, `.svg`, `.gpx`, `.geojson`, `.kml`, `.wasm`, `.pdf` | Original bytes, correct MIME, `Cache-Control: no-store`. |
| `HEAD /...` | Same status / relevant headers as GET, **no body**. For HTML this includes the injected `Content-Length`, so HEAD does the GET's rewrite work under the HTML memory budget (ADR 0046). |
| Missing / forbidden paths | 404 or 403 without file contents; never a directory listing. |
| POST/PUT/PATCH/DELETE | 405 Method Not Allowed. |
| Internal reserved endpoints | Served solely by internal handlers, never from disk. |

Explicit MIME overrides:

```text
.gpx     application/gpx+xml
.geojson application/geo+json
.kml     application/vnd.google-earth.kml+xml
.wasm    application/wasm
```

For browser-rendered HTML, emit `text/html; charset=utf-8` when the file's encoding is compatible. Do not mislabel arbitrary non-UTF-8 files; either preserve their declared charset or serve unchanged with a warning when safe rewriting is unavailable.

Use `Cache-Control: no-store` on static responses and internal endpoints. Stream binary assets. Range requests are **not a P0 requirement**: an ordinary full `200` response must still display PDFs/images; if a real consumer requires `206 Partial Content`, add tested single-range support separately. Do not advertise `Accept-Ranges: bytes` without implementing it.

### 5.2 URL decoding and filtering

Normalize URL paths in one function used before *any* file open. Rules:

1. Decode percent-encoding **once**; reject invalid encoding, NUL, backslash and encoded separator tricks.
2. Require relative components beneath the fixed root; reject `.` and `..` components, including encoded equivalents.
3. Deny any component starting with `.`; this covers `.env`, `.git`, `.well-known`, etc. Document `.well-known` as an intentional P0 limitation.
4. Deny exact directory components `node_modules`, `vendor`, `.venv`, `__pycache__` and editor/temp names. Match **components**, never substrings (e.g., `my-vendor-artwork` is fine).
5. Deny editor swap/backup patterns (`.*.swp`, `.*.swo`, `#*#`, `*.tmp`, `*.bak`, `*.orig`, `*.rej`, `4913`) and names ending in `~`; deny non-files and directories lacking `index.html`.
   Names are compared after a case fold (case-insensitive filesystems), and a component holding a code point HFS+ ignores when comparing names (zero-width characters, directional controls, U+FEFF) is denied outright, so `node\u{200C}_modules` cannot open `node_modules` (the CVE-2014-9390 class).
6. Never interpolate a URL path into a shell command.

Store path components as validated relative names. Handle URL-encoded spaces and Unicode filenames normally; reject malformed encodings.

### 5.3 Symlink-safe open algorithm

At startup, canonicalize the explicitly selected root **once** and open an fd for it. The root itself may have been specified via a symlink; all *served path components below that root* are treated strictly.

For each validated component, use `rustix::fs::openat` relative to the previous directory fd. For intermediate directories, require `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`; for the final file require `O_NOFOLLOW | O_CLOEXEC` and read-only access, then verify the fd refers to a regular file. Open the final file by fd rather than first checking a path and reopening it. The same root-relative approach applies to directory `index.html` resolution.

This is intentionally stricter than `canonicalize(candidate).starts_with(root)`: the latter has check/open races and follows symlinks by default. Avoid string-prefix checks for filesystem confinement. Do **not** rely on `O_NOFOLLOW` on the last component alone; every path component needs protection.

Only Linux and macOS are P0 targets, so this small POSIX-style implementation is acceptable. Write tests that replace a directory with a symlink in a concurrent loop while requests run. Document that this is **HTTP-serving confinement**, not a sandbox preventing other processes running as the same user from reading files.

### 5.4 Host and Origin rules

- Default permitted HTTP Host authority: the selected loopback address and bound port, with a narrowly defined `localhost` alias if desired. Reject unexpected DNS names and mismatched ports to mitigate DNS rebinding.
- In `--allow-remote` mode, accept numeric Host IP authorities that match the explicit bound IP; for wildcard listeners accept numeric destination addresses, **not arbitrary DNS names**. Document that there is no authentication and LAN access is deliberate.
- No permissive CORS headers.
- On the internal SSE endpoint, reject conflicting `Origin` and explicitly cross-site `Sec-Fetch-Site` requests. Requests without those browser headers may still be accepted after Host validation (agents / curl).
- Do not require custom request headers: native browser `EventSource` does not support arbitrary ones.

## 6. HTML script injection

Embed `src/client.js` at compile time with `include_str!("client.js")`. The internal route is `/__site_preview/client.js`; the SSE route is `/__site_preview/events`.

The injected tag carries a **render-time revision snapshot** (captured immediately before reading the HTML). These attributes are per server instance, not visitor identifiers:

```html
<script defer src="/__site_preview/client.js"
        data-instance="startup-abc" data-revision="42"></script>
```

The simpler tag without data attributes works for ordinary reloads but has an important race: an edit occurring between fetching the HTML and establishing the SSE connection would otherwise be silently missed. The injected snapshot closes that gap. Keep its fields escaped and validated; don't inject inline executable JavaScript.

Algorithm:

1. Only consider a successful GET of `text/html` (not XHTML, SVG, XML, error responses or HEAD).
2. When `Sec-Fetch-Dest` exists, inject only for `document` or `iframe`. If it is `empty`, do not modify an HTML response fetched by JS. For clients without Fetch Metadata headers, use a documented best-effort fallback; do not guarantee navigation/data distinction for very old clients.
3. Bound rewrite input to **8 MiB**. Stream larger HTML unchanged and warn once per path; keep serving the page.
4. Parse with `lol_html`. Insert the tag before the closing `body` tag, or at document end when absent. Never match `</body>` inside JavaScript, comments, attributes or text by naïve byte searching.
5. If the exact preview client script is already present, do not insert a duplicate; set its `data-instance` and `data-revision` to the current snapshot instead (a saved copy of a served page would otherwise reload forever; ADR 0043). Rewrite in memory only; never write back to source files.
6. If rewriting fails or a document encoding cannot be handled safely, serve the original bytes without injection and report an actionable warning. Never serve corrupted HTML merely to achieve live reload.
7. Compute `Content-Length` **after** rewriting (or use correct streaming framing); remove stale validators such as `ETag` when changing response bytes.
8. Preserve existing CSP. Never insert permissive policy headers, rewrite `Content-Security-Policy`, or bypass integrity requirements. Warn if detectable policy restrictions block `script-src 'self'` or `connect-src 'self'`; document that some CSP violations are discoverable only in the browser console.

At startup, if `__site_preview` already exists as a real file or directory in the root, fail with an actionable **reserved-path conflict**. Never silently shadow users' site content.

## 7. Live-reload protocol and client state machine

### 7.1 Server state

```text
SiteState {
  instance_id: String,  // unique per process start, not a visitor ID
  revision: u64,         // initially 0, incremented after debounced changes
}
```

Generate a new non-secret instance ID on each process start (e.g. process ID combined with high-resolution startup timestamp). It disambiguates a server restart from a revision reset. This is per server **instance**, never per user/browser; it is not telemetry.

Keep the latest `SiteState` in `tokio::sync::watch`. Every successful debounce publishes an updated value. Subscribers receive the newest value, so no queue grows for disconnected or slow browsers.

### 7.2 SSE contract

On each connection, subscribe to watch state **before** capturing its initial value, then send a `state` baseline event:

```text
event: state
id: 42
data: {"instance_id":"startup-abc","revision":42}

```

On each later revision (from that subscription), send:

```text
event: reload
id: 43
data: {"instance_id":"startup-abc","revision":43}

```

Send a keepalive comment approximately every 15 seconds. Set `Content-Type: text/event-stream`, `Cache-Control: no-store`, and `X-Content-Type-Options: nosniff`. Avoid response buffering. `Last-Event-ID` may be received on reconnection but is not needed for replay: every new connection sends the latest authoritative `state`.

### 7.3 Browser algorithm

The client holds ephemeral, in-memory variables only; **no cookies, localStorage or unique browser IDs**. The server embeds the HTML response's instance/revision in the script attributes. Comparing this with the first SSE event catches a source edit between fetching the document and opening the event stream.

```text
baseline = read currentScript.dataset.instance / dataset.revision
reloading = false
connection = EventSource("/__site_preview/events")

differs(state, baseline):
    return state.instance_id != baseline.instance_id
        OR state.revision != baseline.revision

on state(serverState):
    if baseline is missing:       // defensive fallback for old injected pages
        baseline = serverState
        return
    if differs(serverState, baseline) AND NOT reloading:
        reloading = true
        location.reload()        // catches missed changes and server restarts

on reload(serverState):
    if differs(serverState, baseline) AND NOT reloading:
        reloading = true
        location.reload()
```

A page navigated/reloaded normally gets a new render-time baseline. A still-open page whose connection drops retains its baseline; after reconnection it compares the new authoritative `state` and reloads **once** if the revision or instance changed. A server restart can therefore cause one recovery reload, not an infinite loop. Avoid using `EventSource.lastEventId` as the sole identifier, because a revision resets to zero on restart.

**Snapshot ordering:** read the current SiteState immediately **before** opening the document for serving and inject that value into the response. A subsequent file change then advances the server state. At worst a concurrent save produces one redundant reload, not a permanently stale page.

In P0, **always full-page reload**, including CSS-only changes. P1 may send CSS-specific events with safe fallback.

### 7.4 SSE resource limits

- Limit to **64 simultaneous** SSE connections by default; return 503 if exceeded.
- Use `tokio::sync::watch` (latest value) instead of storing per-client event queues.
- Reclaim the connection permit immediately on disconnect or shutdown.
- Keep network requests same-origin and use browser-native reconnect delays, not application busy polling.
- Explicit shutdown closes all streams and the listener.

## 8. Filesystem watching and global debounce

### 8.1 Startup

1. Traverse the canonical root without following symlinks.
2. Apply the **same eligibility predicate** used by file serving; exclude hidden and ignored subtrees before descent.
3. Install one `notify::RecommendedWatcher` and call `watch(path, NonRecursive)` for each allowed directory, including root.
4. Retain the watcher object for the entire process lifetime.
5. Track watched directories in a `HashSet<PathBuf>`; use it to avoid duplicate watches and remove stale registrations after changes.
6. If initialization or any watch registration fails, report exactly which directory and why. Keep HTTP available; expose degraded watcher state rather than printing an unconditional `Watching: enabled`.

**New directories:** on creation/rename of an eligible directory, traverse it recursively and register all eligible descendants. On removal, remove obsolete watches and rescan the nearest existing parent. Handle editors that write a temporary file and atomically rename it over the original.

### 8.2 Event classification

Relevant: file creation, write/data modification, delete, rename and directory tree changes. Ignore access-only and metadata-only events where distinguishable. Ignore eligible-temp-name changes until the final filename appears. If the OS supplies coarse or ambiguous events, rescan relevant directory entries instead of inventing precision.

Check ignored path components *before* publishing changes. A `.git` or `node_modules` update must not increment the revision.

### 8.3 One **global** trailing-edge debounce

```text
on relevant event:
    dirty = true
    deadline = now + debounce_duration

on new relevant event while dirty:
    deadline = now + debounce_duration

when deadline passes:
    if dirty:
        reconcile directory watches
        revision += 1
        publish latest SiteState
        dirty = false
```

Default debounce = 150 ms. Add a maximum settling interval of around 1 second only if practical editor workloads reveal indefinite postponement; it is not necessary to implement an adaptive debounce in the MVP.

Bridge the synchronous `notify` callback into the Tokio watcher task with a bounded channel. If the channel fills, set a shared atomic `overflow/dirty` flag and wake the task so it does a full eligible-directory rescan and publishes at least one reload. Never block a native OS watcher callback indefinitely and never silently lose all change notification.

### 8.4 Failure behavior

If `notify` fails at runtime, log a warning and mark watch state degraded. Retry only a small bounded rescan/re-registration when meaningful; never spin or constantly retry. HTTP file serving must continue. A polling fallback is **P1**, and the tool must not claim live reload is active when notifications are unavailable.

Known issues requiring documentation: network mounts may not emit native events, Linux inotify quotas may be exhausted, and OS event streams can occasionally overflow. The `notify` documentation explicitly calls out these limitations.

## 9. Lifecycle and concurrency

Use one Tokio runtime and these long-lived concerns:

```text
main
 ├── parse CLI and validate root
 ├── open root file descriptor
 ├── bind TCP listener (OS assigns port when zero)
 ├── initialize SiteState / broadcast channel
 ├── construct HTTP routes and SSE handler
 ├── initialize filesystem watcher and debounce task
 ├── emit and flush ready JSON (or human output)
 ├── optionally open browser
 ├── serve requests until SIGINT or SIGTERM
 └── graceful shutdown: listener → SSE → watcher → tasks
```

Do not spawn a task per watched file. Reading/reworking a large file must not block the event loop; use `tokio::fs`, `spawn_blocking` where appropriate, or a stream constructed from securely opened file descriptors. All background tasks should be owned, cancellable, and joined or bounded on shutdown. Avoid deadlocks if a browser connection remains open during shutdown.

## 10. Agent integration

Create a concise `AGENTS.md` and optional Claude Code skill **in this tool's repository** as examples. They do not alter an end user's site.

Proposed agent instructions:

```markdown
# Static HTML visual verification

When creating or editing a static HTML artifact:

1. Detect the root containing `index.html`.
2. Start `site-preview serve --json --port 0 <root>` as a managed long-running process.
3. Parse the first JSONL `ready` event and capture its `url` and `pid`.
4. Check that the URL returns a successful HTTP response and that `watching` is true.
5. Open the page using the already configured Playwright MCP browser.
6. Inspect screenshots at 390 px, 768 px, and 1440 px viewport widths.
7. Inspect a full-page screenshot, accessibility snapshot, console errors, and failed local requests.
8. Exercise visible controls, responsive layouts, links, map interactions, and local assets relevant to the artifact.
9. Edit source files; observe the updated rendered page via live reload.
10. Repeat until visual and interaction checks pass.
11. Stop the preview process when the task is complete, including failure paths.

Do not infer visual correctness solely from HTML source or DOM snapshots.
Do not bind to `0.0.0.0` to solve container/VM reachability.
```

**Important isolation caveat:** a Playwright process in another container/VM cannot access the coding agent's `127.0.0.1` merely because both processes use loopback. Prefer agent/browser co-location or explicit, access-controlled port forwarding. Playwright MCP itself may use Node.js, but it must remain an **external test/agent dependency**, never a prerequisite for installing or running Site Preview.

## 11. Implementation order: five small, reviewable milestones

### M1 — CLI, serving and boundary tests (P0)

- Create Rust binary crate and `serve` command, version and help.
- Validate root and bind localhost with ephemeral port by default.
- Implement secure component parsing and `openat` file reads.
- Add GET/HEAD, directory indexes, redirects, MIME, no directory listings, `no-store`, blocked paths.
- Implement `--port`, `--host`, `--allow-remote` and Host checks.
- Test traversal, symlinks, dotfiles, forbidden directories, port conflicts and multiple instances.

**Exit gate:** one binary securely serves a sample site without modifying its files.

### M2 — HTML injection + SSE (P0)

- Embed `client.js` and reserve internal routes.
- Add bounded `lol_html` rewrite on eligible document navigations.
- Implement latest-state channel and SSE `state` / `reload` events.
- Implement browser baseline/reconnect algorithm, keepalive and 64-connection limit.
- Test HTML edge cases, MIME isolation, HEAD, CSP warning, two open tabs, reconnect and restart.

**Exit gate:** manually updating `revision` makes an open browser reload exactly once, with no page-load loop.

### M3 — Native watcher + debounce (P0)

- Install non-recursive watches for eligible directory tree.
- Handle create, rename, delete, atomic replacement, newly created nested directories.
- Add global 150 ms debounce; publish one revised state per settled burst.
- Handle ignored events, errors, event overflow and shutdown.
- Add OS integration tests with real filesystem writes and a real listener.

**Exit gate:** saving any relevant source/asset causes exactly one ordinary browser refresh, while `.git` changes do not.

### M4 — Agent workflow + polish (P0)

- Add stable `--json` JSONL readiness, structured warnings and stdout flushing.
- Add graceful SIGINT/SIGTERM handling and opt-in `--open`.
- Write README, `AGENTS.md` / Claude example, and a Playwright smoke test.
- Add integration tests using a real HTTP connection and editable fixture directory.
- Verify browser screenshots at mobile/tablet/desktop widths and local asset fetching.

**Exit gate:** an automated agent can start, discover, visually inspect, edit, confirm reload and shut down the server without manual setup.

### M5 — Distribution and quality gate (P0)

Build and publish assets:

```text
site-preview-aarch64-apple-darwin.tar.gz
site-preview-x86_64-apple-darwin.tar.gz
site-preview-aarch64-unknown-linux-musl.tar.gz
site-preview-x86_64-unknown-linux-musl.tar.gz
SHA256SUMS
```

- Native CI where available: ARM macOS, Intel macOS (`macos-15-intel`), Linux AMD64, Linux ARM64.
- Use musl targets for self-contained Linux binaries; validate on clean minimal Linux images and document minimum kernel expectations.
- Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, tests, and OS-specific smoke tests.
- Test the **published archive binary**, not only `cargo run`.
- Tag `v0.1.0`; include checksums and a minimal install command in README. Homebrew can follow P0 if manual installation is satisfactory.

**Exit gate:** all four binary targets can preview the same fixture without Rust, Node or npm installed.

## 12. Mandatory test matrix

### Pure unit tests

- URL decoding: `%2e%2e`, `%2f`, `%5c`, double encoding, malformed `%`, NUL, Unicode, valid spaces.
- Ignore rules match path **components**, not substrings.
- MIME overrides and `Content-Type` for HTML/CSS/JS/SVG/JSON/GPX/GeoJSON/KML/WASM/PDF.
- One global debounce emits one site revision for an event burst.
- Embedded render-time revision is compared with initial SSE `state`; later `reload` detects changes.
- Server instance change is distinguishable from revision change.

### HTTP and security integration tests

- `/` and nested indexes; directory slash redirect; no directory listing.
- `GET` and `HEAD`; 404, 403, 405; no-store; content length after rewrite.
- HTML rewrite exactly once; source bytes never change.
- Script/HTML comments containing fake `</body>` and mixed-case closing tags.
- HTML fetched with `Sec-Fetch-Dest: empty` is not modified.
- HTML larger than limit is served unchanged with warning.
- No script injection into JSON, SVG, GPX, KML, file downloads or error responses.
- `.git/config`, `.env`, `vendor`, `node_modules`, symlink to outside, symlink in middle of path, atomic symlink swap.
- Host mismatch, forbidden remote binding, disallowed Origin, port in use.
- Explicit internal-route conflict is reported at startup.

### Watcher integration tests (run on macOS **and** Linux)

- Update HTML, CSS, JS, GPX, JSON; create and delete files.
- Rename/atomic save; create nested directory then update file inside it.
- Rename/delete a watched directory, then create it again and edit inside.
- Burst of edits generates one revision after quiet period.
- `.git`, temp file and excluded-tree changes generate no revision.
- Unavailable watcher gives a warning but continues serving HTTP.
- Simulated channel overflow causes rescan / at least one reload.

### Browser integration test

1. Start executable with `--json --port 0` against a temporary fixture.
2. Read ready event and navigate browser to URL.
3. Verify initial text and that the page does not instantly reload in a loop.
4. Edit original HTML and wait for new visible text.
5. Check no unhandled browser exceptions or missing local assets.
6. Disconnect SSE, edit HTML, reconnect SSE and verify one recovery reload.
7. Restart server on the same explicit port; verify no infinite reload.
8. Shut down browser and server; check exit status and process cleanup.

Use bounded waits with generous CI timeouts; never assert an exact 150 ms wall-clock latency under load.

## 13. Performance and resource gates

For an ordinary local site of <= 1,000 files:

- Target startup under 1 second on a typical development laptop, measured after binary launch.
- Target visible browser refresh within about 500 ms of a stable save.
- Idle CPU close to zero; file notifications and SSE are event driven.
- No work done when unchanged except occasional SSE keepalives.
- HTML rewrite allocation bounded by 8 MiB input plus output overhead.
- Binary asset reads streamed instead of buffered in full.
- Maximum 64 live SSE connections; no per-client accumulating queue.
- Connection limits (D44, ADR 0045): a 10 s header-read and idle keep-alive timeout, at most 128 open connections (an excess one is closed at once), and a 3 s shutdown drain; a second SIGINT/SIGTERM exits at once with 130/143.

Benchmark rather than over-engineer. Run at least one soak test (repeated saves + multiple tabs) and a test with nested directories and ignored large trees. If the native watcher hits OS limits, report them clearly.

## 14. P1, explicitly deferred

Only consider these after observing actual user friction:

1. CSS-only refresh retaining page state (with full-reload fallback).
2. Polling fallback for network filesystems / broken native notifications.
3. Configurable SPA fallback.
4. One-shot build command (careful to avoid executing site-provided scripts implicitly).
5. Machine-readable health/status and richer errors (beyond P0 ready/warnings).
6. Local HTTPS.
7. Custom ignore patterns or selectively exposing `.well-known`.
8. Browser-console forwarding.
9. HTTP byte ranges for large PDFs/media if real usage needs them.
10. Homebrew tap/formula and automated release signatures.

Do not add these “just in case.” In particular, do not invent HMR, bundling, JavaScript compilation, frontend framework integration, plugins, telemetry, a deployment backend or a site database.

## 15. Known limitations / explicit policy choices

- Files needing a restrictive CSP may preview correctly but **not live reload**; preserve security policy and warn.
- Some pages are served intact but do not live reload, without a warning: a `</body>` inside SVG `<style>` text or an unclosed `<template>` (the script lands where it is inert), a frameset document (no body to run it in), and a page loaded as a `<frame>` (`Sec-Fetch-Dest: frame`, never injected).
- The client is injected on document navigation, not all fetched HTML data. Old browsers lacking Fetch Metadata allow only best-effort classification.
- All hidden file/directory components, including `.well-known`, are inaccessible in P0.
- Symlinks anywhere beneath the site root are rejected, even if they point within the root.
- Source files are never modified, but **HTTP HTML responses** are intentionally rewritten for preview.
- Polling is absent in P0; some NAS/network-mounted directories cannot provide reliable native change events.
- Browser-side third-party scripts/assets in the user's **site** may make external requests; Site Preview itself makes none. This is a developer preview, not a browser security sandbox.
- `--allow-remote` is explicitly insecure for untrusted networks because there is no authentication.
- Browsers send Fetch Metadata (`Sec-Fetch-*`) only to potentially trustworthy origins, so a page opened at `http://<LAN address>` under `--allow-remote` has none: HTML injection uses its fallback classification as the normal case, and a cross-site `no-cors` request to `/__site_preview/events` (no `Origin`, no `Sec-Fetch-Site`) passes the SSE checks and can hold connection slots (§7.4) until they run out. It cannot read the stream (ADR 0037, amended).
- P0 responds to Range requests with a full resource rather than promising partial-content support.

## 16. Definition of done

- [ ] One executable works without Rust, Node, npm or per-site setup.
- [ ] `site-preview serve .` serves HTML and ordinary static assets on localhost.
- [ ] Source assets and HTML are **never modified** by the tool.
- [ ] File changes reliably cause one debounced browser reload.
- [ ] Initial connection, disconnect, reconnection, and restart do not reload endlessly.
- [ ] `--json --port 0` provides a stable flushed URL for automation.
- [ ] Playwright MCP can inspect the actual rendered page and repeat after edits.
- [ ] Dotfiles, excluded directories, traversal, symlink escapes and DNS rebinding are handled.
- [ ] Watcher failure is visible, while basic HTTP stays usable.
- [ ] CI tests and smoke tests pass for macOS ARM64/x86_64 and Linux ARM64/x86_64.
- [ ] v0.1.0 releases have working archives and SHA-256 checksums.

## 17. Reference projects and primary documentation

**Product-behavior oracles:**

- Five Server — https://github.com/yandeu/five-server — developer experience, injection, browser reloading and CSS behavior; Node implementation is not a dependency.
- Python LiveReload — https://github.com/lepture/python-livereload — simple preview workflow.
- WatchDoc (Go) — https://github.com/absmach/watchdoc — file watching and minimal server structure; its platform/security assumptions must not be copied.
- Original Live Server — https://github.com/tapio/live-server — reload UX.
- miniserve (Rust) — https://github.com/svenstaro/miniserve — mature single-binary distribution and HTTP behavior.
- webserve (Rust) — https://docs.rs/webserve/latest/webserve/ — compare existing live-reload static server rather than reinventing unexamined behavior.
- Playwright MCP — https://github.com/microsoft/playwright-mcp — official agent/browser integration.

**Implementation-authoritative references:**

- Notify documentation — https://docs.rs/notify/latest/notify/ — recursive vs nonrecursive semantics, OS limits, filesystem caveats.
- Notify project — https://github.com/notify-rs/notify — releases and platform behavior.
- Axum SSE — https://docs.rs/axum/latest/axum/response/sse/ — Event and KeepAlive APIs.
- Rustix `openat` — https://docs.rs/rustix/latest/rustix/fs/fn.openat.html — directory-relative secure file opens.
- Cloudflare `lol_html` — https://github.com/cloudflare/lol-html — parser-aware response modification.
- Clap — https://docs.rs/clap/latest/clap/ — CLI argument parsing.
- GitHub Actions runner images — https://github.com/actions/runner-images — release architecture availability.

**Implementation principle:** prefer five small milestones, a short codebase and a tested happy path over broad feature parity with Five Server, BrowserSync, Vite, or miniserve.
