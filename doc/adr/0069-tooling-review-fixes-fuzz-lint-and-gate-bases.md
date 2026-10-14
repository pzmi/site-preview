# 0069. Fuzz targets formatted and linted, the changed-line base of a push, the formal lane's concurrency

Date: 2026-10-13

## Status

Accepted (owner, 2026-10-14). Amends 0060 (formal.yml concurrency) and
0064 (the changed-line base in CI and the freshness stamp). Amended by 0072.

## Context

The FORMAL-10 checkpoint review of the tooling found:

- The fuzz crate (ADR 0063) is outside the workspace, so `cargo fmt --all` and the workspace
  Clippy run never see `fuzz/fuzz_targets/*.rs`, and it did not inherit `[workspace.lints]`.
  One target was already unformatted.
- On a push to `main`, the changed-line gate (ADR 0064) compared with `HEAD~1`, so a push of
  several commits gated only the last one.
- `git ls-files --exclude-standard` honours `.git/info/exclude` and the user's global
  excludes file, so a personal ignore rule could hide an untracked source file from the
  changed-line gate and from the freshness fingerprint; and the freshness stamp did not record
  the toolchain or `RUSTFLAGS`, so a report measured under another build could be reused.
- `formal.yml` shared one concurrency group between the push/pull-request conformance job and
  the 120-minute scheduled Apalache job, so a push to `main` could wait behind a nightly run.

## Decision

- **`fuzz-fmt`**, a new in-process stage of `quick` and `check` right after `fmt-check`, runs
  `rustfmt --check --edition 2024` over `fuzz/fuzz_targets/*.rs` with the stable toolchain
  (rustfmt needs no nightly). `cargo xtask fmt` formats those files too.
- `fuzz/Cargo.toml` carries a copy of `[workspace.lints]` (it cannot inherit them); a xtask unit
  test fails when the two differ. `fuzz.yml` gains a `Clippy (fuzz targets)` job: Clippy with
  the pinned nightly, `-D warnings`.
- **The changed-line base.** `check`'s `changed-lines` stage reads `CHANGED_LINES_BASE`; when it
  is a full hexadecimal commit id other than all zeros (a new branch) and names a commit, the
  base is its merge-base with `HEAD`. Anything else is ignored with a note and the default of
  ADR 0064 applies. `ci.yml` sets it to `github.event.before` on a push, through `env:`, never
  interpolated into a script. A non-hexadecimal value is never passed to git.
- **Excludes.** Untracked files are listed with `--exclude-per-directory=.gitignore` only, so
  only the repository's committed ignore rules apply, the same on every machine.
- **The stamp** is the tree fingerprint followed by the build environment: `rustc -vV` of the
  toolchain the gate uses and the values of `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`,
  `CARGO_BUILD_RUSTFLAGS` and `RUSTUP_TOOLCHAIN`. A report is reused only when all of it matches.
- **formal.yml** keeps its push and pull-request trigger (foundation.md section 10 names
  `Conformance (Quint)` as a lane of its own, and ADR 0060 leaves the choice of required job to
  OPS-04), and its concurrency group now ends in `deep` for scheduled and dispatched runs and
  in `gate` otherwise, so the two never queue behind each other. Making formal.yml
  schedule-only was the alternative; it would remove the named lane before OPS-04 decides.

## Consequences

- `quick` and `check` fail on an unformatted fuzz target; the fix is `cargo xtask fmt`.
- A push of several commits is gated as a whole. A force push whose old head is no longer
  fetchable falls back to ADR 0064's default, with a note in the report.
- A personal ignore rule no longer changes what the gate measures; a file the repository
  should ignore needs a committed `.gitignore` rule.
- Changing the toolchain or `RUSTFLAGS` between `coverage` and `coverage --changed-since`
  measures again instead of reusing a report from another build.
- Pull requests still run the conformance stages twice (ADR 0060); the scheduled lanes no
  longer delay them.
