# 0072. CI job topology: one job per level, `check --only` parity, restore-only PR caches

Date: 2026-10-13

## Status

Accepted (owner, 2026-10-14). Amends 0060 (`formal.yml` no longer runs on
push or pull request) and 0069 (which kept it there until OPS-04 decided).

## Context

`ci.yml` was one `Check` job running `cargo xtask check`: about forty minutes of serial work, one
red light for every cause, and no artifacts a failed run could be diagnosed from. Foundation.md
section 10 names the blocking Linux jobs; section 9 asks for local/CI parity and caches keyed by
lockfile and toolchain. `formal.yml` also ran the conformance stages on every push and pull
request (ADR 0060, 0069), so they ran twice. `security.yml` ran the security stages on every pull
request while `check` ran them again.

The workspace nextest run was a single `unit-tests` stage, so there was nothing for an
`integration` job to call.

## Decision

- **`cargo xtask check --only <stage>[,<stage>...]`** (repeatable) runs the named catalog stages
  through the same stage list as the full `check`, in the gate's order (so `coverage` precedes
  `changed-lines`), with the same table and exit codes. An unknown name exits 2 and lists the
  stage names; no `--only` is the full gate.
- **Jobs of `ci.yml`**, each calling `check --only` and installing only the tools it needs:

  | Job | Stages |
  |:--|:--|
  | `architecture` | `architecture` |
  | `static` | `fmt-check`, `fuzz-fmt`, `clippy`, `docs` |
  | `fast-tests` | `unit-tests`, `doctests`, `client-js` |
  | `integration` | `integration-tests` |
  | `conformance` (`Conformance (Quint)`) | the seven conformance stages |
  | `coverage` | `coverage`, `changed-lines` |
  | `security` | `deny`, `deny-fuzz`, `secrets`, `workflows`, `action-pins` |
  | `browser` | `browser` |
  | `artifact-smoke` | none (`cargo xtask release-check`, OPS-05, ADR 0076) |
  | `required` | `needs:` every job above and fails unless each result is `success` (a cancelled or skipped job fails it) |

  `browser` stays one stage with its Chromium install step in the job. A unit test
  (`should_cover_each_in_check_stage_once_when_ci_jobs_are_read`) fails when an `in_check`
  catalog stage is in no job or in two, and another when `required` does not need every job.
- **`unit-tests` is split by test kind** into `unit-tests` (`-E 'not kind(test)'`: tests compiled
  into libraries and binaries) and a new `integration-tests` stage (`-E 'kind(test)'`: every
  `tests/*.rs` binary, including the black-box, real-filesystem and compile-fail ones). Both keep
  the `ci` profile, the `PROPTEST_CASES` floor and the exclusion of `preview-conformance`.
  `quick` still runs the whole workspace in its one `unit-tests` stage, and `coverage` measures
  the whole run in one pass. The split is mechanical, not a judgement of which test is slow.
- **Failure evidence**, uploaded as artifacts (30 days): the nextest JUnit report of
  `fast-tests`, `integration`, `conformance` and `coverage` on failure; the generated and
  failing ITF traces of `conformance` on failure; `target/coverage/lcov.info` from `coverage`
  always; the Playwright `test-results/` (traces, screenshots) on failure; the smoke logs on
  failure.
- **Caches** are keyed by job, OS, the hash of `rust-toolchain.toml`, `Cargo.lock` and
  `tools/versions.toml`, and the job's tool list, with the job-and-OS prefix as the fallback.
  They are restored on every run but saved only by a push to `main` that did not already hit the
  exact key (`.github/actions/setup-rust`, `.github/actions/save-rust-cache`). A pull request
  therefore never writes an entry that would count against the repository's cache quota and evict
  the entries `main` saved. A cache is a speedup only: cargo fingerprints decide what is rebuilt
  and `xtask` checks the version of every tool it runs.
- **`formal.yml` and `security.yml` become schedule- and dispatch-only.** The `conformance`
  job of `ci.yml` runs exactly the stages `cargo xtask conformance` runs, on every push and
  pull request, so the push and pull-request run of `formal.yml` was a duplicate; the same holds
  for `security.yml` against the `security` job. What remains is what only a schedule gives: the
  nightly conformance run with a fresh seed, the Apalache lane, and the weekly security run
  (a new advisory can fail an unchanged tree). The nightly job is renamed
  `Conformance (Quint, nightly)` so the name `Conformance (Quint)` is the blocking job only.
  This resolves ADR 0069's deferral to OPS-04: the named lane exists in `ci.yml`, so nothing is
  removed. The changed-line base of a push (`CHANGED_LINES_BASE` from `github.event.before`,
  ADR 0069) moves to the `coverage` job, the only one that reads it.
- Permissions stay `contents: read`, every action stays pinned by commit SHA, there is no
  `pull_request_target`, and the `action-pins` and `workflows` stages check the new files.

## Consequences

- A red `required` job names the failing level; the failing job's artifacts say why. Jobs run in
  parallel, so the wall-clock time is that of the slowest job (coverage), at the price of
  compiling in every job; the per-job caches carry that cost.
- Thirteen cache entries of `target/` (nine jobs, with the four legs of `platform` each saving
  their own) plus the scheduled `formal.yml`, `fuzz.yml` and `mutation.yml` ones compete for the
  repository's cache quota. `save-rust-cache` therefore prunes before it saves: the workspace
  crates' own build outputs (rebuilt on a fresh checkout anyway, because their sources are
  newer), incremental directories and nextest reports. Registry dependencies stay. If eviction
  still makes jobs slow, share one cache between jobs with the same build profile; correctness
  is unaffected.
- Adding a catalog stage with `in_check` fails the unit test until a job calls it.
- `cargo xtask test` and `check` list the new stage `integration-tests`; docs that named
  `unit-tests` for black-box suites now name it.
- OPS-03 adds the native macOS and arm64 matrix beside these jobs; OPS-04 documents `required`
  as the check to watch. Windows is not supported (D2).
