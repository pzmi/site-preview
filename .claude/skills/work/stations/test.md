# test

Dispatch: subagent (general-purpose)

## Inputs

implement.{filesChanged, affectedCrates}, intake.{criteria, modelImpact, gate}

## Prompt

Verify test coverage and run the gate for the working-tree diff.

1. Map every `Accept:` clause to at least one test that would fail if the behaviour broke. A clause with none: write the test.
2. Per changed source file, check the level (foundation.md §5):
   - **unit**: pure logic beside the module (`#[cfg(test)] mod tests`), no I/O
   - **property** (`proptest`): invariants over generated inputs (path normalisation, debounce, revision monotonicity); a found counterexample is committed as a regression seed
   - **component**: real application code over testkit in-memory adapters
   - **contract**: a port with a fake and a real adapter runs one shared harness against both
   - **architecture**: `cargo xtask architecture` and its negative fixtures; a new rule has a negative case
   - **integration**: real tempdir filesystem, watcher, sockets on port 0
   - **e2e** (`assert_cmd`): the built binary, its stdout JSONL, stderr and exit codes
   - **compile-fail** (`trybuild`): an API misuse that must not compile
   - **conformance**: Quint traces replayed against the real code, plus the negative control
   - **browser**: Playwright against a served fixture, when the reload loop changed
3. Conventions: `should_<outcome>_when_<condition>`, `// given / when / then`, real objects over fakes, no mocking frameworks, no `#[ignore]`, no sleeps (OS-watcher tests: bounded condition polling with a reason), dynamic ports, `tempfile` dirs. Never assert on log lines alone.
4. Run the gate (`cargo xtask check`, or what `doc/tasks.md` names) in the foreground, never in the background. Iterate to green. A missing tool is installed pinned or reported; a skipped check is never green.
5. Failures outside your diff (someone else's WIP): report them, do not fix or hide them.

## Output

```
{ testsAdded[], criteriaCoverage[{criterion, tests[]}], gate, checkPassed: bool, testCount: int, gaps[], issues[] }
```

## Sensors

Gate exit 0; every criterion has a test; `gaps` empty.

## Gates

To polish: always.
