# 0064. The changed-line gate: base, report freshness, and its place in `check`

Date: 2026-10-13

## Status

Accepted (owner, 2026-10-14). Amended by 0069.

## Context

ADR 0018 computes the ≥95% changed-line gate (foundation.md section 5.3) in xtask from LCOV
plus `git diff -U0`, and TEST-07 names the default base: `origin/main`, falling back to
`HEAD~1`. Three questions are left open. `main` receives direct commits with no pull requests,
so `origin/main` is often `HEAD` itself and a diff against it is empty. The coverage stage
(ADR 0062) writes `target/coverage/lcov.info`, and joining a diff with a report of another
`HEAD`, or of other uncommitted edits, would gate the wrong lines. And the task does not say
whether the gate joins `check`, which foundation.md section 12 phase 2 needs for "an untested
changed line affects the changed-code gate".

## Decision

- **Base:** the merge-base of `REV` and `HEAD`, so commits that reached `REV` after the branch
  left it are never counted as this change. With no `REV`, the merge-base with `origin/main`
  when it is not `HEAD`; otherwise (no `origin/main`, or it already contains `HEAD`, as on a
  push to `main`) `HEAD~1`. No base at all (one commit, or a shallow clone) is an error naming
  `fetch-depth: 0`, never a pass.
- **The change** is the diff from the base to the working tree, plus every untracked,
  non-ignored file under `crates/`, so uncommitted work is gated before its commit. Renames
  are detected; only `crates/**/src/**/*.rs` counts (`tools/xtask` and `tests/` do not).
- **Freshness:** a successful coverage measurement writes `target/coverage/lcov.stamp`, a git
  hash of the path and blob id of every tracked and untracked, non-ignored file, taken before
  the run. It depends on content, not on `HEAD`: committing the measured tree keeps the report
  fresh, and any edit, new file or deletion makes it stale. The report is used only while the
  stamp equals the tree's fingerprint now; `cargo xtask coverage --changed-since [REV]`
  measures again when it does not. A diff is never joined with a report of other content.
- **In `check`:** a `changed-lines` stage right after `coverage`, with the default base. It
  reads the report `coverage` just wrote and never measures: a stale or missing report (the
  coverage stage failed, or the tree changed during the gate) fails the stage with the fix.
  A change with no instrumented changed line passes and says so.

## Consequences

- An untested line in a crate fails `check` locally before commit and in CI after the push
  (CI checks out with `fetch-depth: 0`, so `HEAD~1` exists, and installs `cargo-llvm-cov` with
  the other pinned tools). On a runner without `cargo-llvm-cov`, `coverage` is `UNAVAILABLE` and
  `changed-lines` fails on the missing report: the job is red, never green without the gate.
- When `origin/main` equals `HEAD`, `check` gates the last commit together with any
  uncommitted edits, a superset of the change; a lane branched from an unpushed local `main`
  also carries those commits until `main` is pushed.
- The gate is as deterministic as the coverage run: no network, no timing; the only
  machine-dependent lines (xtask's tool probes) are outside its scope.
- No cost beyond a few git commands in `check`; the command alone costs a measurement only
  when the tree changed since the last one.
