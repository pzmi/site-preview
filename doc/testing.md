# Testing

How site-preview's tests are organised, what each level proves, where it lives, and which
command runs it. The rules come from [`foundation.md`](foundation.md) section 5 (levels and test
quality) and section 9 (commands); the test locations are decided in
[ADR 0016](adr/0016-test-locations.md). `cargo xtask list-checks` prints the machine-readable
catalog of every check, its level, its command and its owner task. Every command in this file
is one of its entries, or a planned one it lists as not yet available. How a change is gated and
integrated is in [`sdlc.md`](sdlc.md); the CI checks that must be green on `main` are in
[`required-checks.md`](required-checks.md).

## Levels

"In place" means the level runs today. "Planned" names the task that adds it.

| Level | What it proves | Where it lives | Runs under | Status |
|:--|:--|:--|:--|:--|
| Unit | Pure logic and boundary examples | `#[cfg(test)] mod tests` beside the module, e.g. `crates/preview-domain/src/`, `crates/preview-adapter-fs/src/` | `unit-tests` (`cargo nextest run`) | in place |
| Doctest | Examples in `///` and `//!` compile and pass | Doc comments; e.g. the `wait_until` example in `crates/preview-testkit/src/poll.rs` | `doctests` | in place |
| Property | Invariants over generated values, with persisted counterexamples | `proptest!` inside `crates/preview-domain/src/` and `crates/preview-application/src/`; black-box properties in `crates/preview-domain/tests/properties.rs` | `unit-tests` and `integration-tests`; the `ci` profile raises the case count | in place |
| Component | Real application code over testkit fakes, no external services | `crates/preview-application/tests/`, `crates/preview-adapter-http/tests/` (router, files, directory, host, client and the SSE stream in `sse.rs` and `stream.rs`, over `InMemorySite`) | `integration-tests` | in place |
| Contract | Each fake and each real adapter of a port meets one shared harness | `crates/preview-testkit/src/contracts/`; runs in `crates/preview-testkit/tests/{site_content_fake,change_source_fake}.rs` and `crates/preview-adapter-fs/tests/{site_content_contract,change_source_contract}.rs` | `unit-tests` and `integration-tests` | in place for `SiteContent` and `ChangeSource` (TEST-05) |
| Architecture | Crate, dependency, lint and test-hygiene rules, and each rule's negative control | `tools/xtask/src/architecture/`; fixtures in `tests/architecture-valid/` and `tests/architecture-invalid/`; `tools/xtask/tests/architecture_fixtures.rs` | `architecture` (inside `quick` and `check`) | in place |
| Security fixtures | The dependency policy, secret scanning and workflow lint reject a seeded bad input | `tools/xtask/tests/security_fixtures.rs` | `integration-tests` | in place |
| Integration | Real tempdir filesystem, threads, the root capability under a concurrent symlink swap | `crates/preview-adapter-fs/tests/` (`root.rs`, `capability.rs`, `symlink_race.rs`) | `integration-tests` | in place; the OS watcher tests (`os_watcher::`, `crates/preview-adapter-fs/src/watcher/os_watcher_tests.rs`) run in the `os-watcher` nextest group |
| CLI black box | The built binary's flags, output and exit codes through `assert_cmd` | `crates/preview-cli/tests/cli_meta.rs` (version, help), `serve_slice.rs` (the served slice), `flags.rs` (port, host and `--allow-remote` flags, warnings, exit codes) | `integration-tests` | in place |
| E2E | The built binary's HTTP and security behaviour over a real socket against a copy of the hostile fixture site, and that serving leaves the tree unchanged | `crates/preview-cli/tests/http_security.rs`, with `crates/preview-cli/tests/support/mod.rs` | `integration-tests` | in place (E2E-01); injection and SSE black-box suite `injection_sse.rs` (E2E-02); watcher suite `watcher.rs` and `watch_reload.rs` (E2E-03) |
| Browser E2E | The real binary under `--json --port 0` driven by Chromium through Playwright (`@playwright/test` 1.56.1, locked): harness fixtures `site` and `preview`, `startPreview`, `trackNavigations` | `tests/e2e/browser/` (`support/preview.ts`, `playwright.config.ts`, `*.spec.ts`) | `browser` (`cargo xtask test --only browser`) | harness and smoke spec in place (E2E-04); load, edit, assets and widths spec `reload.spec.ts` (E2E-05); reconnect and restart recovery spec `reconnect.spec.ts` (E2E-06); hot-reload spec `hot.spec.ts` for CSS swap, asset cache-bust, script reload with scroll restore and the accept API (E2E-09); agent workflow spec `agent-workflow.spec.ts` (E2E-07: plays the agent of `AGENTS.md` and fails when its commands or ready example drift from behaviour) |
| Compile-fail | An invalid API use does not compile (`trybuild`) | `crates/preview-domain/tests/compile_fail.rs`, `crates/preview-adapter-fs/tests/compile_fail.rs` | `integration-tests` | in place (ARCH-08) |
| Client JavaScript | The reload client's `node:test` suite, with no npm dependency | `crates/preview-adapter-http/assets/client.test.mjs` | `client-js` (`node --test crates/*/assets/*.test.mjs`) | in place (HTTP-09) |
| Conformance | Quint models: typecheck, named scenarios and invariant simulation; model traces replayed against the real code, compared after every step | `formal/lifecycle.qnt`, `formal/watcher.qnt`, `formal/reload.qnt`, `formal/scenarios.qnt`, `formal/lifecycle_traces.qnt`, `formal/watcher_traces.qnt`, `formal/reload_traces.qnt` (Quint 0.33.0 locked in `formal/package-lock.json`, [ADR 0052](adr/0052-quint-toolchain-and-lifecycle-model.md)); `crates/preview-conformance`: the ITF replay core ([ADR 0053](adr/0053-conformance-harness.md)), the lifecycle driver over the composition `Server` (`src/drivers/lifecycle.rs`, `tests/lifecycle_conformance.rs`, [ADR 0056](adr/0056-lifecycle-conformance-driver.md)), the reload driver over the real `ReloadState` and SSE handler in process (`src/drivers/reload.rs`, `tests/reload_conformance.rs`, [ADR 0057](adr/0057-reload-conformance-driver.md)) and the watcher driver over the `ChangeCoordinator` (`src/drivers/watcher.rs`, `tests/watcher_conformance.rs`, [ADR 0058](adr/0058-watcher-conformance-driver.md)); negative controls, faulty implementations behind the drivers' seams, each caught at an asserted trace, step, action and field (`tests/negative_controls.rs`, [ADR 0059](adr/0059-conformance-negative-controls.md)); named traces in `formal/traces/regression/{lifecycle,watcher,reload}-*.itf.json`, regenerated from `formal/traces/regression/manifest.toml`; seeded ones generated into `formal/traces/generated/` with `manifest.json` | `formal-typecheck`, `formal-scenarios`, `formal-simulate`, `formal-freshness`, `formal-generate`, `conformance-replay`, `conformance-controls` (`cargo xtask conformance`, in `check`; `unit-tests` excludes `preview-conformance`, [ADR 0060](adr/0060-conformance-stage-and-trace-corpus.md)) | in place (FORMAL-01 to FORMAL-09); models, stimuli, observations, invariants, scope and promoting a counterexample in [`formal-models.md`](formal-models.md) |
| Coverage | Line coverage floor and changed-line coverage | `cargo llvm-cov nextest --workspace`: every in-process suite including conformance and xtask, floors in `coverage-floor.toml` (the overall floor over `crates/` only, `xtask` report only, [ADR 0066](adr/0066-coverage-overall-floor-over-crates-only.md)), LCOV at `target/coverage/lcov.info` ([ADR 0018](adr/0018-coverage-gates.md), [ADR 0062](adr/0062-coverage-stage-scope-and-floors.md)); ≥95% of the instrumented lines changed in `crates/**/src/**/*.rs` since the merge-base with `origin/main` (else `HEAD~1`) covered ([ADR 0064](adr/0064-changed-line-gate-in-check.md)) | `coverage` and `changed-lines` (`cargo xtask coverage [--changed-since [REV]]`, both in `check`) | in place (TEST-06, TEST-07) |
| Mutation | Test quality: whether tests fail when code changes | planned; scope per foundation.md 5.3 | `mutation` (not yet available) | TEST-08 |
| Fuzz | Adversarial parsing and change sequences | `fuzz/` (own workspace, pinned nightly, `fuzz_only`; [ADR 0026](adr/0026-nightly-fuzz-miri-and-concurrency-proof-scope.md), [ADR 0063](adr/0063-fuzz-crate-and-architecture-fuzz-only.md), [ADR 0065](adr/0065-fuzz-targets-beyond-the-domain-and-fuzz-lockfile-audit.md)): targets `decode_path`, `rewrite_html`, `host_header` and `change_sequence` (`fuzz/fuzz_targets/`), each with committed seeds in `fuzz/seeds/<target>/`; properties and triage in [`security.md`](security.md#fuzzing) | `fuzz` (`cargo xtask fuzz <target> --time 60`; not in `check`); nightly in `.github/workflows/fuzz.yml`; `fuzz/Cargo.lock` audited by the `deny-fuzz` stage | in place (SEC-04, SEC-05) |
| Miri and concurrency proofs | Undefined behaviour in the pure crates; the scope of concurrency proofs | `cargo miri nextest` over `preview-domain` and `preview-application` on the pinned nightly ([ADR 0071](adr/0071-miri-lane-for-domain-and-application.md)); Loom, Shuttle and Kani are not used, with triggers, in [`verification-scope.md`](verification-scope.md) | `formal-verify-miri` (`cargo xtask formal-verify --miri`; `formal.yml` job `miri`; not in `check`) | in place (TEST-09) |
| Formal verify | Apalache bounded verification of the Quint invariants and a multi-seed simulation (scheduled lane, not in `check`) | `formal/*.qnt`; Apalache 0.62.1 pinned in `tools/versions.toml`, JVM 17 or newer, [ADR 0061](adr/0061-apalache-formal-verify-scheduled-lane.md) | `formal-verify-bounded`, `formal-verify-simulate` (`cargo xtask formal-verify`; `formal.yml` job `formal-verify`) | in place (FORMAL-10); baseline in [`formal-models.md`](formal-models.md) |

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
- Do not set `cases` in code. `cargo xtask test` sets `PROPTEST_CASES=1024` for its `ci`-profile
  stage (nextest has no profile-level env table); a developer may raise it by exporting a larger
  `PROPTEST_CASES`, which the stage keeps (a value under 1024 is raised to 1024). Outside xtask
  the default is proptest's own.
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
- `ci` (used by `cargo xtask test`): the same, with `fail-fast = false` and a JUnit report at `junit.xml`.
  The xtask stage also sets `PROPTEST_CASES` (1024, or the developer's larger value) and fails if
  nextest prints "ignoring unknown configuration key".
- `miri` (used by `cargo xtask formal-verify --miri`): `retries = 0`, `fail-fast = false`, and a
  300-second slow-test period that terminates after four periods, because Miri is about a
  thousand times slower than native code ([`verification-scope.md`](verification-scope.md)).
- `os-watcher` test group: `max-threads = 4`. Tests whose name starts with `os_watcher::` join it
  automatically through a profile override; the OS watcher tests are its users.
  Use the group only for tests that share a real OS resource, and prefer dynamic ports and
  tempdirs over serialising the whole suite.

## Commands

Run the commands from the repository root. The quick loop, then the full gate before a commit:

```bash
cargo xtask quick          # formatter check, architecture, Clippy, unit/component tests, doctests
cargo xtask check          # the full blocking gate, run before every commit
```

`quick` is not `check`. `check` runs the `quick` stages other than the tests, then `test`
(the `ci` profile and the doctests), the `client-js` node tests, the `browser` Playwright specs, then the seven
conformance stages (`formal-typecheck` to `conformance-controls`), then `coverage` and the `changed-lines` gate
over its report, then the security stages (`deny`, `deny-fuzz`, `secrets`, `workflows`, `action-pins`; `workflows` needs both actionlint and shellcheck, because actionlint skips the `run:` script checks silently without shellcheck). A stage whose tool is missing is reported as
`UNAVAILABLE` with the install instruction and a non-success exit; it never counts as passed.

Per level:

```bash
cargo fmt --all --check                                   # fmt-check: formatting
rustfmt --check --edition 2024 fuzz/fuzz_targets/*.rs     # fuzz-fmt: the fuzz targets, outside the workspace
cargo clippy --locked --workspace --all-targets -- -D warnings   # clippy
cargo nextest run --workspace --locked --profile ci -E 'not kind(test)'   # unit-tests: tests inside libraries and binaries
cargo nextest run --workspace --locked --profile ci -E 'kind(test)'       # integration-tests: every tests/*.rs binary (CI runs them as two jobs)
cargo test --locked --workspace --doc                     # doctests (nextest does not run them)
cargo xtask architecture                                  # rules, fixtures, test hygiene
cargo xtask test                                          # unit-tests and integration-tests under the ci profile, doctests, client-js, then browser
cargo xtask security                                      # deny, deny-fuzz, secrets, workflows, action pins
node --test crates/*/assets/*.test.mjs                    # client-js: the reload client's tests
cargo xtask test --only browser                           # browser: build, npm ci, npx playwright test (needs Node 22 and Chromium 1194)
cargo xtask fmt                                           # apply rustfmt to the workspace and the fuzz targets
```

To select a crate or a test by name, use nextest's package and name-filter options with the
`cargo nextest run` command above. That is an ad-hoc form, not a catalog entry.

Tooling:

```bash
cargo xtask list-checks    # the machine-readable catalog: id, level, command, owner, in_check
cargo xtask doctor         # which pinned tools are present and at the right version
cargo xtask install-tools  # install the pinned tools that doctor reports missing
```

Quint models (needs Node 22 and `npm ci --prefix formal`; a missing one fails with that hint):

```bash
cargo xtask conformance --typecheck   # formal-typecheck: quint typecheck formal/*.qnt
cargo xtask conformance               # all seven stages (formal-models.md): scenarios, simulation, freshness, generation, replay, controls; QUINT_SEED=<seed> repeats a run
cargo nextest run -p preview-conformance --test lifecycle_conformance   # replay alone: the named traces, then 30 fresh seeded ones (seed printed; QUINT_SEED=<seed> repeats it)
cargo nextest run -p preview-conformance --test negative_controls       # faulty implementations, each caught at its exact step
```

The `unit-tests` and `integration-tests` stages and `quick` exclude `preview-conformance`; `check` runs it in
`conformance-replay` and `conformance-controls` (ADR 0060). Each committed trace is regenerated
from `formal/traces/regression/manifest.toml`, and the `formal-freshness` stage fails with
`stale regression trace <file>` when one differs. [`formal-models.md`](formal-models.md) says how
to promote a counterexample.

```bash
cargo xtask coverage                          # instrumented workspace run, LCOV, floors (TEST-06)
cargo xtask coverage --changed-since          # changed-line gate since the merge-base with origin/main, else HEAD~1 (TEST-07)
cargo xtask coverage --changed-since HEAD~3   # the same against another revision
```

`coverage` needs `cargo-llvm-cov` and `cargo-nextest` (`cargo xtask install-tools`) and, because
`preview-conformance` generates its own traces there, `npm ci --prefix formal`. It prints the
overall and per-crate line coverage against `coverage-floor.toml` and exits 1 below a floor
([ADR 0062](adr/0062-coverage-stage-scope-and-floors.md)). The overall figure counts the files
under `crates/` only; `tools/xtask` is measured and printed on its own row, report only, because
its tool-probe tests take different branches on machines with different tools installed
([ADR 0066](adr/0066-coverage-overall-floor-over-crates-only.md)). CI installs `cargo-llvm-cov`
with the other pinned tools; without it `coverage` is `UNAVAILABLE` and `changed-lines` fails.

`--changed-since` gates only the change: the lines `git diff -U0` adds or modifies in
`crates/**/src/**/*.rs` between the merge-base of REV and `HEAD` and the working tree, plus
every untracked file there. At least 95% of those with an LCOV `DA:` record must be covered; it
prints each uncovered `file:line`, and counts apart the changed lines with no record (comments,
signatures), which stay out of the ratio. It reuses `target/coverage/lcov.info` only while
`target/coverage/lcov.stamp` matches the content of the tree (every tracked and untracked, non-ignored file), and
measures again otherwise. In `check`, the `changed-lines` stage reads the report the `coverage`
stage just wrote and fails rather than measure twice if it no longer matches
([ADR 0064](adr/0064-changed-line-gate-in-check.md)).
`cargo xtask formal-verify` is the scheduled lane (not in `check`; `formal.yml`, nightly): it needs
Node and Quint, a JVM 17 or newer, and the pinned Apalache release, and fails with the fix when one
is missing ([ADR 0061](adr/0061-apalache-formal-verify-scheduled-lane.md)).

`cargo xtask release-check [--target T]` (OPS-05, [ADR 0076](adr/0076-release-check-archive-and-notices.md);
not in `check`, run by the `artifact-smoke` CI job and by the release workflow on each native
runner) builds the release binary for `T` (default: the host), packages
`site-preview-<T>.tar.gz`, extracts it into a fresh temporary directory and tests the extracted
binary under `env -i PATH=/usr/bin:/bin`. It needs `cargo-about` (`cargo xtask install-tools
cargo-about`) and the Rust target (`rustup target add T`; the Linux release targets are musl
and may need `musl-tools`). Logs of the run are in `target/release-check/<T>/logs/`.
`release-check --target T --archive PATH` skips the build and smokes an existing archive named
`site-preview-<T>.tar.gz` (the release workflow's `verify` job), and `cargo xtask sbom` writes one
CycloneDX SBOM per release target with the pinned `cargo-cyclonedx`
([`release.md`](release.md), [ADR 0078](adr/0078-release-workflow.md)).

## Checks the gate runs

`cargo xtask list-checks` marks the stages `check` runs with `"in_check": true`. Today those are
`fmt-check`, `fuzz-fmt`, `clippy`, `docs` (the documentation against the code, [ADR 0077](adr/0077-documentation-consistency-stage.md)), `unit-tests`, `integration-tests`, `doctests`, `client-js`, `browser`, `architecture` (with its `ARCH-*` rules),
the seven `conformance` stages (`formal-typecheck` to `conformance-controls`), `coverage`, `changed-lines`, `deny`, `deny-fuzz`, `secrets`, `workflows` and `action-pins`. (`security` is the umbrella command for the last five, not a stage of its own.) `check` runs each stage once; a stage that
is in the catalog but not yet implemented is absent from the run, not skipped.

## CI jobs and failure evidence

`.github/workflows/ci.yml` runs one job per level, each calling `cargo xtask check --only <stages>`,
which runs the named stages through the same stage list as the full `check` (in the gate's order;
an unknown name exits 2 and lists the stage names). The same command reproduces a job locally
([ADR 0072](adr/0072-ci-job-topology-and-stage-parity.md)).

| Job | `check --only` stages | Tools it installs | Artifacts |
|:--|:--|:--|:--|
| `architecture` (Architecture and policy) | `architecture` | none | none |
| `static` (Static quality) | `fmt-check,fuzz-fmt,clippy,docs` | none | none |
| `fast-tests` (Fast tests) | `unit-tests,doctests,client-js` | `cargo-nextest`, Node 22 | JUnit on failure |
| `integration` (Integration) | `integration-tests` | `cargo-nextest`, `cargo-deny`, Gitleaks, actionlint, shellcheck (the xtask security fixtures run them) | JUnit on failure |
| `platform` (Platform, a matrix of four legs) | none: `cargo xtask test --only platform-tests` on Linux x86-64 and arm64 and macOS arm64 and x86-64 (ADR 0074) | `cargo-nextest` | JUnit on failure, per leg |
| `conformance` (Conformance (Quint)) | the seven `formal-*` and `conformance-*` stages | `cargo-nextest`, Quint (`npm ci --prefix formal`) on Node 22 | generated ITF traces and JUnit on failure |
| `coverage` (Coverage) | `coverage,changed-lines` | `cargo-nextest`, `cargo-llvm-cov`, `cargo-deny`, Gitleaks, actionlint, shellcheck, Quint | `lcov.info` always, JUnit on failure |
| `security` (Security) | `deny,deny-fuzz,secrets,workflows,action-pins` | `cargo-deny`, Gitleaks, actionlint, shellcheck | none |
| `browser` (Browser) | `browser` | Playwright and Chromium | Playwright `test-results/` (traces, screenshots) on failure |
| `artifact-smoke` (Artifact smoke) | none: `cargo xtask release-check --target x86_64-unknown-linux-musl` (ADR 0076) | `cargo-about`, the musl target and `musl-tools` | logs on failure |
| `required` (Required) | none | none | none |

`required` needs every other job and fails unless each finished `success` (a failed, cancelled or
skipped job fails it): it is the one result to watch. The unit test
`should_cover_each_in_check_stage_once_when_ci_jobs_are_read` fails when an `in_check` stage is in
no job or in two, so a stage cannot be dropped silently. Every job that runs Node installs 22.x with a SHA-pinned `actions/setup-node`.

Caches are keyed by job, OS, CPU architecture, `rust-toolchain.toml`, `Cargo.lock`, `tools/versions.toml` and the
job's tool list. Every run restores them; only a push to `main` saves one, so a pull request never
displaces `main`'s entries. `coverage` and `changed-lines` need the full history
(`fetch-depth: 0`); on a push the changed-line gate starts from `github.event.before`
([ADR 0069](adr/0069-tooling-review-fixes-fuzz-lint-and-gate-bases.md)).

`formal.yml` (nightly conformance with a fresh seed, plus the Apalache lane) and `security.yml`
(weekly) run on a schedule or by dispatch only; their blocking runs are the `conformance` and
`security` jobs above.
