# 0022. HTML charset labeling and injection eligibility

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

plan.md §5.1 and §6 forbid mislabeling non-UTF-8 HTML and forbid corrupting HTML to achieve live reload.

## Decision

Label HTML `text/html; charset=utf-8` and make it eligible for injection only when the bytes are valid UTF-8 and any `<meta charset>` agrees. Otherwise serve unchanged as `text/html` and warn once per path (`html-not-utf8`).

## Consequences

- Legacy-encoded pages preview without live reload.
