# 0006. Bootstrap integration: one DCO-signed commit per task

Date: 2026-10-10

## Status

Accepted

## Context

Tasks are executed by fresh subagents and integrated by an orchestrator. The owner integrates the whole plan directly on `main` (ADR 0032), so no branch protection blocks those pushes.

## Decision

For the whole plan each task is exactly one DCO-signed commit (`git commit -s`) with subject `<ID>: <summary>`, integrated onto `main` by the orchestrator. Implementing agents never push, tag or change remote settings. No task in the plan switches to a pull-request flow or enables branch protection (ADR 0032, which supersedes ADR 0027).

## Consequences

- History reads as one commit per task id.
- Only the orchestrator talks to GitHub.
