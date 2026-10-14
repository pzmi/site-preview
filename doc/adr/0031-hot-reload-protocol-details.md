# 0031. Hot-reload protocol details

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

ADR 0030 leaves classification, caps, the coalescing gap, scroll persistence and iframes open. The latest-state channel coalesces revisions, so a slow client can miss an intermediate change set.

## Decision

Classify kinds by extension (`.css`; `.js`/`.mjs`; `.html`/`.htm`; images, fonts and media as asset; everything else other). Set `full = true` when: there are more than 64 changes, an overflow or rescan occurred, a directory changed, any html or other path changed, a css, js or asset path was removed, or the connection's previously sent revision is not `revision - 1` (never hot-apply across a gap). Carry the scroll position across a JS reload in `history.state.__sitePreview` (merged only into object or null state, removed after restore); no cookies, `localStorage`, `sessionStorage` or URL changes. Form state is not restored in P0. Same-origin iframes are documents with their own injected client.

A change's `path` is root-relative and percent-encoded per component (the WHATWG path percent-encode set plus `%`, non-ASCII as UTF-8), and decodes back to the same components through the server's request-path decoder; clients compare decoded paths. A rename is not a removal: editors that save atomically rename a temporary file onto the target, which must stay hot; when the old side of a rename leaves a page referencing a missing file, the in-place load fails and the client turns that into a full reload. A directory change carries no path of its own. A full change set keeps up to 64 paths for information only.

## Consequences

- A missed change always degrades to a full reload, never to a stale page.
- Sites that store non-object `history.state` lose scroll restore only.

## Amendments

- **2026-10-11 (DOM-10, recorded here after the fact).** The second paragraph of the
  Decision was added in place by DOM-10 rather than as a dated amendment: the per-component
  percent-encoding of `path` and its round trip through the request-path decoder, the rule
  that a rename is not a removal, and that a full change set keeps up to 64 paths for
  information only. The decision itself is unchanged.
- **2026-10-11 (APP-04 checkpoint review).** `Create`, `Remove` and `Rename` events do not
  say whether the entry was a file or a directory, so classification checks their last
  component both ways. A name denied as a file is ignored, as before. A name denied only as
  a directory (`vendor`, `node_modules`, `__pycache__` reported as created, removed or
  renamed) makes the change set full (reason `Undisclosed`) without adding its path, so the
  name of an excluded directory never reaches a client (ADR 0039). A name eligible both ways
  is recorded as a file and also requests a watch-set reconcile, because it may be a
  directory that appeared, vanished or was moved in. `Modify` is checked as a file and
  `DirectoryChanged` as a directory, as before. The change-set API classifies events itself
  (`ChangeSet::accumulate(&event)`, `Debouncer::record(now, &event)`), so a caller cannot
  pair an ineligible path with a relevant classification.
