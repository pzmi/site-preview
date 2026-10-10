# 0005. Proportionate foundation with an early vertical slice

Date: 2026-10-10

## Status

Accepted

## Context

foundation.md front-loads enforcement and quality machinery. plan.md wants a small working tool quickly. A milestone made only of scaffolding gives later gates nothing real to exercise.

## Decision

Order: scaffold → architecture checker → domain/application → serving, with a thin working `serve` slice as early as possible (CLI-01) → injection+SSE → watcher → CLI/lifecycle/JSON → formal models and conformance → hardening (fuzz, mutation, coverage) → release.

## Consequences

- Gates exercise real code from the second milestone on.
- Formal models are retrofitted (see ADR 0014).
