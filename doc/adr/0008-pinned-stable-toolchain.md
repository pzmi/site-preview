# 0008. Pinned stable toolchain, edition 2024, committed lockfile

Date: 2026-10-10

## Status

Accepted

## Context

Reproducible builds and agent sandboxes need one known compiler. The sandbox has rustc 1.97.0.

## Decision

Pin `1.97.0` exactly in `rust-toolchain.toml` with `rustfmt`, `clippy` and `llvm-tools-preview`. Use edition 2024 and resolver 3. Commit `Cargo.lock`. CI and release use `--locked`.

## Consequences

- Toolchain bumps are their own reviewed change.
- Nightly is used only for fuzz and Miri (ADR 0026).
