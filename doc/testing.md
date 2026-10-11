# Testing

How site-preview's tests are organised, what each level proves, where it lives, and which
command runs it. The rules come from [`foundation.md`](foundation.md) section 5 (levels and test
quality) and section 9 (commands); the test locations are decided in
[ADR 0016](adr/0016-test-locations.md). `cargo xtask list-checks` prints the machine-readable
catalog of every check, its level, its command and its owner task. Every command in this file
is one of its entries, or a planned one it lists as not yet available.

## Levels

"In place" means the level runs today. "Planned" names the task that adds it.

| Level | What it proves | Where it lives | Runs under | Status |
|:--|:--|:--|:--|:--|
| Unit | Pure logic and boundary examples | `#[cfg(test)] mod tests` beside the module, e.g. `crates/preview-domain/src/`, `crates/preview-adapter-fs/src/` | `unit-tests` (`cargo nextest run`) | in place |
| Doctest | Examples in `///` and `//!` compile and pass | Doc comments; e.g. the `wait_until` example in `crates/preview-testkit/src/poll.rs` | `doctests` | in place |
| Property | Invariants over generated values, with persisted counterexamples | `proptest!` inside `crates/preview-domain/src/` and `crates/preview-application/src/`; black-box properties in `crates/preview-domain/tests/properties.rs` | `unit-tests`; the `ci` profile raises the case count | in place |
| Component | Real application code over testkit fakes, no external services | `crates/preview-application/tests/`, `crates/preview-adapter-http/tests/` (router and files over `InMemorySite`) | `unit-tests` | in place |
| Contract | Each fake and each real adapter of a port meets one shared harness | `crates/preview-testkit/src/contracts/`; runs in `crates/preview-testkit/tests/site_content_fake.rs` and `crates/preview-adapter-fs/tests/site_content_contract.rs` | `unit-tests` | in place for `SiteContent`; `ChangeSource` planned (TEST-05) |
| Architecture | Crate, dependency, lint and test-hygiene rules, and each rule's negative control | `tools/xtask/src/architecture/`; fixtures in `tests/architecture-valid/` and `tests/architecture-invalid/`; `tools/xtask/tests/architecture_fixtures.rs` | `architecture` (inside `quick` and `check`) | in place |
| Security fixtures | The dependency policy, secret scanning and workflow lint reject a seeded bad input | `tools/xtask/tests/security_fixtures.rs` | `unit-tests` | in place |
| Integration | Real tempdir filesystem, threads, the root capability under a concurrent symlink swap | `crates/preview-adapter-fs/tests/` (`root.rs`, `capability.rs`, `symlink_race.rs`) | `unit-tests` | in place; OS watcher tests planned with the watcher |
| CLI black box | The built binary's flags, output and exit codes through `assert_cmd` | `crates/preview-cli/tests/cli_meta.rs` | `unit-tests` | partial; the full suite is E2E-01 |
| E2E | The installed binary's HTTP and security behaviour | planned | | E2E-01 (first M1 suite), E2E-02, E2E-04+ (browser) |
| Compile-fail | An invalid API use does not compile (`trybuild`) | planned | | ARCH-08 |
| Conformance | Quint model traces replayed against the real code | `crates/preview-conformance` is a placeholder today | `conformance` (not yet available) | FORMAL-01 to FORMAL-09 |
| Coverage | Line coverage floor and changed-line coverage | planned; targets in [`foundation.md`](foundation.md) 5.3 and [ADR 0018](adr/0018-coverage-gates.md) | `coverage` (not yet available) | TEST-06, TEST-07 |
| Mutation | Test quality: whether tests fail when code changes | planned; scope per foundation.md 5.3 | `mutation` (not yet available) | TEST-08 |
| Fuzz | Adversarial parsing and change sequences | planned; scope in [ADR 0026](adr/0026-nightly-fuzz-miri-and-concurrency-proof-scope.md) | `fuzz` (not yet available) | SEC-04, SEC-05 |
| Miri and concurrency proofs | Undefined behaviour and synchronisation schedules | planned; scope in [ADR 0026](adr/0026-nightly-fuzz-miri-and-concurrency-proof-scope.md) | | TEST-09 |
| Formal verify | Apalache on the Quint invariants (scheduled lane) | planned | `formal-verify` (not yet available) | FORMAL-10 |

