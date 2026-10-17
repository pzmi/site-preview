# 0080. Doctor asserts the gate's toolset, a `commits` stage, and no `cfg!` platform branches

Date: 2026-10-14

## Status

Accepted (owner, 2026-10-14).

Commit-message enforcement is superseded by [ADR 0083](0083-remove-commit-message-validation.md); the other decisions remain in force.

## Context

Three gaps let a local run disagree with CI. shellcheck was missing on a laptop, so actionlint
skipped its `run:` checks and passed there while CI failed; `doctor` listed the whole manifest
but not what each `check` stage needs, and CI never ran it. Commit messages broke the rules of
CLAUDE.md (`<ID>: summary` under 72 characters, a sign-off, trailers last and in order) with
nothing to catch it: `git commit -s` on a message that already ends in trailers appends
`Signed-off-by` after `Co-Authored-By`. And `if cfg!(target_os = "macos")` compiles every
platform's arm everywhere, so on Linux the macOS arm is instrumented but never run and the
changed-lines gate (ADR 0064) counts it uncovered.

## Decision

- **`cargo xtask doctor [--json] [--all]`** reports every tool a `check` stage declares
  (`needs`/`and_needs`), with the stages that need it: `ok`, `missing` or `wrong` and the
  install hint. Quint (`formal/node_modules`) and the Chromium build are declared by their
  stages too and checked by the same code those stages run. It exits 1 when any is missing or
  wrong. `--all` adds the tools only other lanes need; `--json` prints an array in the style of
  `list-checks`. The CI `static` job installs the whole gate toolset and runs `doctor` first.
- **A `commits` stage** in `check` checks each non-merge commit of `$COMMITS_BASE..HEAD`
  (default `origin/main..HEAD`): the subject `<ID>: summary` with a task id (`LANE-NN`),
  under 72 characters, starting with an imperative verb (a suffix heuristic); a blank line
  after it; a `Signed-off-by` line; and a last paragraph holding only `Signed-off-by`, then any
  `Co-Authored-By`, then any `Claude-Session`. Keys and order only; no value is checked. An
  empty range passes with a note, so commits already on `main` are never checked. CI runs it
  in `static` with `COMMITS_BASE` set from `github.event.before` on a push to `main` (through
  `env:`), and to the merge commit itself on a pull request.
- **`ARCH-platform-branch`**: a `cfg!` naming `target_os`, `target_family`, `target_vendor`,
  `target_env`, `unix` or `windows` in `crates/*/src` is a violation. Platform code is
  `#[cfg]` items or blocks; the nine existing uses were rewritten that way.

## Consequences

The `static` job takes longer (the cached cargo tools, the scanners, Node, Quint and a
Chromium download). A squash-merge or bot commit that reaches `main` without the format fails
`commits` on that push; the merger edits the message instead. `cfg!(miri)` and other
non-platform predicates stay allowed.
