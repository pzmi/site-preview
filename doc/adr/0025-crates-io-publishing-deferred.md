# 0025. crates.io publishing deferred

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

ADR 0007 makes only `preview-cli` publishable, but crates.io rejects a crate whose path dependencies are not published.

## Decision

v0.1.0 is distributed only as GitHub release archives. Publishing to crates.io is out of scope until a new ADR chooses an approach (publish internal crates under a prefix, or vendor them).

## Consequences

- `cargo install site-preview` does not work in P0; the README documents archive install.
