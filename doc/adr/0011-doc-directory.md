# 0011. Use doc/ instead of docs/

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md §2 places documentation under `docs/`. The repository already uses `doc/` for plan.md, foundation.md and tasks.md.

## Decision

All documentation lives under `doc/`: `doc/architecture.md`, `doc/architecture-rules.md`, `doc/testing.md`, `doc/security.md`, `doc/formal-models.md`, `doc/sdlc.md`, `doc/runbook.md` and `doc/adr/`. `SECURITY.md`, `CONTRIBUTING.md`, `CHANGELOG.md` and `AGENTS.md` stay at the root.

## Consequences

- foundation.md paths under `docs/` are read as `doc/`.
