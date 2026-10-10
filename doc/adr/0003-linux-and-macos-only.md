# 0003. P0 platforms are Linux and macOS only

Date: 2026-10-10

## Status

Accepted

## Context

plan.md targets Linux and macOS, and its secure-open design is POSIX `openat`. foundation.md mentions a Windows CI matrix.

## Decision

P0 supports Linux (x86_64, aarch64; musl release builds) and macOS (arm64, x86_64). Windows CI, Windows target checks and Windows release archives are deferred.

## Consequences

- No task adds Windows CI.
- The architecture checker evaluates Linux and macOS targets only and says so in its output.
- Revisit with a new ADR if Windows users appear.
