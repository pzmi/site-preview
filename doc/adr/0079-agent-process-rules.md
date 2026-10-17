# 0079. Agent process rules from the P0 build

Date: 2026-10-14

## Status

Accepted (owner, 2026-10-14)

Commit-message enforcement is superseded by [ADR 0083](0083-remove-commit-message-validation.md); the other decisions remain in force.

## Context

The P0 build ran many background agents in parallel worktrees on one machine, integrating
directly on `main` (ADR 0032). The same process failures recurred: commits landed on top of red
CI; parallel lanes took the same ADR number or edited the same registry (the `formal.yml`
matrix, the conformance model list, the `testing.md` tables, the ADR index) and conflicted on
merge; agents ended a turn while the gate still ran in the background, or killed processes by
name pattern and took down another lane's gate; per-worktree `target/` directories filled the
disk; `git commit -s` placed `Signed-off-by` after the session's trailer block; a local gate
passed where CI failed because a pinned tool was missing or wrong; `cfg!(target_os = …)` runtime
branches compiled platform code that did not belong on a target; acceptance criteria that only
the owner can meet (tag pushes, workflow dispatch, settings, rulesets) were left to agents; and
owner decisions piled up until the end of the plan.

## Decision

- Commits are landed by `cargo xtask land`, one at a time, each only after the previous commit's
  CI is green; nothing lands on red CI. `land` refuses a dirty or stale `main`, resolves only
  `Cargo.lock` and the ADR index, and pushes only on a green `check`.
- ADRs are created with `cargo xtask adr new`; the orchestrator reserves numbers when it
  dispatches and passes `--number`. A shared registry has one owning lane per dispatch round,
  or is regenerated.
- The commit trailer block is last, in the order `Signed-off-by`, `Co-Authored-By`,
  `Claude-Session`, checked by the `commits` stage of `check`.
- Agents run the gate in the foreground, commit only on a green gate, never end a turn with work
  running in the background, kill only PIDs they started, and use a per-lane
  `CARGO_TARGET_DIR` (under `/dev/shm` only when RAM allows) that is deleted when done.
- `cargo xtask doctor` is the first step when the local gate and CI disagree.
- Platform code uses `#[cfg]` items or blocks, never `cfg!()` runtime branches (an architecture
  rule).
- Acceptance criteria only the owner can meet are marked `(owner)` in `doc/tasks.md`; owner
  decisions are batched and asked at each ⛳ checkpoint.

## Consequences

`CLAUDE.md`, the `orchestrate` and `work` skills, `doc/tasks.md` and `doc/sdlc.md` state these
rules; the `commits` stage, the `cfg!` architecture rule, `doctor`, `adr` and `land` enforce
the mechanical ones. Landing is slower (one CI run per commit) but `main` stays green and a red
run names one commit.
