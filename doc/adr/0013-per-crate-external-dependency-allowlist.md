# 0013. Per-crate external dependency allowlist

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md §3.2 polices workspace edges. It does not say how third-party crates per layer are controlled. The latest-state channel (`tokio::sync::watch`) is needed by both the watcher side and the SSE side, and adapters may not depend on each other.

## Decision

`architecture.toml` gains an `[external]` table listing the third-party crates (and, optionally, features) each crate may use directly. `preview-domain` may use only `percent-encoding`. `preview-application` may use `tokio` with feature `sync` only. `preview-cli` uses no `tokio`; composition owns the runtime (ADR 0028).

## Consequences

- ARCH-03 enforces crates and resolved features.
- Adding a dependency edits the allowlist in the same commit.
