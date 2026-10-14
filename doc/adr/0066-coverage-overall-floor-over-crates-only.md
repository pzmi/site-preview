# 0066. The overall coverage floor over `crates/` only; `xtask` report only

Date: 2026-10-13

## Status

Accepted (owner, 2026-10-14). Amends ADR 0062.

## Context

ADR 0062 floors `overall_line` over every instrumented line, `tools/xtask` included. Two facts
make that figure unstable. `xtask` covers about 84% of its own lines, so it drags the overall
figure down: `main` measured 92.20% against a floor of 92. And its tool-probe tests take
different branches depending on which tools are installed (java, cargo-fuzz, the nightly
toolchain); the CI runner lacks several that a developer machine has, so CI can measure below
the floor while a local run passes. A separate floor for `xtask` below 90 would break ADR
0018's minimum, and one at 90 would fail today.

## Decision

- `overall_line` is computed over the files under `crates/` only: the product crates and the
  test-support crates (`preview-testkit`, `preview-conformance`). Files outside `crates/` and
  `tools/xtask/` count nowhere.
- `tools/xtask` is still measured, and printed on its own row as `report only`. It has no floor;
  `coverage-floor.toml` refuses an `xtask` key.
- The crates-only floor is calibrated once at the measured value rounded down, measured with
  java, cargo-fuzz, cargo-mutants and the nightly toolchain hidden, as on the CI runner:
  95.61%, so `overall_line = 95`. This sets a floor for a new metric; it lowers nothing.
  Per-crate floors are unchanged.
- Raising a floor stays a plain edit; lowering one still needs owner review (ADR 0018).

## Consequences

- The floored figure depends on product and test-support code only, so it no longer moves
  with the tools a machine has, and it sits higher than the old workspace-wide one.
- A regression in `xtask`'s own tests is visible in the report but fails no gate; the
  changed-line gate already left `tools/xtask` out (ADR 0064).
- CI installs `cargo-llvm-cov` with the other pinned tools, and its cache key names the tool set.
