# 0002. Multi-crate workspace supersedes the single-crate layout

Date: 2026-10-10

## Status

Accepted

## Context

plan.md §4 proposes a single binary crate. foundation.md §2 proposes a workspace with domain, application, adapter, composition, CLI, testkit and conformance crates and an enforced dependency law.

## Decision

Use the foundation.md §2 workspace as the code structure. plan.md §4's layout is superseded. plan.md stays authoritative for product behavior: the HTTP contract, security rules, SSE protocol, watcher semantics and CLI.

## Consequences

- plan.md module names map onto crates (paths→domain+adapter-fs, serve→adapter-http, reload→application+adapter-http, watcher→adapter-fs+composition, main→cli+composition).
- The architecture checker enforces the boundaries.
