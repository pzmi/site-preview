# 0045. Connection limits and a bounded shutdown drain

Date: 2026-10-12

## Status

Accepted (owner, 2026-10-14)

## Context

M1 served through `axum::serve`, which sets no header-read or idle timeout, caps no
connections and, under `with_graceful_shutdown`, waits for every open connection without a
bound. The E2E-01 checkpoint review found three consequences: a client that sends half a
request head, or stops reading a large file, keeps the process alive after Ctrl-C (and a
second Ctrl-C did nothing); a half-open connection stays open indefinitely; and a few hundred
of them exhaust a low descriptor limit (macOS gives a process 256 by default) and starve the
server. plan.md §9 asks for a graceful shutdown that does not deadlock on an open browser
connection, and §7.4 caps event streams at 64, but neither names a bound for ordinary
connections. plan.md §3 says a clean SIGINT or SIGTERM exits 0 and is silent on a second one.

## Decision

- Composition serves with hyper's HTTP/1 connection builder (`hyper` with `server` and
  `http1`, `hyper-util` with `tokio` and `service` for `TokioIo`, `TokioTimer` and
  `TowerToHyperService`) over its own accept loop, instead of `axum::serve`. Both crates
  were already in the lock file through axum; they become direct dependencies of
  `preview-composition` only. axum keeps only its `http1` feature there.
- `ServerLimits` in `ServeConfig` holds the numbers; the CLI never exposes them, tests shrink
  them. The defaults:
  - **Header read timeout 10 s.** hyper starts it whenever it waits for a request head, so it
    is also the idle keep-alive timeout. It never applies while a response (an event stream
    included) is being written. Browsers reopen a closed idle connection transparently.
  - **128 connections at once.** Room for the 64 event streams of §7.4 plus parallel page
    loads, and half of macOS's default descriptor limit, leaving the rest for the watcher and
    the files being served. A connection accepted beyond the cap is closed at once rather
    than left in the backlog: the client fails fast instead of hanging, and the server never
    holds more than the cap. A failed `accept` (for example `EMFILE`) pauses the loop for
    100 ms instead of ending the run.
  - **Shutdown drain 3 s.** The first SIGINT or SIGTERM closes the listener, ends every event
    stream (the HTTP-10 `Shutdown` trigger), asks each connection to finish its response, and
    waits at most 3 s; connections still open then are dropped and the process exits 0 (it
    did what it was asked). If connections are open, one line on stderr says so and that
    Ctrl-C again stops at once.
- A second SIGINT or SIGTERM during the drain drops every connection and exits with 128 plus
  the signal number (130, 143), as a shell reports a process stopped by that signal.
- `--json` and `--open` are refused with a usage error (exit 2) until CLI-06 and CLI-07 wire
  them, rather than accepted and ignored: an agent asking for JSON would otherwise parse
  human text.

## Consequences

- No client can hold the process after a stop for longer than the drain bound, and a second
  Ctrl-C always works. Pinned by the black-box tests in `crates/preview-cli/tests/lifecycle.rs`
  and the accept-loop tests in `crates/preview-composition/src/connections.rs`.
- The accept loop is ours to maintain (about one hundred lines). It speaks HTTP/1.1 only, as
  M1 already did (axum's `http2` feature was never enabled), and supports no upgrades.
- There is no write timeout: a client that stops reading holds its connection (and one of the
  128 permits) until it goes away or the server stops. Revisit if the cap is ever reached in
  practice.
- CLI-05 builds on this loop (moving signal handling to `shutdown.rs` and adding the watcher
  to the shutdown order) rather than on `axum::serve`'s graceful shutdown.
