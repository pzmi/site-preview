# 0032. The whole plan is integrated on main

Date: 2026-10-10

## Status

Accepted

## Context

ADR 0006 defines one DCO-signed commit per task, integrated onto `main` by the orchestrator. ADR 0027 proposed switching to branch protection and a pull-request flow at OPS-04. The owner decided that the whole plan stays on `main`.

## Decision

Every task in `doc/tasks.md` is one DCO-signed commit (`git commit -s`, subject `<ID>: <summary>`) that the orchestrator fast-forwards or cherry-picks onto `main` and pushes. No task switches to a pull-request flow or enables branch protection. A pull-request flow may be introduced only after the plan is complete, through a new ADR. CI may still run on `pull_request` events so outside contributors get checks.

This supersedes ADR 0027.

## Consequences

- No branch protection blocks pushes to `main` during the plan.
- Review checkpoints (the ⛳ tasks) and CI on `main` are the quality net.
- Red CI on `main` is fixed immediately on `main`, before further tasks build on it.
- Agents still never push, tag or change remote settings; only the orchestrator pushes.
