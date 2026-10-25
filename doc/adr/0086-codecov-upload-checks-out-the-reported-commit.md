# 0086. Codecov upload checks out the reported commit

Date: 2026-10-25

## Status

Proposed. Amends 0082 (the Codecov upload job).

## Context

ADR 0082 has `coverage-report.yml` upload the LCOV artifact without a checkout. Every upload
since then is queued, and Codecov then marks it "Unusable report": the commit shows 0 files.
The Codecov CLI sends a file list (its "network") with each upload, built from `git ls-files`
in the working directory, or from a walk of that directory when there is no repository. With no
checkout the list holds only `coverage/lcov.info`. Codecov resolves each `SF:` path of the
report against that list, after the `fixes` of `codecov.yml`, and drops any path the list does
not hold. Every source file is dropped, so the report is empty.

## Decision

The upload job checks out the reported commit (`workflow_run.head_sha`) with
`persist-credentials: false` before it downloads the report, so the CLI's file list is the
repository's tracked files. The job still runs no repository code: no build, no script, no
local action. The other guards of ADR 0082 stay: only a successful CI run of a push to `main`
in this repository, `id-token: write` for OIDC, no stored Codecov token, never part of
`Required`.

## Consequences

- Codecov matches the report's paths to repository files and shows per-file coverage.
- The job holding `id-token: write` now has the tree on disk. It runs nothing from it, and the
  checkout is of a commit already on `main`, so this adds no code path an attacker could reach.
- `doc/security.md` and the workflow's header comment describe the checkout.
