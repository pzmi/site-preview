# 0026. Nightly scope, fuzz crate placement, concurrency proofs

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

cargo-fuzz and Miri need nightly. foundation.md lists Loom, Shuttle and Kani for complexity that is introduced.

## Decision

Pin one dated nightly for fuzz and Miri only. `fuzz/` is a separate cargo-fuzz workspace listed as `fuzz_only` in `architecture.toml`. Loom, Shuttle and Kani are not adopted in P0, because the code uses only tokio `watch` and `Semaphore`. Adopt them when a custom synchronization primitive appears.

## Consequences

- `doc/verification-scope.md` records what verifies which property.
