# 0029. Performance targets are measured, not gating

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

plan.md §13 sets targets. foundation.md §12 phase 5 says budgets become blocking only after a stable procedure and an approved budget.

## Decision

`cargo xtask bench` measures startup, save-to-reload latency, idle CPU and a soak, and records results in `doc/performance.md`. It is not part of `check` until the owner approves a budget.

## Consequences

- Regressions are visible but do not block merges in P0.
