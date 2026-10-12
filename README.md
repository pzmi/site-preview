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
| `html-too-large` | An HTML page over 8 MiB is served unchanged, without live reload. |
| `html-not-utf8` | An HTML page that is not UTF-8, or whose `<meta>` charset names another encoding, is served unchanged without live reload. |
| `html-rewrite-failed` | The reload script could not be inserted safely; the page is served unchanged. |

## Security posture (planned)

- Binds to loopback (`127.0.0.1`) by default; non-loopback binds require `--allow-remote` and print a warning.
- Dotfiles and hidden directories are denied.
- Symlinks beneath the site root are rejected.
- `Host` header checks guard against DNS rebinding.
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
