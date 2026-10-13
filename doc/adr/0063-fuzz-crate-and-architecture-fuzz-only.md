# 0063. The fuzz crate: a separate workspace, a `fuzz_only` category and the `xtask fuzz` command

Date: 2026-10-12

## Status

Proposed (planner default — owner may override). Amended by ADR 0065: the fuzz crate may also
depend on `preview-application` and `preview-adapter-http`, and `fuzz/Cargo.lock` is audited by
the `deny-fuzz` stage.

## Context

ADR 0026 pins one dated nightly for fuzzing and Miri and places `fuzz/` outside the workspace
as `fuzz_only` in `architecture.toml`. SEC-04 builds it. cargo-fuzz needs a nightly compiler
and sanitizer flags that must never reach the production build, and its dependencies
(`libfuzzer-sys`, `arbitrary`, a C++ runtime build) are not part of the product's supply chain.
The architecture checker (ADR 0033) only knew workspace members, so a fuzz crate outside the
workspace was invisible to it: nothing stopped a second, unlisted fuzz crate, or a fuzz crate
that quietly reached a crate it should not.

## Decision

- `fuzz/` is a cargo-fuzz crate named `preview-fuzz`. It is not a workspace member: its
  manifest holds its own `[workspace]` table and it has its own `Cargo.lock`, so the root
  workspace, `cargo xtask check`, `cargo deny` and the stable toolchain never build or see it.
  `fuzz/rust-toolchain.toml` pins the nightly of `tools/versions.toml` (`nightly-2026-10-01`);
  a unit test requires the two to agree.
- `architecture.toml` gets a fourth category, `[workspace] fuzz_only`. The rule `ARCH-fuzz-crate`
  checks, from the manifest at `fuzz/Cargo.toml`: the package is listed; every listed name is
  that package; it is not a workspace member (a path dependency from a member would make it
  one); it has its own `[workspace]` table; each path dependency is a workspace crate listed
  in `[dependencies.normal] <fuzz crate>`; each other dependency is in `[external.normal]
  <fuzz crate>`. Features are not checked, because the fuzz crate has no `cargo metadata` view
  in the root workspace. Negative fixtures: `unlisted-fuzz-crate`,
  `fuzz-crate-without-own-workspace`, `fuzz-depends-on-unlisted-crate`.
- `cargo xtask fuzz <target> [--time N] [--reproduce <artifact>]` requires `cargo-fuzz` and the
  nightly (ADR 0010: a missing one fails with its install command), accepts only a name that
  is a file in `fuzz/fuzz_targets/`, and runs `cargo +<nightly> fuzz run`. A run reads the
  committed seeds `fuzz/seeds/<target>/` and writes `fuzz/corpus/<target>/` (ignored); a crash
  artifact lands in `fuzz/artifacts/<target>/` (ignored). `fuzz` is not part of `check`: it is
  bounded by time, needs the nightly, and belongs to the scheduled lane (SEC-05).
- The fuzz crate may depend on the domain only. Its targets call the real domain functions.

## Consequences

- The fuzz crate's dependencies are not covered by the root `deny.toml` run; SEC-05 decides
  how the scheduled lane audits `fuzz/Cargo.lock`.
- Semantic source rules (ARCH-06, ARCH-07) walk workspace members only, so the fuzz target
  source is checked by review and by the nightly's own compiler, not by xtask.
- A new fuzz target is a file in `fuzz/fuzz_targets/`, a `[[bin]]` entry and a seed directory;
  a unit test fails when a target has no seeds.
