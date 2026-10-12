# site-preview

Zero-configuration local static-site preview server with live reload, as a single Rust binary.

> **Status: pre-alpha, under construction.** There is no code and no release yet. This repository currently contains the project plan and project metadata only. Everything under "Planned usage" describes the intended design, not working software.

## What it is

A globally installable development server that makes editing a static HTML site feel like editing Markdown. Point it at a directory containing `index.html` and it serves the site, watches for changes, and reloads the browser. No Node.js, npm, per-site configuration, build step, or changes to your source files are needed. Live-reload code is injected only into served HTML responses.

## What it is not

- Not a static site generator, bundler, or publishing service.
- Not a Node.js tool; the installed binary has no Node, npm, or Python runtime requirement.
- Not a browser security sandbox: it is a developer preview.

## Planned usage

Not implemented yet:

```text
site-preview serve [dir]                 # serve a directory (default: current directory)
site-preview serve --json                # machine-readable startup events (JSONL on stdout)
site-preview serve --port 4173           # exact port (default: OS-assigned)
site-preview serve --open                # open a browser (opt-in)
site-preview serve --allow-remote        # bind outside loopback (insecure, no auth)
```

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

## Security posture

- Binds to loopback (`127.0.0.1`) by default; a non-loopback bind requires `--allow-remote` and prints the `remote-binding` warning.
- Every request passes a `Host` gate first (421 for a name that is not this server), which guards against DNS rebinding.
- The reload stream (`/__site_preview/events`) refuses a foreign `Origin` or a cross-site `Sec-Fetch-Site` with 403; no CORS headers are sent and no route changes state.
- Dotfiles and hidden directories are denied.
- Symlinks beneath the site root are rejected.
- Source files are only read; the reload script is added to served responses, never to files on disk.
- `--allow-remote` has no authentication; do not use it on untrusted networks.

See [SECURITY.md](SECURITY.md) for scope and reporting, and [doc/security.md](doc/security.md) for the threat model and enforced rules.

## Supported platforms

Linux and macOS (ARM64 and x86_64). Windows is not supported.

## Documentation

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
