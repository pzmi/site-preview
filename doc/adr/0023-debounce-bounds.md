# 0023. Bounds for --debounce

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

plan.md §3 asks to reject zero or nonsensically large debounce durations.

## Decision

Accept durations with `ms` or `s` units in the range 10 ms to 10 s inclusive. Anything else is a usage error (exit 2) with the allowed range in the message.

## Consequences

- The domain `DebounceWindow` type enforces the range.
