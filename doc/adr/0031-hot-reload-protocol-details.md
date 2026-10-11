# 0031. Hot-reload protocol details

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

ADR 0030 leaves classification, caps, the coalescing gap, scroll persistence and iframes open. The latest-state channel coalesces revisions, so a slow client can miss an intermediate change set.

## Decision

Classify kinds by extension (`.css`; `.js`/`.mjs`; `.html`/`.htm`; images, fonts and media as asset; everything else other). Set `full = true` when: there are more than 64 changes, an overflow or rescan occurred, a directory changed, any html or other path changed, a css, js or asset path was removed, or the connection's previously sent revision is not `revision - 1` (never hot-apply across a gap). Carry the scroll position across a JS reload in `history.state.__sitePreview` (merged only into object or null state, removed after restore); no cookies, `localStorage`, `sessionStorage` or URL changes. Form state is not restored in P0. Same-origin iframes are documents with their own injected client.

A change's `path` is root-relative and percent-encoded per component (the WHATWG path percent-encode set plus `%`, non-ASCII as UTF-8), and decodes back to the same components through the server's request-path decoder; clients compare decoded paths. A rename is not a removal: editors that save atomically rename a temporary file onto the target, which must stay hot; when the old side of a rename leaves a page referencing a missing file, the in-place load fails and the client turns that into a full reload. A directory change carries no path of its own. A full change set keeps up to 64 paths for information only.

## Consequences

- A missed change always degrades to a full reload, never to a stale page.
- Sites that store non-object `history.state` lose scroll restore only.
