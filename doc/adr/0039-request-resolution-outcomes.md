# 0039. Request resolution outcomes: no existence leak, snapshot before open

Date: 2026-10-11

## Status

Proposed (planner default — owner may override)

## Context

APP-03 turns a raw request path into what to serve. plan.md §5.1 allows 404 or 403 for
refused paths and §5.2 rule 5 denies directories such as `node_modules` and `vendor`, but
`eligibility::path` checks a final component without a trailing slash as a file (ADR 0012).
Whether `/vendor` is a directory is known only after the open, so a redirect, a 403 or an
I/O error at that point would reveal that the denied directory exists. The `SiteContent`
port (ADR 0038) has an `Io(kind)` refusal that the task's result list does not name, and the
reload channel the snapshot reads from (APP-02) was built in parallel.

## Decision

- Order: decode, eligibility, snapshot, open (plan.md §7.3). A malformed or ineligible path
  is never opened and the snapshot is not taken for it.
- `PathRefusal::Malformed` is `BadRequest`; `PathRefusal::Traversal` (`..`, encoded
  separators, `a//b`) is `NotFound`, since such a path names nothing that can exist.
- An ineligible path is `Forbidden(Ineligible(reason))`: decided from the text alone, it
  reveals nothing about the disk.
- When the final name, requested without a trailing slash, would be denied as a directory,
  every outcome but a plain file is `NotFound`: a directory (with or without index), a
  refused or failing index, and any open refusal. The response then equals that of a
  missing entry.
- Otherwise a content refusal `Forbidden(reason)` is `Forbidden(Content(reason))`,
  `NotFound` is `NotFound`, and `Io(kind)` is a new outcome `Unavailable(kind)`, which the
  HTTP adapter answers with a fixed 500 and no detail.
- A file requested with a trailing slash is `NotFound`. `RedirectToSlash` carries the decoded
  `SitePath`, so the `Location` is encoded from validated components.
- The snapshot is a parameter `impl FnOnce() -> SiteState`, called at most once, so the
  query needs no runtime; the HTTP adapter passes `|| reload.current()` over the shared `ReloadState` (APP-02).

## Consequences

- HTTP-01 maps `BadRequest` to 400, `NotFound` to 404, `Forbidden` to 403, `Unavailable` to
  500, all with fixed bodies. HTTP-04 builds `Location` from `RedirectToSlash.path`.
- A directory denied only as a directory may be opened (its index handle dropped unread)
  before it is recognised; its bytes are never served.
