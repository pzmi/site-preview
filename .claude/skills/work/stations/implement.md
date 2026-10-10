# implement

Dispatch: subagent (general-purpose). Parallel sub-subagents only for independent, crate-scoped pieces.

## Inputs

plan.{planPath, checklist, adrs}, intake.{criteria, cites, taskId, modelImpact, gate}, review.findingsToFix (if looping)

## Prompt

Implement task {taskId}. Checklist: {plan.planPath}. Criteria: {criteria}. Gate: {gate}. {findingsToFix ? "Fix these review findings only: " + findingsToFix : ""}

1. Read `CLAUDE.md`; its rules bind you. Read the cited `doc/plan.md` and `doc/foundation.md` sections, the Accepted ADRs for the crates you touch, and the files the checklist names.
2. Work the checklist in order, ticking `- [x]` in `.work/<id>/plan.md` as each lands. Write the test with the code, not after; a failing test first where practical.
3. Model first: if the task touches lifecycle, watcher or reload behaviour, change the Quint model, its scenarios and the conformance driver before the Rust code (foundation.md §6.2). Never remove a modelled invariant to get green.
4. Rules that fail the gate or the review if broken:
   - **Architecture law** (foundation.md §2.1, §3.3): domain has no `std::fs`, `std::net`, `std::process`, env reads, uncontrolled clock, randomness, tokio or HTTP; adapters do not reference sibling adapters or composition; production crates never depend on testkit or conformance; a new crate gets its workspace member and `architecture.toml` entry in the same commit. An architecture-check failure is fixed by fixing the structure, never by loosening `architecture.toml` unless the task says so.
   - `unsafe_code` is forbidden. No `todo!`, `unimplemented!`, `dbg!`. No `#[ignore]`. No `#[allow(...)]` without an ADR naming it.
   - Tests are named `should_<outcome>_when_<condition>` with `// given`, `// when`, `// then`. No sleeps in deterministic tests; OS-watcher integration tests may use bounded condition polling with a comment giving the reason. Real objects over handwritten fakes from testkit; no mocking frameworks.
   - Security (plan.md §5): one decode function, component deny list, per-component `openat` with `O_NOFOLLOW`; never `canonicalize` + `starts_with`; never a URL path in a shell command; no absolute paths, query strings or secrets in logs.
5. A doc comment's behavioural claim names the test that pins it. Comments only for non-obvious constraints and why.
6. A new dependency: `cargo add` (in the right crate, or `[workspace.dependencies]`), and list it in `deviations` so ship names it in the commit message. An architectural decision: write or supersede the ADR from the checklist in `doc/adr/` (next free number; index in `doc/adr/README.md`).
7. `cargo fmt --all`, then the gate until green (`cargo xtask quick` while iterating once it exists). Nothing outside the task's files is staged or reverted.

## Output

```
{ filesChanged[], affectedCrates[], newDependencies[], adrsWritten[], summary, deviations[], issues[] }
```

`affectedCrates`: `git diff --name-only HEAD; git ls-files --others --exclude-standard` mapped to `crates/<name>` or `tools/xtask`.

## Sensors

Diff non-empty; `cargo fmt --all --check` exit 0; every checklist item ticked or listed in `deviations` with a reason.

## Gates

To test: always.
