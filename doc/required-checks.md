# Required checks

The CI result that must be green on `main` is the `required` job of
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). It is an aggregate: it needs every
other job of the workflow and fails when any of them failed, was cancelled or was skipped, so a
job cannot drop out of the gate by not running ([ADR 0072](adr/0072-ci-job-topology-and-stage-parity.md)).
Each job runs `cargo xtask check --only <stages>` (or, for `platform`,
`cargo xtask test --only platform-tests`), so the same stages run on a laptop with
`cargo xtask check` ([`sdlc.md`](sdlc.md)).

## No branch protection

No branch protection rule and no ruleset is enabled on `main` or any other branch, and no task
enables one. Integration is direct on `main` ([ADR 0032](adr/0032-whole-plan-on-main.md)), so
"required" here means "must be green, and red is fixed immediately on `main`", not "GitHub
refuses the merge". If a protection rule is ever introduced, it requires the single check
`Required` below and nothing else, and it comes with a new ADR.

## The checks

The check name shown by GitHub is the job's `name`. The workflow runs on pushes to `main` and on
pull requests.

| Job id | Display name | Runs (`cargo xtask ...`) |
|:--|:--|:--|
| `architecture` | Architecture and policy | `check --only architecture` |
| `static` | Static quality | `check --only fmt-check,fuzz-fmt,clippy,docs` |
| `fast-tests` | Fast tests | `check --only unit-tests,doctests,client-js` |
| `integration` | Integration | `check --only integration-tests` |
| `platform` | Platform (ubuntu-24.04) | `test --only platform-tests` |
| `platform` | Platform (ubuntu-24.04-arm) | `test --only platform-tests` |
| `platform` | Platform (macos-15) | `test --only platform-tests` |
| `platform` | Platform (macos-15-intel) | `test --only platform-tests` |
| `conformance` | Conformance (Quint) | `check --only formal-typecheck,formal-scenarios,formal-simulate,formal-freshness,formal-generate,conformance-replay,conformance-controls` |
| `coverage` | Coverage | `check --only coverage,changed-lines` |
| `security` | Security | `check --only deny,deny-fuzz,secrets,workflows,action-pins` |
| `browser` | Browser | `check --only browser` |
| `artifact-smoke` | Artifact smoke | `cargo xtask release-check --target x86_64-unknown-linux-musl`: builds, packages and smokes the release archive |
| `required` | Required | the aggregate; needs all of the above |

The four `platform` rows are the legs of one matrix job (`fail-fast: false`, so one red leg never
hides another). `required` needs the job id `platform`, which is green only when every leg is
([ADR 0074](adr/0074-native-platform-matrix.md)). Every stage of `check` is in exactly one job; a
test in `tools/xtask/src/check.rs` fails when a stage is in none or in two.

## Other workflows

These are not in `required`. `formal.yml` (Apalache, extended simulation, Miri), `fuzz.yml`,
`mutation.yml` and `security.yml` run on a schedule or by hand. `release.yml` runs on a pushed
`v*` tag or by hand from a tag, and makes a draft release ([`release.md`](release.md)). `codeql.yml` also runs on pushes
and pull requests, as its own workflow. A red run of any of them is reported to the owner and
fixed as a task, but does not make `Required` red.

## When a check goes red

Read the failed job's log, name the failing line, and fix it on `main`
([`sdlc.md`](sdlc.md#integration-procedure)). Locally, a stage that fails only from machine load
may be rerun alone with `cargo xtask check --only <stage>`; the rerun is reported with the
original failure, and the full `check` is still run before the commit. In CI a red job is
fixed, never re-run to green; the one exception is a job that died before any test body ran (a
runner or network failure), which may be re-run once and is reported.
