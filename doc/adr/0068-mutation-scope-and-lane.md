# 0068. Mutation testing: scope, test selection and the weekly lane

Date: 2026-10-13

## Status

Proposed (planner default — owner may override)

## Context

Foundation.md section 5.3 asks for a killed-mutant score of at least 90% on critical domain
algorithms, reported weekly, with browser and end-to-end suites excluded by design.
TEST-08 adds the runner but does not say which code is in scope, which tests may kill a
mutant, whether the run joins `check`, or what a missed mutant does to the command's exit code.

## Decision

- **Scope** (`.cargo/mutants.toml`): `preview-domain`, `preview-application` and
  `preview-adapter-http/src/inject`. These are the pure policies and the HTML injection,
  whose tests are fast and exact. Adapters, composition and the CLI are exercised by
  integration, conformance and browser suites that are not selected here, and a survivor
  there would only say that those suites were not run.
- **Test selection:** cargo-mutants tests only the package that owns the mutated file, with
  nextest. The `preview-cli` black-box tests, `preview-conformance` and the Playwright suite
  therefore never run for these mutants. The nextest filter also drops the trybuild
  `compile_fail` binaries (a nested cargo build per mutant that cannot see a behavioural
  change) and the OS watcher tests. A survivor means the selected tests miss the path; it
  is never claimed eliminated by a suite that did not run.
- **Score:** `caught / (caught + missed + timeout)`; unviable mutants are left out and a
  timeout counts against the score.
- **Not in `check`:** a run takes far longer than any gate stage. `cargo xtask mutation` is a
  catalog row with `in_check = false`; `mutation.yml` runs it weekly and on demand, with
  read-only permissions, and uploads the report.
- **Exit code:** missed mutants and timeouts are findings, not failures; a failing unmutated
  baseline, a missing tool or a crash is. `--in-diff <rev>` narrows a run to the mutants on
  lines changed since the merge-base with `rev`, the base `changed-lines` uses (ADR 0064).
  Gating new survivors is a later ratchet (foundation.md section 5.3).

## Consequences

- `doc/mutation-baseline.md` holds the measured score and the triage of every survivor;
  a survivor is either equivalent (with the reason) or needs a test.
- A weekly red is impossible from survivors alone, so a drop in score is seen only by reading
  the summary until the ratchet exists.
- Widening the scope means adding a glob and re-measuring the baseline.