Each `xtask` subcommand that is not yet available prints `not available yet` and names the task
that introduces it.

## Naming and structure

Every test function is named `should_<outcome>_when_<condition>`, and its body has `// given`,
`// when` and `// then` comments in that order. The architecture checker enforces both:

- `ARCH-test-name` rejects a test whose name does not match, and rejects a macro that hides a
  test. It recognises `proptest!` blocks, so property tests follow the same name rule.
- `ARCH-test-structure` rejects a test whose body lacks the three markers, or has them out of
  order.

```rust
#[test]
fn should_return_not_found_when_directory_lacks_index() {
    // given
    // ...

    // when
    // ...

    // then
    // ...
}
```

Test support uses real objects over handwritten fakes from `preview-testkit`. No mocking
framework is used.

## Waiting and sleeping

Deterministic tests never sleep and never poll. They synchronise with injected clocks
(`FakeClock`), scripted sources (`ScriptedChangeSource`), channels, barriers or readiness
signals. `ARCH-test-sleep` rejects `std::thread::sleep`, timed parks and receives, and tokio's
timers in test code.

The one sanctioned wait is `preview_testkit::poll::wait_until(deadline, reason, condition)`.
It re-checks a condition at a short interval until a deadline and returns a `Timeout` that names
what was awaited. It is for tests that observe the real operating system, such as OS-watcher
events, and each such test states its reason. The deadline only bounds how long a failing test
hangs; timing is never the oracle. The helper is the only file the sleep rule exempts,
`crates/preview-testkit/src/poll.rs`. No deterministic test uses it.

## Contract harnesses

A port with more than one implementation gets a generic harness (foundation.md 5.1). The
harness runs the same named cases against a fake and against each real adapter, so the fake
cannot be looser than production.

The `SiteContent` port is the worked example:

- `preview_testkit::contracts::SiteContentFactory` builds an implementation from a declarative
  `SiteTree` (directories, files, links, special entries, files outside the root).
- `SITE_CONTENT_CASES` lists the cases by name (for example `in_root_symlink`,
  `symlink_middle`, `non_regular`, `concurrent_calls`).
- `site_content_contract::<F>()` panics with the full report if any case fails.
  `check_site_content_contract::<F>()` returns a `ContractReport` instead, for negative controls.
  The report lists the failed case names and the expected and actual outcome of each.

### Adding an adapter to a contract

1. Implement `SiteContentFactory` for the adapter under test in the adapter's own tests, mapping
   each `SiteEntry` onto the real storage (see `TempSite` in
   `crates/preview-adapter-fs/tests/site_content_contract.rs`).
2. Add a `#[test]` named `should_pass_contract_when_...` that calls `site_content_contract` and
   asserts `report.cases_run() == SITE_CONTENT_CASES`.
3. Add a negative control: a deliberately broken implementation that breaks one named case, with
   a test that calls `check_site_content_contract` and asserts `failed_cases()` names exactly that
   case. Two examples exist: `BrokenFollowsSymlinks` in
   `crates/preview-testkit/tests/site_content_fake.rs` (fake) and `ForbiddenToContents` in
   `crates/preview-adapter-fs/tests/site_content_contract.rs` (real adapter). Both fail the same
   case, `in_root_symlink`, so the harness is shown to catch a broken implementation on either side.

Not every clause is portable, so the harness has no case for it. A superuser holding DAC
capabilities, as in a container, reads a mode-000 directory, so a real adapter cannot be made to
refuse with `Forbidden(PermissionDenied)` in a test. That variant is covered only by the fake's
own unit test, `should_classify_leaf_when_entry_is_special_or_denied_or_broken` in
`crates/preview-testkit/src/site.rs`, not by the shared contract.

## Race tests

`crates/preview-adapter-fs/tests/symlink_race.rs` swaps a real directory for a symbolic link
outside the root while a test thread opens a file through it. The loops are bounded by iteration
counts, never by time. A negative control, `should_return_outside_bytes_when_path_is_opened_naively_during_swap`,
opens the path the naive way and shows the race is real, so a passing capability test means
something.

