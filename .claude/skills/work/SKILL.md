---
name: work
description: >
  This skill should be used when the user asks to "work on FND-01", "pick up a task",
  "implement task HTTP-03", "do the next task", or names a task id from doc/tasks.md
  (or a GitHub issue number). Runs the full station sequence: intake, plan,
  implement, test, polish, review, qa, ship, babysit. Ends with a DCO-signed-off
  commit, pushed when the caller owns the push.
---

# Work: task orchestrator

One task from `doc/tasks.md` (or one GitHub issue) to a green, signed-off commit. Thin routing here; the detail lives in `stations/{name}.md`, read on entry to each station.

## The gate

Every station that says "the gate" means the command `doc/tasks.md` names as live:

| Stage of the repo | Gate |
|:--|:--|
| No `cargo xtask` yet | `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` |
| `xtask` has `quick`, no `check` | `cargo xtask quick` |
| `xtask` has `check` | `cargo xtask quick` while iterating, `cargo xtask check` before commit |

`quick` is not `check` (foundation.md §9). Never report `quick` as the full gate.

Run the gate in the foreground, with the lane's own `CARGO_TARGET_DIR` when the caller names one
(else one under the scratchpad; `/dev/shm` is RAM, only when memory allows), and delete that dir when the task
ends. Local green but CI red, or the reverse: `cargo xtask doctor` first.

## Tooling without `gh`

| | Agent without `gh` (sandbox) | With `gh` |
|:--|:--|:--|
| CI verdict | MCP `actions_list`, `actions_get` | `gh run list`, `gh run watch` |
| CI failure logs | MCP `get_job_logs` (`failed_only`) | `gh run view --log-failed` |
| Issue text | MCP `issue_read` | `gh issue view` |

Detect `gh` with `command -v gh`. An implementing agent handed a worktree and told "do not push" stops after the commit and reports; pushing and CI belong to whoever owns the push.

## Constraints

Never:

- Ask "shall I proceed?" or confirm commits or checklists. A question is for a real ambiguity only, phrased so one short reply answers it
- Skip a station, or implement without a plan checklist
- Run a repeatable job as a bare ad-hoc command once an `xtask` subcommand covers it (`cargo xtask architecture`, not a hand-rolled `cargo metadata | jq`); a missing repeatable job becomes an `xtask` subcommand
- Loosen `architecture.toml`, `[workspace.lints]`, `deny.toml` or a test to get green, unless the task says so; add `#[allow(...)]` without an ADR, `#[ignore]`, or a sleep in a deterministic test
- Add a dependency other than with `cargo add` (name it and why in the commit message)
- Commit without polish, without a green gate on the exact tree, or without a sign-off
- End the turn with the gate, a build or a server still running in the background, or waiting on one
- Kill a process by name pattern (`pkill -f`, `killall`): only PIDs you started
- Force-push, bypass hooks (`--no-verify`), push tags, or merge a PR
- Review your own diff inline: the review station dispatches `/review` subagents
- Run `playwright install` (Chromium is preinstalled; `PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`)
- Put a model id, secret, token, internal hostname or private-repo content in any repo file (public repo)

Always:

- Read `CLAUDE.md` first, and pass its rules to every subagent
- Read `stations/{name}.md` before entering that station
- Write `.work/<id>/<station>.json` after every station and update `.work/<id>/work-order.json` before the next (see `work-order.md`)
- Commit with the subject `<ID>: <summary>` and a trailer block last, in the order `Signed-off-by`, `Co-Authored-By`, `Claude-Session` (the `commits` stage checks it). `git commit -s` appends `Signed-off-by` after an existing trailer block, so with the caller's trailers write every line by hand and drop `-s`
- Use the trailer block the caller's instructions give; add none of your own
- Stage only files you changed (never `git add -A` or `git add .`)
- Record an architectural decision as an ADR: a change that adds or reverses one adds a new record with `cargo xtask adr new "<title>"` (`--number N` when the caller reserved one; Nygard format: Status, Context, Decision, Consequences; `cargo xtask adr index --fix` updates `doc/adr/README.md`) or supersedes an existing one, in the same commit
- Stop at every checkpoint task (⛳ in `doc/tasks.md`) after the commit: report that a `/review` range is due, and do not start the next task

## Input

`$ARGUMENTS`: a task id, or a GitHub issue number or URL, plus optional `--small`/`--medium`/`--large`. Absent: take the first task in `doc/tasks.md` whose dependencies all have a commit (`git log --oneline | grep " <dep>:"`) and which has none of its own yet.

## Startup

1. Read `processes.md` and `work-order.md`.
2. `.work/` must be ignored (`git check-ignore -q .work/x`); if it is not, add `/.work/` to `.gitignore` in this task's commit and say so. `mkdir -p .work/<id>`. If `.work/<id>/work-order.json` exists, resume after the last station in `stationOutputs`, after checking `git log` and `git status` so finished work is never redone.
3. Working tree must be clean of other people's changes: `git status --short`. Someone else's uncommitted edits are left untouched and unstaged; mention them in the report.

## Routing

```
run_inline(intake); run_inline(plan)
for station in process.sequence_after(plan):
    def = read("stations/{station}.md")
    out = inline ? execute(def) : subagent(def, inputs_from(work_order))
    write(".work/<id>/{station}.json", out); work_order.stationOutputs[station] = true
    if not sensors(def.sensors, out): retry once with the failure attached, else stop and report
    next = gates(def.gates, out)   # first passing gate wins; none -> stop and report
    if next != following(station): jump(next); work_order.loopCount += 1
```

## Dispatch

Inline: intake, plan, qa, ship, babysit. Subagent (fresh context, one station, structured output): implement, test, polish, review. A subagent crash counts as a failed sensor. Subagents work in the same directory (the worktree, if one was given).

## Errors

| Scenario | Action |
|:--|:--|
| Sensor fails twice | Stop. Report station, sensor, output. |
| No gate passes | Stop. Report station, output, gates. |
| Gate red for a reason outside the diff | Stop. Report; do not "fix" other people's WIP. |
| A required tool is missing (nextest, deny, quint) | Install it pinned (`cargo install --locked`, `npm ci` in `formal/`) or stop and report; never treat a skipped check as green. |
| CI red 3 times | Stop at babysit. Report the failing job and step. |
| Force-push needed | Stop at ship. Ask the owner. |
| Plan or task text is wrong or contradictory | Fix it minimally in `doc/plan.md` or `doc/tasks.md` in the same commit and say so (an ADR if it is a decision). |

## Files

`processes.md`, `work-order.md`, and `stations/{intake,plan,implement,test,polish,review,qa,ship,babysit}.md`.

## Completion

Report in a few lines: what was built, the gate that ran and its result with the test count, the commit sha, push and CI status (or "commit ready, not pushed"), any deviation from the plan, anything a later task must know, and `issues` from the work order.
