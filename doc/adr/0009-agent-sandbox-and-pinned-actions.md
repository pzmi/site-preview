# 0009. Agent sandbox assumptions and SHA-pinned actions

Date: 2026-10-10

## Status

Accepted

## Context

Agents run in a sandbox without Docker. Chromium for Playwright is preinstalled at `/opt/pw-browsers`. Node is available. Unpinned actions are a supply-chain risk.

## Decision

No task requires Docker. Never run `playwright install`; use `PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`. Node is a dev-only dependency (Quint, Playwright, `node --test`), never a runtime one. GitHub Actions are pinned by full commit SHA with a version comment.

## Consequences

- The Playwright version is pinned to match the preinstalled Chromium build (ADR 0016).
- SEC-02 checks action pinning.
