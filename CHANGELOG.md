# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - Unreleased

First release. The owner replaces `Unreleased` in this heading with the tag date when pushing `v0.1.0`
([doc/release.md](doc/release.md)). Install instructions: [README](README.md#install).

### Added

- `site-preview serve [DIRECTORY]`: a zero-configuration local server for a static site. Point it
  at a directory (default `.`) and open the printed `Local:` URL. One binary; no Node.js, npm,
  Rust or per-site setup is needed to run it, and nothing is written to your site.
- Live reload: a native file watcher with one global debounce (default 150 ms, `--debounce`)
  reloads open tabs once per settled burst of saves. Edits made while a tab was disconnected, and
  server restarts on the same port, recover with one reload and never loop.
- Hot swap: a changed stylesheet or image updates in place without a navigation; a changed script
  reloads once and restores the scroll position; a page can opt in to handling script changes with
  `window.__sitePreview.accept`.
- Serving: GET and HEAD, directory indexes, trailing-slash redirects, MIME types for common web
  and map formats (GPX, GeoJSON, KML, WASM, PDF), `Cache-Control: no-store`, no directory
  listings.
- `--json` for tools and AI agents: a flushed `ready` line with the URL (use `--port 0`, the
  default, for a free port), `watcher` events when live reload degrades or recovers, and stable
  warning codes on stderr. `--open` opens the browser; `AGENTS.md` and an example Claude skill
  describe the agent workflow.
- Graceful shutdown on SIGINT and SIGTERM with a short drain; a second signal exits at once.
- Release archives for Linux x86_64 and aarch64 (static musl) and macOS arm64 and x86_64, each
  with a CycloneDX SBOM, a `SHA256SUMS` file and a build-provenance attestation
  ([README](README.md#install)).

### Security

- Listens on loopback only. Any other address needs `--allow-remote`, which prints a warning;
  there is no authentication, so use it only on trusted networks.
- Host checking on every request (421 for another name) against DNS rebinding; the reload stream
  refuses foreign `Origin` and cross-site requests.
- Path confinement: one percent-decoding, a deny list per path component, and file opens through
  a directory handle with no symlink following. Traversal, encoded and double-encoded dots,
  symlinks anywhere under the root (even pointing inside it), and a symlink swapped in during a
  request are refused.
- Dotfiles and hidden directories (`.git`, `.env`, `.well-known`), `node_modules` and `vendor`
  are never served.
- Your files are only read. The reload script is added to HTML responses on document navigations
  and never to files, JSON, SVG, downloads or error pages; HTML over 8 MiB is served unchanged.
- No telemetry and no network requests of its own. Connection limits: 64 reload streams, 128
  connections, 10 s header and idle timeouts.
- Verified with unit, property, contract, end-to-end and Chromium tests, Quint-modelled
  lifecycle, watcher and reload protocols replayed against the code, fuzzing of the path, HTML,
  Host and change-sequence parsers, and line-coverage floors. See
  [doc/release-checklist.md](doc/release-checklist.md).

### Known limitations

- A restrictive Content Security Policy in a page can stop live reload; the policy is never
  changed (`warning[csp-blocks-live-reload]` for a `<meta>` policy).
- A few page shapes are served intact but do not live reload: a frameset, a page loaded as a
  `<frame>`, a `</body>` inside SVG `<style>` text, an unclosed `<template>`.
- No polling fallback: network mounts and some container shares send no change events, and OS
  event queues can overflow. The server then serves without live reload, or with the watcher
  degraded, and says so.
- Linux needs inotify and its watch quota (`fs.inotify.max_user_watches`) can run out on big
  trees; see [doc/runbook.md](doc/runbook.md).
- If the site root is deleted, moved or replaced, live reload stops (`root-removed`); restart to
  serve a directory recreated at that path.
- Range requests are answered with the whole file.
- Hidden files, including `.well-known`, cannot be served.
- Windows is not supported. The Linux and macOS minimum versions in the runbook are the
  toolchain's floor; CI covers only GitHub's Ubuntu 24.04 and macOS 15 runners.
- It is a developer preview, not a browser sandbox.

[Unreleased]: https://github.com/pzmi/site-preview/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pzmi/site-preview/releases/tag/v0.1.0
