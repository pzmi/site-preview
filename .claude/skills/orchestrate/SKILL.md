---
name: orchestrate
description: >
  This skill should be used when the owner asks to "run the next tasks", "orchestrate",
  "work through the task list", "do FS-01 to FS-03", "run lane X", "spawn agents for ...", or
  hands over a batch of tasks or a CI failure to drive to green. Runs several `/work`
  tasks through background subagents in git worktrees, merges each back onto the
  integration branch, runs the gate on the merged tree, pushes, watches CI through the
  GitHub MCP tools, and stops at checkpoints for the owner. One task alone: use `/work`
  directly instead.
---

# Orchestrate: many tasks, one integration branch

The orchestrator never implements a task itself. It picks tasks, starts one background subagent
per task with `/work`, merges the result onto the integration branch, gates and pushes it, and
keeps the owner informed. The owner may be driving from a phone: short messages, plain words,
one question at a time, never a token in chat.

## Input

`$ARGUMENTS`: task ids (`FS-01 FS-02`), a lane name from `doc/tasks.md`, a GitHub Actions run or job
URL (a failure to fix), optionally `--branch <name>` (the integration branch; default `main`),
or nothing (the next ready tasks in `doc/tasks.md` order).

## Startup

1. Read `CLAUDE.md`, the top of `doc/tasks.md` (conventions, decisions, which gate command is
   live: `cargo xtask check`, or `cargo xtask quick` / the fallback before `check` exists) and
   the rows of the tasks in scope with their dependencies.
2. A task is **ready** when every dependency has a commit and the task has none yet. Subjects
   are `<ID>: summary`, so: `git log --oneline <integration> | grep " <dep>:"`.
3. `git status` on the integration branch must be clean; someone else's edits are left alone
   and reported. `git fetch origin` and confirm the local branch is not behind.
4. Toolchain: the SessionStart hook installs the pinned toolchain and fetches crates. If it
   reported a missing tool the gate needs (cargo-nextest, cargo-deny, cargo-llvm-cov, quint),
   install it once now (`cargo install --locked <tool>` / `npm ci` in `formal/`), not per agent.

## Lanes and worktrees

Tasks whose file sets are disjoint run in parallel, one worktree each, under the session
scratchpad (never `/tmp`, never inside the repo):
`git worktree add <scratchpad>/wt-<lane> -b <lane> <integration>`.
Reuse a lane's worktree for its next task after `git -C <scratchpad>/wt-<lane> reset --hard
<integration>`; `git reset --hard` is only ever run with `-C` naming a scratchpad worktree.
Each worktree has its own `target/`; the first build in a lane is slow, later ones are not.

Shared files that serialise tasks (run them one after another, or expect a rebase):

| File | Why | On conflict |
|:--|:--|:--|
| `Cargo.toml` (workspace `members`, `[workspace.dependencies]`) | every new crate or shared dep | merge both lists by hand, keep order |
| `Cargo.lock` | every dependency change | never hand-merge: take the integration side (`git checkout --ours Cargo.lock` in a cherry-pick), then `cargo update -w` to re-resolve; `cargo generate-lockfile` only as a last resort, since it bumps everything |
| `architecture.toml` | every new crate or edge | merge both entries; `cargo xtask architecture` must pass |
| `doc/tasks.md` | status notes, decisions | keep both edits |
| `.cargo/config.toml`, `tools/xtask/` | the gate itself | serialise: never two lanes on the gate |

## Model per task

The `Model` column of `doc/tasks.md` decides; when a row has none:

| Work | Model |
|:--|:--|
| Design, security (path confinement, Host/Origin), protocol/concurrency, Quint models, anything a checkpoint guards | opus |
| A well-specified implementation with clear `Accept:` | sonnet |
| Mechanical: renames, doc pointers, a pinned version bump, a lint fix | haiku |
| Final multi-aspect review, an incident with unclear cause | the strongest model available |

## The task agent's prompt

Start each with the Agent tool, `run_in_background: true`, the model from the table, and this
prompt, filled in:

```
Task <ID> of site-preview (<one-line title>). Work ONLY in the worktree <path> (branch <lane>);
first `git -C <path> reset --hard <integration>`, then cd there for every command.
Read CLAUDE.md, then run Skill('work') with argument <ID> and follow every station.
Context the task row does not carry: <facts from this session: what CI or the owner showed,
decisions already made, files another lane is changing right now>.
Gate: <`cargo xtask check` | `cargo xtask quick` | the fallback in CLAUDE.md>, as doc/tasks.md says.
Constraints: commit on <lane> with `git commit -s` and exactly this trailer block:
<the trailer block from the session's attribution instructions>
Stage only your own files. Do not push, do not touch <integration>, no `gh`, no PR, no tags.
Never run `playwright install` (Chromium is preinstalled under /opt/pw-browsers). No model ids
in repo content. Report: commit sha(s), files changed, the last line of the gate's output,
deviations from the task row and why, and anything a later task must know.
```

A follow-up to a finished agent goes through `SendMessage` to the same agent, which keeps its
context; a new Agent call starts cold. Never start a second agent on the same task.

## Merge back and gate

1. Read the report. A claim that matters (a gate result, a test count, a sha) is spot-checked
   with `git -C <wt> log` / `show --stat`.
2. On the integration branch: `git merge --ff-only <lane>` when it has not moved, else
   `git cherry-pick <sha>...`. Conflicts: the shared-file table above. A conflict in code the
   agent wrote goes back to that agent via `SendMessage` (rebase its lane, re-run the gate).
3. Run the live gate on the merged tree (`cargo xtask check`, or what `doc/tasks.md` names),
   in the background, output to the scratchpad. Red on the merged tree but green in the lane
   is an interaction bug: hand it to the later task's agent, never "fix" it here.
4. Push: `git push origin <integration>`, confirmed with `git ls-remote origin <integration>`.
   Rejected: `git fetch`, rebase, gate again, push. Never force.

## After the push

- **CI** (`gh` may be unavailable, so use MCP): `actions_list` (workflow runs for the branch,
  match the head sha), `actions_get` until `completed`, then `get_job_logs` with `failed_only`
  and `tail_lines` 60 to 120 (a large log lands in a file: grep it). Red is work now: name the
  failing line, hand the fix to a work agent (or the owning lane), merge, gate, push. Never
  skip, `#[ignore]`, or retry a test to green.
- **A failure the owner reports**: read the failed job's log first, name the line that failed,
  then fix the code, or tell the owner the one setting to change and where.

## Checkpoints and the owner

- A task marked ⛳ in `doc/tasks.md` stops its lane after its commit is merged and green: run
  `/review` over `<newest checkpoint-* tag>..HEAD` (in a subagent), hand HIGH and MEDIUM
  findings to a work agent as a fix task, merge and gate, then create the local annotated tag
  `checkpoint-<ID>` (never pushed), report to the owner and wait for a go.
- Ask only what is the owner's to decide (spend, an account, a destructive or outward step, a
  real product choice). Recommend one option. Everything else: decide, do, report.
- A credential is never pasted into chat. It goes into the environment settings or a GitHub
  secret; say which variable name will be read.
- Report in plain words: what now works, what the owner needs to do (numbered, with the exact
  screen and button), and what is still running. Never only a sha.

## Never

Implement a task in the orchestrator's own context; merge a pull request; force-push; push
tags; delete a remote branch; `git reset --hard` outside a scratchpad worktree; hand-merge
`Cargo.lock`; loosen `architecture.toml`, a lint or a test to get green; start a second agent on
the same task; or predict an agent's result before its notification arrives.
