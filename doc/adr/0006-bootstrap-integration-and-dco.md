# 0006. Bootstrap integration: one DCO-signed commit per task

Date: 2026-10-10

## Status

Accepted

## Context

Tasks are executed by fresh subagents and integrated by an orchestrator. Branch protection is premature before CI is stable.

## Decision

During bootstrap each task is exactly one DCO-signed commit (`git commit -s`) with subject `<ID>: <summary>`, integrated onto `main` by the orchestrator. Implementing agents never push, tag or change remote settings. The PR flow and branch protection start at the task defined in ADR 0027.

## Consequences

- History reads as one commit per task id.
- Only the orchestrator talks to GitHub.
