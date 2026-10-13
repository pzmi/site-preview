# Verification scope

What verifies which property, what each check does not cover, and which tools are deliberately
not used. Decisions: D25 and [ADR 0026](adr/0026-nightly-fuzz-miri-and-concurrency-proof-scope.md)
(nightly, Miri, concurrency proofs), [ADR 0071](adr/0071-miri-lane-for-domain-and-application.md)
(the Miri lane). The test levels themselves are in [`testing.md`](testing.md); the models in
[`formal-models.md`](formal-models.md).

## What verifies what

| Property | Verified by | Where it runs | Does not cover |
|:--|:--|:--|:--|
| Pure domain rules (path decoding, eligibility, Host and Origin, debounce, change sets, injection, lifecycle and watcher-health machines) | Unit tests and `proptest` properties in `preview-domain`; 1024 cases per property in `unit-tests` (`PROPTEST_CASES`, `tools/xtask/src/test.rs`) | `check` | Inputs a random generator does not reach: that is fuzzing's job |
| Application orchestration (`ChangeCoordinator`, `ReloadState`, request resolution) | Unit and component tests over testkit fakes; properties in `preview-application` | `check` | Real file system, sockets, timing |
| Adapter behaviour (confined opens under a symlink race, OS watcher, HTTP routing, SSE, streaming injection) | Integration, contract, CLI black-box and E2E tests over real tempdirs and sockets | `check` | Interleavings no test schedules; see "Concurrency" |
| Lifecycle, watcher and reload protocol (no lost change, one listener owner, monotone revisions, resources released) | Quint models; the model's traces replayed against the real code and compared after every step; negative controls; simulation with a fresh seed | `check` (`conformance`), nightly in `formal.yml` | Anything the model abstracts away; the "Scope" section of `formal-models.md` lists it |
| The same invariants, for every nondeterministic choice up to a depth | Apalache bounded verification (`quint verify`) of each `allInvariants`, plus a larger multi-seed simulation | Scheduled `formal-verify` job (`cargo xtask formal-verify`), not in `check` | Depth beyond the bounds in `BOUNDED` (2 to 4 steps); liveness |
| No undefined behaviour in the pure crates | Miri over the `preview-domain` and `preview-application` tests | Scheduled `miri` job (`cargo xtask formal-verify --miri`), not in `check` | See "Miri" below |
| Parsers and the change pipeline survive adversarial input | `cargo-fuzz` targets `decode_path`, `rewrite_html`, `host_header` and `change_sequence`, each with committed seeds and asserted properties ([`security.md`](security.md#fuzzing)) | Nightly `fuzz.yml`; `cargo xtask fuzz <target>` locally; not in `check` | Behaviour with no oracle in the target; code outside the four targets |
| Tests exercise the code | LLVM line coverage floors (`coverage-floor.toml`, overall and per crate) and at least 95% of changed instrumented lines | `check` (`coverage`, `changed-lines`) | Whether the assertions are strong enough |
| Tests would fail if the code were wrong | Mutation testing, `cargo xtask mutation` (TEST-08; scope per foundation.md section 5.3) | Scheduled, not in `check` | Equivalent mutants; paths the selected tests do not execute |
| Crate layering, pure domain, no `unsafe`, test naming and hygiene, `fuzz_only` isolation | The architecture checker and its negative fixtures; `unsafe_code = "forbid"` in the workspace lints | `quick`, `check` | Semantics: it reads structure, not behaviour |
| Supply chain and workflow safety | `cargo deny`, gitleaks, `actionlint`, the action-pin check | `check` (`security`) | Vulnerabilities not yet in the advisory database |

## Miri

`cargo xtask formal-verify --miri` runs `cargo +<nightly> miri nextest run --profile miri
-p preview-domain -p preview-application` on the dated nightly of `tools/versions.toml`, from
the scheduled `miri` job of `.github/workflows/formal.yml` (and on demand). The workspace
forbids `unsafe` (`unsafe_code = "forbid"`), so Miri's value here is checking the dependencies'
`unsafe` as the tests exercise it (`proptest`, `regex`, `tokio::sync`, `percent-encoding`) and
catching a change in the compiler or standard library that exposes a latent problem.

Setup, once per machine: `cargo xtask install-tools cargo-nextest nightly`, then
`rustup +nightly-2026-10-01 component add miri rust-src` and `cargo +nightly-2026-10-01 miri
setup`. A missing component fails with that command; it is never a skip.

What the lane changes so that the tests can run under an interpreter that is about a thousand
times slower than native code and refuses host access (isolation stays on):

| Change | Why | Where |
|:--|:--|:--|
| `PROPTEST_CASES=4` (the caller's value wins), forwarded with `-Zmiri-env-forward` | Isolation hides the host environment; without forwarding, proptest ran 256 cases and two tests timed out | `miri_env` in `tools/xtask/src/formal_verify.rs` |
| `PROPTEST_DISABLE_FAILURE_PERSISTENCE=true`, forwarded | Proptest's persistence calls `getcwd`, which isolation refuses; the committed `proptest-regressions/` seeds are replayed by `unit-tests`, not here | same |
| A `miri` nextest profile with a 300 s slow-test period (four periods) | The 30 s of the other profiles is far too short | `.config/nextest.toml` |
| Shorter generated inputs under `cfg!(miri)`: `HOT_EVENTS` 70 instead of 100 (still past the 64-path cap) in `change_set.rs`, `SCHEDULE_STEPS` 30 instead of 120 in `debounce.rs`, `INPUT_LENGTH` 3 instead of 4 symbols in the exhaustive enumeration of `path.rs` | These tests are heavy per case or exhaustive, and the case count does not shorten them | `crates/preview-domain/src/` |
| **Excluded:** the `compile_fail` test binaries (`-E 'not binary(compile_fail)'`) | `trybuild` starts `cargo` and `rustc` and reads the file system; Miri cannot run either. They check that invalid API use does not compile, which is not a property of executed code, and `integration-tests` runs them | `MIRI_FILTER` |

No test is `#[ignore]`d, and no `#[allow]` was added.

Not covered: the adapter crates (file system, sockets, the OS watcher, a real clock), the
composition root and the CLI, which Miri cannot run or which add nothing without `unsafe` of
their own; four proptest cases per property, so a rare input only a long run generates is left
to the 1024-case `unit-tests` lane and the fuzz targets.

### Result

Measured on 2026-10-13 with `cargo xtask formal-verify --miri` on `nightly-2026-10-01`
(Miri from the `miri` component of that nightly), cargo-nextest 0.9.148, a shared 4-core
sandbox: 249 tests run, 249 passed, none skipped, no undefined behaviour reported; 809 s of
test time and 834 s for the stage including the build. The slowest test is
`change_set::tests::should_be_hot_only_when_every_change_is_hot_and_within_cap`, over 300 s
(one slow period of the `miri` profile). The 250th test of the two crates is the excluded
`compile_fail` binary. `cargo miri nextest` works with the pinned nextest, so the
`cargo miri test` fallback is not used. The first runs found no UB either; they failed only on
the environment and size problems the table above lists (isolation hiding `PROPTEST_CASES`,
the `getcwd` call of proptest's persistence, and four tests over the 120 s default timeout).
The scheduled job runs on a hosted runner; raise a bound above only after measuring it here.

## Concurrency: Loom, Shuttle and Kani are not used

D25 and ADR 0026 do not adopt them. Each checks a thing the code does not have:

- **Loom** explores the interleavings of code that implements its own synchronization with
  `std::sync` atomics or `UnsafeCell`, by replacing those primitives with instrumented ones. The
  workspace has no such code: it forbids `unsafe`, and every shared state goes through
  `tokio::sync` (`watch` for the reload state and shutdown, `Semaphore` for the connection and
  injection budgets, `mpsc` and `Notify` in the watcher bridge), or a `std::sync::Mutex` or
  `AtomicBool` used as a flag or a set behind a short critical section (`warnings.rs`,
  `output.rs`, the bridge). Those are the library's primitives, tested by their authors; what
  is ours is the protocol around them, and that is what the Quint models and their conformance
  replay check (lifecycle, watcher, reload).
- **Shuttle** randomises schedules of larger concurrent programs. Our concurrency is the
  protocol above, with its own deterministic drivers; the symlink race and the OS watcher are
  integration tests against the real thing, which Shuttle cannot model.
- **Kani** proves bounded properties of Rust functions with a model checker. The pure
  algorithms that would suit it (path decoding, eligibility, Host policy, debounce) are covered
  by exhaustive and property tests, fuzz targets with oracles, and Miri; a proof harness would
  restate them at the cost of a second toolchain.

Adopt each when its trigger holds, in a task that adds the dependency (`cargo add`), the xtask
subcommand and the scheduled job, and updates this file and an ADR:

| Tool | Trigger |
|:--|:--|
| Loom | Code appears that implements its own synchronization primitive or lock-free structure over `std::sync::atomic` or `unsafe`, or a data structure shared across threads whose correctness depends on atomic orderings. |
| Shuttle | A concurrent component with more moving parts than the models can state (for example more than one stage running its own tasks over shared state) whose bug would be a schedule, and a Quint model of it would be a restatement of the code. |
| Kani | A bounded pure algorithm whose correctness is arithmetic or index reasoning that property tests and the fuzz oracle cannot settle (for example an offset computation in the rewriter), or any `unsafe` block, which the lints currently forbid. |

An escaped concurrency bug found by any other check also opens the question: record it here
with the check that missed it and the tool that would have caught it.
