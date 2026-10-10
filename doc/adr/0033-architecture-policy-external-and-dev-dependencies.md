# 0033. Architecture policy: external allowlist shape and dev-dependencies

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

ADR 0013 adds an `[external]` table to `architecture.toml` listing the third-party crates, and optionally the features, each crate may use. FND-01 left it as a flat list (`crate = ["dep"]`) with no dependency kind, so dev-only crates (`tempfile`, `assert_cmd`, `predicates`) were either listed as if they were normal dependencies or not listed at all. foundation.md §3.2 rule 3 asks the checker to distinguish normal, build and dev dependencies.

## Decision

- `[external]` has three sub-tables, `[external.normal]`, `[external.dev]` and `[external.build]`, each mapping a workspace crate to `{ <third-party crate> = <features> }`.
- A dev-dependency is listed under `[external.dev]`. A crate already allowed under `[external.normal]` for the same workspace crate is also allowed as its dev-dependency. A build-dependency is allowed only under `[external.build]`.
- `<features>` is either a list of feature names, where `"default"` stands for the dependency's default features, or `"*"` for any feature. Production crates use lists.
- In each `[workspace]` category, a list that is present must not be empty, and each crate is in exactly one category.
- `[exceptions]` stays empty until a task defines an exception kind; unknown keys fail.
- The checker runs `cargo metadata --locked`. A negative fixture workspace commits its own `Cargo.lock`, and it stubs any third-party crate as a local vendored source, so fixtures resolve without network access.

## Consequences

- ARCH-01 parses this schema strictly. ARCH-03 enforces kinds, features and sources against resolved metadata.
- Adding a dev-dependency edits `[external.dev]` in the same commit, like any other dependency.
