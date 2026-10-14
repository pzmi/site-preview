# 0062. The `coverage` stage: what is measured, and the floors

Date: 2026-10-12

## Status

Accepted (owner, 2026-10-14). Amended by 0066: the overall floor counts
`crates/` only, and `xtask` is reported without a floor.

## Context

ADR 0018 sets the line-coverage floor at 90% with `coverage-floor.toml` as the ratchet, and
foundation.md section 5.3 asks that full coverage include every in-process suite. Two facts
shape the stage. `preview-conformance` is excluded from `unit-tests` and replays in the
`conformance` stage (ADR 0060), reading the traces `formal-generate` wrote when
`CONFORMANCE_GENERATED_MANIFEST` is set, and generating its own with the locked Quint when it
is not. And `tools/xtask` is a workspace member whose tests are part of the in-process suites.

## Decision

- **Measured:** `cargo llvm-cov nextest --workspace --locked --profile ci` with LCOV output
  at `target/coverage/lcov.info`: every workspace member, `preview-conformance` (replay and
  negative controls) and `tools/xtask` included. Files under `tests/` are left out by
  `cargo-llvm-cov`.
- **Not measured:** the Playwright suite and the `client-js` suite (they run outside the Rust
  instrumentation, and the browser suite is costly by design), and doctests (not
  instrumentable on stable). No `#[coverage(off)]` and no ignore-regex are used.
- **Conformance traces:** the stage removes `CONFORMANCE_GENERATED_MANIFEST`, so the
  conformance tests generate their own traces. The stage then needs `npm ci --prefix formal`
  and does not depend on `formal-generate` having run or on a stale
  `formal/traces/generated/`. The cost is one more generation of traces per `check`.
- **The measure** is the ratio of `DA:` records with a non-zero hit count, computed by xtask
  from the LCOV file in integer arithmetic (no rounding at the boundary). A crate is the
  directory under `crates/`, or `xtask`.
- **Floors** (`coverage-floor.toml`): `overall_line` and a `[crate]` table with
  `preview-domain` and `preview-application`. Each starts at the measured value rounded down,
  never below 90; the parser refuses a value outside 90..=100 and an unknown key, and a floor
  naming a crate absent from the report fails. A run below a floor prints the gap per crate
  and exits 1.
- `coverage` is registered in `check`, after `conformance` and before `security`.

## Consequences

- `check` gains the cost of one instrumented build and test run of the workspace.
- The overall figure includes `xtask`, whose tests probe installed tools, so it can differ a
  little between machines with and without the optional tools; the overall floor sits below
  the measured value for that reason.
- Raising a floor is a plain edit of `coverage-floor.toml`; lowering one needs owner review.
- TEST-07 reads the same LCOV file for the changed-line gate.
