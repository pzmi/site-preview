# 0074. Native platform matrix: a `platform` job on four runners, one extra non-gate stage

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Extends 0072.

## Context

The product's riskiest code is the operating system's: inotify and FSEvents, `openat` with
`O_NOFOLLOW` and its errnos, signals, canonical temporary roots. Foundation.md section 10 asks
for platform-specific integration and binary tests on their native OS; decision D2 rules out
Windows. ADR 0072 left every job on `ubuntu-24.04`.

`cargo xtask check --only integration-tests` is not usable on a macOS or arm64 Linux runner: it
includes the `xtask` package's security fixture tests, which run the real cargo-deny, Gitleaks
and actionlint, and `install-scanners` fetches the linux x86-64 builds only.

## Decision

- **A `platform` job** in `ci.yml` runs on a matrix of `ubuntu-24.04`, `ubuntu-24.04-arm`,
  `macos-15` (arm64) and `macos-15-intel`, `fail-fast: false`. It installs `cargo-nextest` and
  runs `cargo xtask test --only platform-tests`. `required` needs it, so the shape test
  (`should_need_every_other_job_when_required_job_is_read`) keeps it in the gate.
- **`platform-tests`** is the nextest `ci` run of the whole workspace (unit and integration
  tests: adapter-fs, the OS watcher, the symlink race, the CLI black-box suites) minus
  `preview-conformance` (Linux canonical, ADR 0060), minus the `xtask` package (tooling, whose
  fixtures need the Linux scanners) and minus the `compile_fail` binaries (rustc diagnostics).
  It is a catalog row with `in_check = false` and is not part of `check`, because on Linux the
  `unit-tests` and `integration-tests` stages already contain it; `check` and the
  one-stage-one-job test are unchanged. Gitleaks and actionlint stay out of this job.
- **Caches** add `runner.arch` beside `runner.os` to the key and its fallback prefix, and the
  job namespace carries the matrix runner, so no platform restores another's entry. The cached
  paths (`~/.cargo/...`, `target`) are the same on macOS.
- Failure evidence: the JUnit report, named by runner.

## Consequences

- A macOS or arm64 regression turns `required` red. Behaviour that cannot hold on macOS is
  either fixed in the adapter with a test, gated with `cfg(target_os)` and a reason, or recorded
  in `doc/runbook.md`; `#[ignore]` stays forbidden.
- Linux x86-64 runs the product tests three times (fast, integration, platform); the price of a
  uniform four-way matrix is accepted over a separate arm-only stage.
- OPS-04 requires two consecutive green runs on `main`.
