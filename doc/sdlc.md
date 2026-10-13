# Software development life cycle

How a change to site-preview goes from a task to green CI on `main`. It records what the
repository does today. The rules come from [`foundation.md`](foundation.md) section 11 (the
execution contract) and section 9 (commands); the integration decision is
[ADR 0032](adr/0032-whole-plan-on-main.md) (D31 in [`tasks.md`](tasks.md)), which supersedes the
planned pull-request flow of [ADR 0027](adr/0027-pr-flow-start.md). The checks CI requires are in
[`required-checks.md`](required-checks.md). Outside contributors start at
[`../CONTRIBUTING.md`](../CONTRIBUTING.md).

## Definition of Ready

A task in [`tasks.md`](tasks.md) is ready to start when:

- every task it depends on has a commit on `main` (subjects are `<ID>: <summary>`, so
  `git log --oneline main | grep " <dep>:"` finds it) and the task has none of its own;
- its row states what is built (create/change), the sections of the specs to read, the
  acceptance criteria and the impact line;
- the user-visible behaviour, the invariants, the crates it touches, the threat model (when
  inputs or resources change), the test levels, the model impact and any compatibility effect
  are stated or follow from the cited sections. A short task records these in a line or two; a
  large task has an ADR or design note first;
- a new dependency, capability or crate edge has its architecture policy and ADR planned before
  code (foundation section 11, step 2).

A task whose text contradicts a decision in the Decisions table is not ready until the
contradiction is fixed; the Decision wins.

## The agent procedure

Work is run by the `/orchestrate` skill
([`.claude/skills/orchestrate/SKILL.md`](../.claude/skills/orchestrate/SKILL.md)) and the `/work`
skill ([`.claude/skills/work/SKILL.md`](../.claude/skills/work/SKILL.md)). They are the real
procedure; this section summarises it.

1. **Orchestrate.** The orchestrator never implements. It picks the ready tasks, creates one git
   worktree per lane under the session scratchpad, and starts one background agent per task with
   `/work <ID>`. Tasks that change the same shared file (`Cargo.toml`, `Cargo.lock`,
   `architecture.toml`, `doc/tasks.md`, `tools/xtask/`) run one after another.
2. **Work.** The `/work` skill runs the stations `intake`, `plan`, `implement`, `test`,
   `polish`, `review`, `qa`, `ship`, `babysit`, writing its state to `.work/<ID>/` (ignored by
   git). The implementing agent reads `CLAUDE.md`, the architecture and testing documents and
   the relevant ADRs; updates the architecture policy or an ADR first when a new dependency or
   capability is needed; updates the Quint model, scenarios and conformance driver first for a
   lifecycle, watcher or reload change; writes failing tests before the smallest implementation;
   runs `cargo xtask quick` while working and `cargo xtask check` before the commit; and reports
   exactly which gates ran and which did not.
3. **Land.** The agent makes exactly one commit on its lane branch with `git commit -s`: subject
   `<ID>: <summary>` (imperative, under 72 characters), a body that says why, an `Impact:` line,
   the sign-off, and the attribution trailer block the session supplies. Only its own files are
   staged. The agent does not push, tag or touch `main`. The orchestrator reads the report,
   spot-checks it, then brings the commit onto `main` (`git merge --ff-only`, or
   `git cherry-pick` when `main` moved).
4. **Gate.** The orchestrator runs `cargo xtask check` on the merged tree. A failure that is
   green in the lane but red when merged is an interaction bug and goes back to an agent; the
   orchestrator does not patch it.
5. **Push.** `git push origin main`, confirmed with `git ls-remote origin main`. A rejected push
   is fetched, rebased, gated again and pushed again. Never a force-push, never a tag push.
6. **Watch CI.** The orchestrator waits for the run of the pushed commit and reads the
   `Required` result (see [`required-checks.md`](required-checks.md)). Without `gh` it uses the
   GitHub MCP tools (`actions_list`, `actions_get`, `get_job_logs`).

Agents never suppress a failing test, widen a lint exemption, remove a modelled invariant or
rewrite a golden fixture to obtain green. A missing pinned tool is an actionable failure that is
reported, never a skipped check.

## Definition of Done

A task is done when:

- code, tests and Quint models agree, and the affected documentation and ADRs are updated in
  the same commit (a change that adds or reverses an architectural decision adds or supersedes
  an ADR, listed in [`adr/README.md`](adr/README.md));