## Property tests and seeds

Property tests use `proptest`. Rules, from foundation.md 5.2 and the header of
`crates/preview-domain/tests/properties.rs`:

- Generate values that reach the boundaries (dots, escapes, separators), not only arbitrary bytes.
  Keep the exhaustive deterministic tests as the floor.
- Do not set `cases` in code. The `ci` nextest profile sets `PROPTEST_CASES = "1024"`; a developer
  may set it the same way. The default is proptest's own.
- A black-box property in `crates/<crate>/tests/` persists failures to
  `proptest-regressions/<file>.txt` with `FileFailurePersistence::Direct`. Inline properties use
  proptest's default persistence. Both write under `crates/<crate>/proptest-regressions/`.
- A found counterexample is committed as a seed. The next run replays it first, so the failure
  stays a regression test. The directory is committed and never ignored. A fix that makes a seed
  obsolete removes that one line, not the whole file.

`crates/preview-domain/proptest-regressions/` holds only a `.gitkeep` today: no counterexample
has been found yet.

## nextest profiles and groups

`.config/nextest.toml` defines the configuration. Flaky reruns count as failures.

- `default` (used by `cargo nextest run`): `retries = 0`, a 30-second slow-test period that
  terminates after four periods.
- `ci` (used by `cargo xtask test`): the same, with `fail-fast = false`, `PROPTEST_CASES = "1024"`
  and a JUnit report at `junit.xml`.
- `os-watcher` test group: `max-threads = 4`. Tests whose name starts with `os_watcher::` join it
  automatically through a profile override. Nothing uses it yet; the watcher adds the first tests.
  Use the group only for tests that share a real OS resource, and prefer dynamic ports and
  tempdirs over serialising the whole suite.

## Commands

Run the commands from the repository root. The quick loop, then the full gate before a commit:

```bash
cargo xtask quick          # formatter check, architecture, Clippy, unit/component tests, doctests
cargo xtask check          # the full blocking gate, run before every commit
```

`quick` is not `check`. `check` runs the `quick` stages other than the tests, then `test`
(the `ci` profile and the doctests), then `security`. A stage whose tool is missing is reported as
`UNAVAILABLE` with the install instruction and a non-success exit; it never counts as passed.

Per level:

```bash
cargo fmt --all --check                                   # fmt-check: formatting
cargo clippy --locked --workspace --all-targets -- -D warnings   # clippy
cargo nextest run --locked --workspace                    # unit-tests: the default profile
cargo test --locked --workspace --doc                     # doctests (nextest does not run them)
cargo xtask architecture                                  # rules, fixtures, test hygiene
cargo xtask test                                          # unit-tests under the ci profile, then doctests
cargo xtask security                                      # deny, secrets, workflows, action pins
cargo xtask fmt                                           # apply rustfmt (developer convenience)
```

To select a crate or a test by name, use nextest's package and name-filter options with the
`cargo nextest run` command above. That is an ad-hoc form, not a catalog entry.

Tooling:

```bash
cargo xtask list-checks    # the machine-readable catalog: id, level, command, owner, in_check
cargo xtask doctor         # which pinned tools are present and at the right version
cargo xtask install-tools  # install the pinned tools that doctor reports missing
```

Planned, printing `not available yet` until their task lands:

```bash
cargo xtask conformance    # FORMAL-01 to FORMAL-09
cargo xtask coverage       # TEST-06, TEST-07
cargo xtask mutation       # TEST-08
cargo xtask fuzz           # SEC-04
cargo xtask formal-verify  # FORMAL-10
cargo xtask release-check  # OPS-05
```

## Checks the gate runs

`cargo xtask list-checks` marks the stages `check` runs with `"in_check": true`. Today those are
`fmt-check`, `clippy`, `unit-tests`, `doctests`, `architecture` (with its `ARCH-*` rules),
`security`, `secrets`, `workflows` and `action-pins`. `check` runs each stage once; a stage that
is in the catalog but not yet implemented is absent from the run, not skipped.
