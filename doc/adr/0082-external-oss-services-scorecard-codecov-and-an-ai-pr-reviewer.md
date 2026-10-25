# 0082. External OSS services: Scorecard, Codecov and an AI PR reviewer

Date: 2026-10-14

## Status

Accepted (owner, 2026-10-14). Amended by 0086.

## Context

The repository is public, so several hosted services are free for it. Most of them would repeat
a check the gate already runs: Clippy, cargo-deny, Gitleaks, actionlint, CodeQL, Dependabot,
coverage floors, fuzzing, mutation testing and Quint conformance (ADR 0024). A second
quality gate in a SaaS product (SonarQube Cloud, Codacy, Snyk) would mostly report Clippy and
advisory findings again, and would make a vendor's availability part of whether `main` is
green. Three services add something the gate cannot do itself:

- **OpenSSF Scorecard** grades the repository's supply-chain practice from the outside (pinned
  dependencies, token permissions, dangerous workflow patterns, signed releases) and publishes
  the score.
- **Codecov** keeps a history of the coverage the `coverage` job already measures, per file
  and per commit, which the LCOV artifact (30-day retention) does not.
- **An AI pull-request reviewer** (CodeRabbit, free for public repositories) is a reviewer
  independent of the agents that write the code. Integration is direct on `main` (ADR 0032),
  so today it sees only pull requests: Dependabot's and outside contributors'.

## Decision

- **Scorecard** runs in `.github/workflows/scorecard.yml` on each push to `main`, weekly and by
  hand, with `publish_results: true`. Its SARIF goes to code scanning; the README shows the
  badge. `security-events: write` and `id-token: write` are granted to that job only.
- **Codecov** is fed by `.github/workflows/coverage-report.yml`, which runs on `workflow_run`
  of CI. For a successful CI run of a push to `main` in this repository it downloads the
  `lcov-<run>-<attempt>` artifact of the `coverage` job and uploads it with OIDC, so no Codecov
  token is stored. It checks nothing out and runs no repository code, so `id-token: write` is
  never held by a job that builds or tests the tree. It is not a job of `ci.yml`, so it is not
  in `Required` (ADR 0072). `codecov.yml` maps the runner's absolute paths to repository paths,
  measures `crates/` only (as `coverage-floor.toml` does, ADR 0066), and marks every Codecov
  status informational.
- **`coverage-floor.toml` and the changed-lines stage stay the coverage gate.** Codecov is a
  report. A Codecov status never blocks, and a disagreement between the two is settled by
  `cargo xtask coverage`.
- **CodeRabbit** is configured by `.coderabbit.yaml`. It reviews against `CLAUDE.md`,
  `AGENTS.md` and the cited spec sections, and its built-in Clippy, Gitleaks and actionlint
  runs are off because the gate runs them. Its findings are claims to verify, never a gate.
- **Not adopted:** SonarQube Cloud, Codacy and Snyk (they repeat the gate), Coveralls (repeats
  Codecov), a second CI provider (GitHub-hosted runners are free for public repositories), and
  a second AI reviewer running by default. A documentation site (GitHub Pages) waits until
  `v0.1.0` is tagged (OPS-07), when it has an install path to show.

## Consequences

- The owner enables each service once, outside the repository: installs the CodeRabbit GitHub
  App, activates the repository on Codecov, and turns on secret scanning and push protection
  in the repository settings. Until then `coverage-report.yml` fails visibly and
  `.coderabbit.yaml` is inert. Scorecard needs no sign-up.
- Scorecard will score `Branch-Protection` and `Code-Review` low while ADR 0032 holds. That is
  the documented trade-off of direct integration, not a defect to work around.
- Three more actions are pinned by commit SHA (`ossf/scorecard-action`,
  `codecov/codecov-action`, and `github/codeql-action/upload-sarif` at the CodeQL pin), checked
  by `action-pins` and kept current by Dependabot.
- Coverage from pull requests is not uploaded: their CI runs may come from forks, and on this
  repository's direct-to-`main` flow the push is what matters. A PR flow (a new ADR, per ADR
  0032) can add it.
- Implemented by OPS-08.
