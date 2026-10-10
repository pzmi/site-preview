# 0036. Hygiene and test-quality rules and their exceptions

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

foundation.md §3.3 (last bullet) and §5.2, and rules 4 and 5 of `doc/tasks.md`, forbid
unapproved lint suppressions, ignored tests, coverage exclusions, sleeps in deterministic
tests and production `todo!()`/`unimplemented!()`/`dbg!()`, and require test names of the
form `should_<outcome>_when_<condition>` with `// given`, `// when`, `// then`. Rule 5 lets an
`[exceptions]` entry citing an ADR approve a departure, but `architecture.toml` had no
place for one. TEST-01 introduced the single sanctioned sleep, `wait_until` in
`preview-testkit/src/poll.rs`. ARCH-07 enforces these on the collector of ADR 0035.

## Decision

- `[[exceptions.lints]]` entries name `path` (workspace-relative file), `lint` (as written:
  `dead_code`, `clippy::too_many_lines`; `coverage` for `#[coverage(off)]`; the clippy lints
  `clippy::todo`, `clippy::unimplemented`, `clippy::dbg_macro` for those macros) and `adr`.
  An `#[allow]`, `#[expect]` or `#[coverage(off)]` (also through `cfg_attr`, also inside
  macro bodies) passes only with a matching entry whose `doc/adr/<adr>-*.md` exists. An entry
  that matches nothing fails, so approvals cannot outlive the code they approved.
- `#[ignore]` is never allowed; there is no exception.
- `todo!`, `unimplemented!` and `dbg!` (bare or through `std`/`core`) fail in production code
  of every crate; test code may use them.
- Sleeping (`std::thread::sleep`, `sleep_ms`, `tokio::time::sleep`, `sleep_until`, also
  through aliases) fails in test code: test targets, `#[cfg(test)]` items, test functions
  and every file of a `test_only` crate. The exemption is structural, not a policy entry:
  only production code of `preview-testkit/src/poll.rs`, the bounded `wait_until` helper,
  may sleep.
- Every function with a test attribute (`#[test]`, `#[tokio::test]`, any path ending in
  `test`) is named `^should_[a-z0-9_]+_when_[a-z0-9_]+$` and has `// given`, `// when` and
  `// then` comment lines in that order in its body. A test written inside a macro body
  fails, since neither can be checked there.
- `cargo xtask list-checks` lists every architecture rule id with the task that introduced
  it, as rows of the `architecture` stage.

## Consequences

- A combined `// given / when` comment no longer passes; tests that used one were rewritten.
- Real-OS tests that must wait call `preview_testkit::poll::wait_until` with a reason.
- Approving a suppression takes an ADR and a policy entry in the same commit; removing the
  suppression requires removing the entry.
