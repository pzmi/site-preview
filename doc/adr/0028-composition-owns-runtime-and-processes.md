# 0028. Composition owns runtime, signals and process spawning

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

foundation.md says the CLI delegates through composition. Signal handling and opening a browser are runtime or process concerns.

## Decision

`preview-composition` builds the Tokio runtime, installs SIGINT/SIGTERM handlers, owns cancellation and launches the browser opener (`open` or `xdg-open`, argument vector, no shell). `preview-cli` parses flags, renders human or JSON output and maps errors to exit codes.

## Consequences

- `preview-cli` has no tokio dependency (ADR 0013).
