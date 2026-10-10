# 0016. Where each test level lives

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md proposes a root `tests/e2e/`, but the workspace root is a virtual manifest. Browser automation needs a runner. The P0 browser client is JavaScript.

## Decision

Rust black-box tests (`assert_cmd`) live in `crates/preview-cli/tests/`. Browser tests live in `tests/e2e/browser/` with Node `@playwright/test`, pinned (lockfile) to the release that matches preinstalled Chromium 1194 (1.56.x). `client.js` is unit-tested with `node --test` and no dependencies. The shared fixture site is `tests/fixtures/site/`. Architecture fixtures live in `tests/architecture-invalid/` and `tests/architecture-valid/`.

## Consequences

- Node stays dev-only.
- A Playwright upgrade must match the preinstalled browser build.
