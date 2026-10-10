# intake

Dispatch: inline

## Inputs

`$ARGUMENTS`: task id, issue number or URL, optional size override; the caller's prompt (worktree, branch, gate, "do not push").

## Steps

1. **Find the task.** Task id: read its section in `doc/tasks.md` and its row in the overview (dependencies, model, ⛳ mark, gate). Issue number: read it with MCP `issue_read` (or `gh issue view` when `gh` exists); if its title starts with a task id, use that task's section too. Absent: pick per SKILL.md "Input".
2. **Dependencies.** Every dependency must already be committed (`git log --oneline | grep " <dep>:"`). A missing one: stop and report; do not start work that builds on nothing.
3. **Read the citations.** Read every `plan.md` and `foundation.md` section the task cites, foundation.md §2 (workspace, dependency law) and §5.2 (test rules), the Accepted ADRs in `doc/adr/` that touch the affected crates, and every existing file the task modifies.
4. **Definition of Ready** (foundation.md §11): user-visible behaviour, invariants, affected crates, threat model when inputs or resources change, test levels, model impact (does it touch lifecycle, watcher or reload state? then the Quint model changes first), testable `Accept:`. Not met: ask the owner one short question; if the gap is only wording, fix it minimally in `doc/tasks.md` and say so.
5. **Classify** per `processes.md`.
6. **Branch and gate.** Stay on the branch you were given (`main`, or the worktree lane the caller named). Record the gate the task file names, and whether you own the push (the caller said "do not push": you do not).
7. Write `work-order.json` and `intake.json`.

## Output

```
{ taskId, title, criteria[], cites[], dependsOn[], affectedCrates[], modelImpact: "none"|"<model>.qnt", size, process, checkpoint: bool, branch, gate, pushOwner: bool, issues[] }
```

## Sensors

Task section found; every dependency committed; `criteria` non-empty (each `Accept:` clause one entry).

## Gates

To plan: always.