- `cargo xtask check` exits 0 on the merged tree. `quick` is never reported as `check`;
- the architecture rules and their negative fixtures pass, and the packaged-binary smoke passes
  once `release-check` exists (OPS-05);
- the commit is signed off, has the task's subject, and states its impact;
- it is pushed to `main` and the `Required` check is green for that commit;
- the report names the gates that ran, their versions and seeds, and any that did not run.
  A manual exception needs the owner's explicit approval and a follow-up task, and is never
  reported as green.

For a review checkpoint (a task marked with ⛳ in `tasks.md`) it is also done only after the
review described below.

## The gate list

`cargo xtask check` runs these stages, in this order (the table it prints at the end lists the
same). The list is the `in_check` entries of `cargo xtask list-checks`, which prints every check
with its level, command and owner task as JSON; that output is authoritative, and a test
(`should_run_exactly_the_in_check_stages_when_check_is_built`) keeps `check` and the catalog in
step.

| Level | Stages |
|:--|:--|
| Static | `fmt-check`, `fuzz-fmt` |
| Architecture | `architecture` (the policy, the semantic rules and each rule's negative fixture; the catalog lists one `ARCH-*` entry per rule) |
| Static | `clippy` |
| Unit and integration | `unit-tests`, `integration-tests`, `doctests`, `client-js` |
| End to end | `browser` |
| Conformance | `formal-typecheck`, `formal-scenarios`, `formal-simulate`, `formal-freshness`, `formal-generate`, `conformance-replay`, `conformance-controls` |
| Coverage | `coverage`, `changed-lines` |
| Security | `deny`, `deny-fuzz`, `secrets`, `workflows`, `action-pins` |

`cargo xtask check --only <stage>[,<stage>...]` runs just those stages, in the gate's order. CI
uses it so that each job and the same command on a laptop run the same code
([ADR 0072](adr/0072-ci-job-topology-and-stage-parity.md)).

Not part of `check`: `quick` (the fast loop: formatting, architecture, Clippy, fast tests),
`platform-tests` (run on four native platforms by CI, [ADR 0074](adr/0074-native-platform-matrix.md)),
`mutation`, `fuzz`, `formal-verify` and its scheduled stages, `bench`, `release-check`.
[`testing.md`](testing.md) describes each level.

A missing pinned tool fails the stage with the command that installs it
(`cargo xtask doctor` reports them, `cargo xtask install-tools` installs them); nothing is
skipped. The browser stage needs Chromium, which `install-tools` does not install.

## Integration procedure

- **Direct on `main`.** Every task is integrated on `main` for the whole plan. No task switches
  to a pull-request flow or enables branch protection or rulesets. A pull-request flow may
  follow only after the plan is complete, through a new ADR (ADR 0032).
- **One signed-off commit per task**, `git commit -s`, subject `<ID>: <summary>` (ADR 0006).
- **Only the orchestrator pushes.** Implementing agents never push, tag or change remote
  settings. Checkpoint tags (`checkpoint-<ID>`) are annotated, local and never pushed.
- **No force-push.** A rejected push is fetched, rebased and gated again. History on `main` is
  never rewritten.
- **The CI result to watch is `Required`.** CI runs on every push to `main` and on every pull
  request. The `required` job needs every other job, so one result says whether the whole gate
  is green ([`required-checks.md`](required-checks.md)). Because nothing blocks a push, the
  check is a signal that the owner and the orchestrator act on, not an enforced barrier.
- **Review at ⛳.** After a checkpoint task is merged and green, the orchestrator runs
  `/review` over the range since the newest `checkpoint-*` tag, has HIGH and MEDIUM findings
  fixed as a follow-up task (merged and gated the same way), creates the local tag
  `checkpoint-<ID>`, reports to the owner and waits for a go before starting dependent tasks.
  The checkpoint tasks are listed in `tasks.md`.
- **Red CI on `main` is fixed immediately, on `main`.** A red `Required` check is work now: the
  failing job and line are named from the job log, a fix goes in as a new task commit, is gated
  and pushed, and no further task builds on the red commit. A test is never skipped, ignored or
  retried to green. After three consecutive red attempts the work stops and the failing job and
  step are reported to the owner.
- **Outside contributions** arrive as pull requests, which run the same CI. The maintainer
  integrates an accepted one as a signed-off commit on `main`
  ([`../CONTRIBUTING.md`](../CONTRIBUTING.md)).
