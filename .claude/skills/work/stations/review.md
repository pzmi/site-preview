# review

Dispatch: subagent (general-purpose), one `/review` subagent per independent slice. Never a self-read of the diff in the orchestrator.

## Inputs

intake.{taskId, cites, criteria, checkpoint}, implement.{filesChanged, affectedCrates}

## Prompt

Load `/review` and run it on the uncommitted diff plus the new files of task {taskId} (the working tree against `HEAD`; the skill reads a range, so pass `HEAD` and say the changes are uncommitted; for a checkpoint task also pass the range since the newest local `checkpoint-*` tag). The skill is read-only and reports findings; it edits nothing.

Slicing: one subagent for a small diff; for a LARGE task one per crate (`crates/<name>` plus its tests) and one for cross-cutting rules (architecture policy, Quint model and conformance, docs and ADRs, plan fidelity). Give each subagent `CLAUDE.md`, the cited sections and the `Accept:` clauses.

Triage the combined findings:

- HIGH and MEDIUM that are local (a test, a doc comment, a guard, a missing ADR) go to a fix pass: dispatch `implement` with `findingsToFix`, no re-planning.
- A finding that needs a structural change (a moved responsibility, a new port, a crate boundary, a protocol or model change) is a gate decision, below.
- LOW findings: fix if trivial, otherwise list.
- A finding you disagree with goes in `findingsSkipped` with the reason; never silently dropped.

After fixes the gate must be green.

## Output

```
{ findingsApplied[{file, summary}], findingsSkipped[{file, summary, reason}], structuralChangesNeeded: bool, checkPassed: bool, issues[] }
```

## Sensors

Gate exit 0; no HIGH finding unresolved or skipped.

## Gates

- To qa: AGENT, no structural change needed.
- To implement: AGENT, structural change needed and `loopCount < 2`.
- To qa: SCRIPT, `loopCount >= 2` (force on; remaining findings go in the ship report).
