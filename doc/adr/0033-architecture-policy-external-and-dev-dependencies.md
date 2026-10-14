# 0033. Architecture policy: external allowlist shape and dev-dependencies

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

ADR 0013 adds an `[external]` table to `architecture.toml` listing the third-party crates, and optionally the features, each crate may use. FND-01 left it as a flat list (`crate = ["dep"]`) with no dependency kind, so dev-only crates (`tempfile`, `assert_cmd`, `predicates`) were either listed as if they were normal dependencies or not listed at all. foundation.md §3.2 rule 3 asks the checker to distinguish normal, build and dev dependencies.

## Decision

- `[external]` has three sub-tables, `[external.normal]`, `[external.dev]` and `[external.build]`, each mapping a workspace crate to `{ <third-party crate> = <features> }`.
- A dev-dependency is listed under `[external.dev]`. A crate already allowed under `[external.normal]` for the same workspace crate is also allowed as its dev-dependency. A build-dependency is allowed only under `[external.build]`.
- `<features>` is either a list of feature names, where `"default"` stands for the dependency's default features, or `"*"` for any feature. Production crates use lists.
- An edge's features are what that crate asks of the dependency: its declared features, `"default"` unless default features are off, and `dep/feature` entries of its own active features. Both that set and the allowed list are expanded through the dependency's feature table before comparing, so listing `"full"` allows what `full` implies. Features another crate enables on the same package do not count against this crate.
- Every package in the resolved graph comes from crates.io, unless `[exceptions.sources]` maps its source without query or revision (for example `"git+https://host/repo" = { adr = "NNNN" }`) to an ADR that exists in `doc/adr/`. A path dependency outside the workspace is always rejected.
- Edges are evaluated per target with `cargo metadata --filter-platform` for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (ADR 0003: not Windows). A violation present on only some targets names them.
- In each `[workspace]` category, a list that is present must not be empty, and each crate is in exactly one category.
- `[exceptions]` holds only the kinds a task defines (`sources` from ARCH-03); unknown keys fail.
- The checker runs `cargo metadata --locked`. A negative fixture workspace commits its own `Cargo.lock`, and it stubs any third-party crate as a local vendored source, so fixtures resolve without network access.

## Consequences

- ARCH-01 parses this schema strictly. ARCH-03 enforces kinds, features and sources against resolved metadata.
- Adding a dev-dependency edits `[external.dev]` in the same commit, like any other dependency.
