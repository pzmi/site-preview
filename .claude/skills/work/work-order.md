# Work order

One JSON file per task, `.work/<id>/work-order.json`, plus one `.work/<id>/<station>.json` per finished station. `.work/` is gitignored: it is the run's memory, not a deliverable, so a fresh session can resume.

## work-order.json

```
{
  "taskId": "S7",             // or "issue-42"
  "title": "Symlink-safe open",
  "size": "S|M|L", "process": "SMALL|MEDIUM|LARGE",
  "checkpoint": false,        // task carries the ⛳ mark
  "branch": "main|<lane>",    // the branch the commit lands on
  "gate": "check|quick|fallback",
  "pushOwner": false,         // false when the caller said "do not push"
  "status": "<station name>|done|failed",
  "stationOutputs": { "intake": true, "plan": true },
  "loopCount": 0,
  "commitSha": null,
  "issues": []
}
```

`issues` collects anything a station found wrong with the process itself (`{station, category, description}`, category one of `skill-gap`, `sensor-miss`, `gate-miss`, `tooling`, `process`); the completion report surfaces them so the owner can amend this skill.

## Station files

`.work/<id>/<station>.json` holds that station's output object as `stations/<station>.md` defines it. The next station reads named prior outputs only. Station `plan` also writes `.work/<id>/plan.md`, the checklist.

## Field lifecycle

| Field | Writer | Readers |
|:--|:--|:--|
| taskId, title, size, process, checkpoint, gate, pushOwner | intake | all |
| branch | intake | ship, babysit |
| criteria, cites, modelImpact (intake output) | intake | plan, implement, test, review |
| checklist, adrs (plan output) | plan | implement, test, review, ship |
| filesChanged, affectedCrates | implement | test, polish, review, qa |
| commitSha | ship | babysit |
| status, stationOutputs, loopCount | orchestrator | orchestrator |
