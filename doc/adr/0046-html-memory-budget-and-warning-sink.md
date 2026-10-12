# 0046. HTML memory budget, HEAD parity and the warning sink

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

## Context

plan.md §6 has HTML of at most 8 MiB read whole, checked for UTF-8 (ADR 0022) and rewritten
in memory. The M1 review found that every concurrent request then holds its own copy until
the client has read it: 200 stalled GETs of an 8 MiB page took the process to 1.65 GB. plan.md
§5.1 asks HEAD for "the same relevant headers as GET", and the injected GET's
`Content-Length` is only known after the rewrite. Warnings must reach stderr in the ADR 0020
format, once per path, but the HTTP adapter cannot name the CLI.

## Decision

- **Budget.** Every HTML response that holds a page in memory reserves twice the page size
  plus 64 KiB from one shared byte budget (a `tokio::sync::Semaphore` counted in KiB,
  `HtmlBudget`, 128 MiB by default) before reading it, and keeps the reservation until its
  body has been sent or dropped. The body is sent in 64 KiB copies, so a connection's write
  buffer never pins the whole page after the reservation is released. A request that does
  not fit waits for a release; a single page larger than the whole budget takes all of it.
  Waiting was chosen over serving unchanged when the budget is spent: unchanged service would
  have to label a page without having checked its encoding (mojibake for UTF-8 pages without
  a `<meta charset>`) and would silently drop live reload under load. The cost is that
  clients that stop reading can delay other HTML responses until they disconnect; with the
  default budget that takes eight stalled 8 MiB pages, or thousands of stalled small ones.
- **Streaming.** HTML over 8 MiB and every other file are streamed in 64 KiB chunks and are
  outside the budget.
- **HEAD.** A HEAD of an HTML page does the work of the GET with the same `Sec-Fetch-Dest`
  (read, check and, where the GET would inject, rewrite) under the budget, then sends those
  headers, the rewritten `Content-Length` included, with no body. It reports no warning.
- **Warnings.** The adapter defines a `WarningSink` port (`warn(code, message)`), re-exported
  by composition; `run` takes one and the CLI passes a sink that writes the ADR 0020 line to
  stderr. The adapter forwards each `(code, path)` pair once, remembering at most 4096 pairs;
  a message starts with the request path re-encoded from its validated components, never an
  absolute path or a query string. Nothing reaches an HTTP response.

## Consequences

- Peak memory for HTML buffers is the budget, independent of the number of connections.
- A HEAD of HTML reads the file, unlike any other HEAD.
- Lane work that builds an `AppState` passes `injection: Injection::default()` in tests.
- HTTP-08 adds `csp-blocks-live-reload` through the same sink and memo.
