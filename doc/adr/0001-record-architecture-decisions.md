# 0001. Record architecture decisions

Date: 2026-10-10

## Status

Accepted

## Context

Two specifications (`doc/plan.md`, product behavior; `doc/foundation.md`, engineering foundation) conflict in places and leave choices open. Agents with small contexts need a short, citable record of what was decided and why.

## Decision

Record each architectural decision as a lightweight ADR in `doc/adr/NNNN-kebab-title.md` (Michael Nygard format: Status, Context, Decision, Consequences, at most about 30 lines). Index every ADR in `doc/adr/README.md`. A change that adds or reverses a decision adds a new ADR or supersedes an old one in the same commit. Owner decisions are Accepted. Planner defaults are Proposed until the owner accepts or overrides them.

## Consequences

- `doc/tasks.md` cites decisions as D*n*, which is ADR *n*+1.
- DOC-05 checks that the index and the files agree.
- Agents never flip Proposed to Accepted.
