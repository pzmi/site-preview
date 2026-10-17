# 0083. Remove commit-message validation

Date: 2026-10-17

## Status

Accepted.

## Context

CI run 39033576579 passed the code and test gates but failed because a Dependabot
squash commit did not have a task ID in its subject. Commit-message formatting is
not useful enough to block dependency updates or other contributions.

## Decision

Remove the `commits` stage from `cargo xtask check`, the catalog and CI, and delete
its validator and tests. Remove `COMMITS_BASE` and the static job's full-history
checkout. Dependabot uses its default commit messages without a custom prefix.

This supersedes only the commit-message enforcement portions of ADRs 0079 and
0080. Their other decisions, including `doctor` and platform branch checks, remain
in force. Commit style and DCO sign-off remain contributor guidance.

## Consequences

Commit subjects, task IDs and trailer formatting no longer fail the build.
The remaining quality, security and test stages continue to block CI failures.
