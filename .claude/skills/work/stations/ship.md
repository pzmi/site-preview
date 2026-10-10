# ship

Dispatch: inline

## Inputs

intake.{taskId, title, branch, checkpoint, gate, pushOwner}, implement.{filesChanged, newDependencies, adrsWritten}

## Steps

1. **Final gate.** The gate green once more on the exact tree being committed (`cargo xtask check` once it exists). `git status --short`: every changed file is yours or listed; someone else's uncommitted edits stay unstaged.
2. **Docs.** A decision that adds to or reverses `doc/plan.md`, `doc/foundation.md` or an Accepted ADR has its ADR in this commit (`doc/adr/NNNN-kebab-title.md`, next free number, listed in `doc/adr/README.md`; a reversed ADR marked `Superseded by NNNN`). Affected docs updated in the same commit.
3. **Commit.** `git add <the files>` (never `-A`, never `.`). `git commit -s` with:
   - subject `<ID>: <summary>` (imperative, under 72 chars)
   - a body saying what and why; each new dependency named with its reason; the gate that ran
   - the trailer block exactly as the caller's instructions give it, at the end; none if none was given
   - no model ids, secrets, tokens, internal hostnames or private-repo content
   Never `--no-verify`. If a hook fails, fix the cause; do not bypass.
4. **Push.** Only when the caller owns the push (`pushOwner`): `git push origin <branch>`, confirmed with `git ls-remote origin <branch>`. Rejected: `git fetch`, `git rebase origin/<branch>`, the gate, push again. Never force. Not the push owner: stop here and report "commit ready" with the sha.
5. Checkpoint task: say that a `/review` is due; do not start the next task.
6. Write `ship.json`.

## Output

```
{ commitSha, pushed: bool, issues[] }
```

## Sensors

Commit exists, has a `Signed-off-by:` line (`git log -1 --format=%B | grep -q '^Signed-off-by:'`), and its subject starts with `<ID>: `; the gate was green on it; if pushed, the remote head equals the sha.

## Gates

- To babysit: pushed.
- To end: not the push owner (report "commit ready").
- Stop and ask: a force-push would be needed.
