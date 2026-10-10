# 0014. Formal models retrofitted in P0, model-first afterwards

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md §6.2 asks for model-first development. ADR 0005 orders formal work after the CLI is complete.

## Decision

In P0 the Quint models (`lifecycle`, `watcher`, `reload`) are written after the vertical slice exists and are conformance-checked against it. From FORMAL-09 on, any protocol, state or concurrency change updates the model, scenarios and conformance observation first.

## Consequences

- Early domain state machines are designed with later modeling in mind (explicit inputs and observable state).
- `doc/formal-models.md` states the rule.
