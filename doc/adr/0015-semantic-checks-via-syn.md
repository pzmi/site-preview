# 0015. Semantic architecture checks via a qualified tool or syn

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

foundation.md §3.3 suggests trialling cargo-bylaw but warns it is young. Text grep misses aliases, macros and re-exports.

## Decision

ARCH-05 qualifies cargo-bylaw against macro-generated imports, re-export aliases, `cfg` branches, trait impls and generated code. If it does not qualify, implement the semantic rules in xtask by parsing sources with `syn` and resolving `use` aliases. A parse failure or unsupported construct is a failure, never a pass.

## Consequences

- Each semantic rule has a negative fixture (ARCH-06, ARCH-07).
