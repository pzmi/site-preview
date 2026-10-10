# 0019. `check` composes registered stages

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md wants one full gate, `cargo xtask check`, but gates are introduced across many tasks. A skipped stage must never look green.

## Decision

xtask keeps a catalog of stages. `check` runs every stage registered as part of it, and a stage that does not exist yet is simply absent (not "skipped"). The task that introduces a gate registers its stage. `quick` (fmt, clippy, architecture, fast tests) is explicitly not `check`. From OPS-01 on, every task must pass `check`.

## Consequences

- `list-checks` shows exactly what `check` covers at any point.
