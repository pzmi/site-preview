# plan

Dispatch: inline

## Inputs

intake.{taskId, criteria, cites, affectedCrates, modelImpact, process}

## Steps

1. **Existing plan?** If `.work/<id>/plan.md` exists, reuse it and honour ticked items (resume).
2. **Explore** only what the task names: the cited sections, the files it modifies, and the nearest existing pattern (a sibling crate or module, an existing test of the same level).
3. **Write the checklist** to `.work/<id>/plan.md`: one line per step, `- [ ] path: what`, each traceable to an `Accept:` clause or a `CLAUDE.md` rule. It must cover, where relevant:
   - **the crate each item belongs in** per foundation.md §2.2: domain is pure (no I/O, tokio, clock, randomness, env); application holds commands, queries and port traits; adapters translate technology and never name a sibling adapter or composition; composition is the only wiring; the CLI only parses and delegates
   - **crate-scoped steps**: each step inside one crate; a port another crate consumes, or composition wiring, is its own step
   - **a new crate**: workspace `members` entry and `architecture.toml` entry in the same commit, `[lints] workspace = true`
   - **model first** (foundation.md §6.2) when `modelImpact` is not `none`: the Quint model and invariants, named scenarios, the conformance driver's observation, then the Rust code; plus a negative control
   - **the test levels** touched (foundation.md §5): unit, property, component, contract, architecture, integration, e2e, compile-fail, conformance, browser; one test per `Accept:` clause at least
   - **new dependencies**: `cargo add`, why this crate, its license, and whether `deny.toml` accepts it
   - **ADRs**: a step that adds or reverses an architectural decision (a new dependency of consequence, a crate boundary, a security or protocol policy, a lint or rule exception) writes `doc/adr/NNNN-kebab-title.md` with the next free number, or marks an existing one `Superseded by NNNN`, and updates `doc/adr/README.md`
4. **Owner question.** LARGE with a genuine design fork: one short question with a recommended default, then continue on the answer. No fork: no question.
5. Write `plan.json`.

## Output

```
{ planPath, checklist[], testLevels[], newDependencies[], adrs[], issues[] }
```

## Sensors

`plan.md` exists and has at least one item per `Accept:` clause; `newDependencies` empty or each justified; a model-first item precedes the code when `modelImpact` is not `none`.

## Gates

To implement: always.
