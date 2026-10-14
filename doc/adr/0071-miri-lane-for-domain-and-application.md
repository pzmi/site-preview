# 0071. The Miri lane: domain and application tests, forwarded proptest settings, no compile-fail

Date: 2026-10-13

## Status

Accepted (owner, 2026-10-14)

## Context

ADR 0026 pins one dated nightly for `cargo-fuzz` and Miri and leaves the Miri lane to
TEST-09. Running `cargo miri nextest run -p preview-domain -p preview-application` on that
nightly showed three things that decide how the lane is built:

- Miri's isolation hides the host environment from the interpreted program. `PROPTEST_CASES`
  never reached proptest, which ran its default 256 cases per property under an interpreter
  two to three orders of magnitude slower than native code (two tests exceeded the 120 s
  nextest timeout), and the failure-persistence code, which calls `getcwd`, aborted the
  property tests with an unsupported-operation error.
- The `compile_fail` suites use `trybuild`, which starts `cargo` and `rustc` as child
  processes and reads the file system. Miri cannot run either.
- `cargo miri nextest` works with the pinned nextest (0.9.148), so the `cargo miri test`
  fallback is not needed.

## Decision

- `cargo xtask formal-verify --miri` (not in `check`; catalog row `formal-verify-miri`,
  `in_check = false`) runs `cargo +<nightly> miri nextest run --profile miri --locked --no-fail-fast -p
  preview-domain -p preview-application -E 'not binary(compile_fail)'`, the nightly read from
  `tools/versions.toml`.
- It sets `PROPTEST_CASES` (4 unless the caller sets it) and
  `PROPTEST_DISABLE_FAILURE_PERSISTENCE=true`, and appends `-Zmiri-env-forward=` for both to
  the caller's `MIRIFLAGS`, so the settings reach the interpreted program. Isolation stays on:
  `-Zmiri-disable-isolation` is not used, so the lane cannot touch the host.
- The only exclusion is the `compile_fail` binaries, recorded in `doc/verification-scope.md`.
  No test is `#[ignore]`d or `cfg`-gated for Miri, and no `#[allow]` is added. The ordinary
  `unit-tests` stage keeps running the excluded suites and replays the committed
  `proptest-regressions/` seeds.
- A missing Miri component or nextest fails with the install command, never a skip.
- `formal.yml` gains a `miri` job (schedule and manual dispatch, `contents: read`) that
  installs the component with `rustup +<nightly> component add miri rust-src`.

## Consequences

- Miri checks every unit and component test of the two crates for undefined behaviour at
  four cases per property. UB does not depend on the case count, but a rare input that only
  a long run generates is left to the fuzz targets and the 1024-case ordinary lane.
- Raising the case count is a measured change: the duration is recorded in
  `doc/verification-scope.md`.
- The adapters (file system, sockets, clock) stay outside the lane; adding one needs a
  decision about what Miri can run of it.
