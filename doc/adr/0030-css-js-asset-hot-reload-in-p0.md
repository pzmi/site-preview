# 0030. CSS, JS and asset hot reload in P0

Date: 2026-10-10

## Status

Accepted

## Context

plan.md §7.3 says P0 always does a full-page reload, and §14 defers CSS-only refresh. The owner wants a faster edit loop for styles, scripts and assets.

## Decision

Hot reload is P0. A debounced revision carries the set of changed eligible paths, and the SSE `reload` event becomes `{instance_id, revision, changes:[{path, kind}], full}`. Kinds are css, js, html, asset and other. CSS is swapped in place with a cache-busting query, waiting for load before removing the old node (`@import` best-effort); no match means a full reload. JS reloads the page and restores the scroll position, unless the page opted in through `window.__sitePreview.accept(path, handler)`. Images and CSS `url()` assets are cache-busted, and no match means a full reload. HTML, overflow and unknown changes do a full reload. The render-time baseline and instance logic of plan.md §7.3 is unchanged.

## Consequences

- Supersedes plan.md §7.3 "always full reload" and §14 item 1.
- Details are in ADR 0031.
- Tasks: DOM-10, HTTP-12..16, E2E-09; the reload model covers it.
