# babysit

Dispatch: inline

## Inputs

ship.{commitSha, pushed}, intake.branch

## Steps

1. **Find the run** for the pushed commit: MCP `actions_list` (workflow runs, filter by branch, match `head_sha`), then `actions_get` until its `status` is `completed`; with `gh`, `gh run watch`. Poll patiently with the Monitor tool or spaced checks; never a busy loop, and never tell the owner to "check back" if you can wait.
2. **Green**: every required job succeeded. Done.
3. **Red**: read the failing job with MCP `get_job_logs` (`failed_only`, `tail_lines` 60 to 120; a large log lands in a file: grep it). Name the failing line. Reproduce with the matching `cargo xtask` subcommand, fix, run the gate, commit `-s` (a fix commit repeats the task id: `<ID>: fix <what>`), push, wait again. A platform-only failure (macOS) that cannot be reproduced here: fix from the log, say so. At most 3 fix attempts, then stop and report the failing job, step and your diagnosis.
4. Never skip, `#[ignore]`, or re-run a flaky test to green; a flake is a bug with a seed.
5. Write `babysit.json`.

## Output

```
{ finalStatus: green|blocked, checks[{name, conclusion}], fixesPushed[{sha, summary}], ciAttempts, issues[] }
```

## Sensors

CI green, or blocked after 3 fixes with the diagnosis recorded.

## Gates

To end: always (terminal).
