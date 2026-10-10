# 0027. When the PR flow starts

Date: 2026-10-10

## Status

Superseded by [0032](0032-whole-plan-on-main.md)

## Context

ADR 0006 defers branch protection until CI is stable.

## Decision

Branch protection and the PR flow start at OPS-04, once the Linux+macOS matrix (OPS-03) is green on `main` twice in a row. From then on, every task is a PR gated by the `required` check, and commits remain DCO-signed.

## Consequences

- The owner or orchestrator applies the settings; agents never do.
