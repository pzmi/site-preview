# 0018. Coverage gates and changed-line computation

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md §5.3 proposes 90% overall coverage and 95% changed-line coverage via `diff-cover` (a Python tool).

## Decision

The overall line-coverage floor starts at 90% and is ratcheted in `coverage-floor.toml` (domain and application floors start at their measured value, minimum 90). The changed-line gate (≥95% of instrumented changed lines) is computed in Rust by xtask from LCOV plus `git diff -U0`.

## Consequences

- No Python dependency.
- Lowering a floor needs owner review.
