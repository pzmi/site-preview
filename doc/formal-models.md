# Formal models and conformance

site-preview's lifecycle, file watcher and reload protocol are specified as Quint models in
`formal/`. Each model's traces are replayed against the real Rust code, and the two are
compared after every step (foundation.md section 6). This page covers what each model
covers, what the replay drives and observes, which invariants are checked, what is left out,
how the gate runs it, and how a counterexample becomes a regression trace.

## The rule: model first (D13)

The models were written after the code and fitted to it (ADR 0014). **From FORMAL-09 on, a
change to the protocol, state or concurrency of the lifecycle, the watcher or the reload
path starts in the model** (foundation.md section 6.2):

1. Name the external stimuli and the observable state.
2. Change the Quint model and its invariants. Keep safety and liveness separate, and never
   remove an invariant to get a green run.
3. Add or change the named runs in `formal/scenarios.qnt`, and the matching script in
   `formal/<model>_traces.qnt` in the same commit (an xtask unit test compares each script
   with its run, action by action).
4. Run `cargo xtask conformance` and check that the simulation passes and the changed traces
   are stale.
5. Change the conformance driver's stimulus map and observation (`crates/preview-conformance`).
6. Change the Rust code until the replay agrees, then commit the regenerated traces.

A model change without the matching driver and code change fails the replay. A code change
without a model change fails it too, wherever the model observes the difference.

## The models

| Model | Main module | Step relations | Driver | System under test |
|:--|:--|:--|:--|:--|
| `formal/lifecycle.qnt` | `lifecycle` | `step` (the domain machine), `serverStep` (the composition's `Server`), `serverStepMostlyBound` (the same, sampled so a bind fails one start in eight) | `drivers/lifecycle.rs` | the real `Server`: real sockets, the native watcher, real tasks ([ADR 0052](adr/0052-quint-toolchain-and-lifecycle-model.md), [ADR 0056](adr/0056-lifecycle-conformance-driver.md)) |
| `formal/watcher.qnt` | `watcher` | `step` (an OS lane, a pipeline lane, an ending lane) | `drivers/watcher.rs` | the real `ChangeCoordinator` (classification, change set, debouncer, health) behind the pipeline's own decision step `WatchDecisions`, with a real `ReloadState` and a fake clock ([ADR 0054](adr/0054-watcher-model.md), [ADR 0058](adr/0058-watcher-conformance-driver.md), [ADR 0070](adr/0070-conformance-harness-review-fixes.md)) |
| `formal/reload.qnt` | `reload` | `step` | `drivers/reload.rs` | the real `ReloadState` and SSE handler through the real router, in process with paused time ([ADR 0057](adr/0057-reload-conformance-driver.md)) |

### Stimuli

- **Lifecycle:** `startServer(bindOk, watcherOk)`, `refuseStart`, `request`, `openStream` and
  `stopServer`. A bind failure is a port the driver already holds. A refused start is a root
  that does not exist. A watcher failure is the `start_with_unavailable_watcher` seam.
- **Watcher:** the operating system's notifications (modify, create, delete, rename, atomic
  save, temp-file writes and `.git` changes, which the bridge drops before its channel as the
  code does (ADR 0075), directory deletion and recreation, root deletion in
  its two parts (the entries, then the root, which only the parent's watch reports: ADR 0073)
  and recreation,
  duplicates, reordering, queue overflow, runtime failure) come from the trace's own
  environment, and so does the start-up (every directory watched, `d` unwatchable, or `d`'s
  registration past the relisting cap). The driver hands the bridge's delivery to
  `WatchDecisions::receive` and a due deadline to `WatchDecisions::settle`, the step
  `WatchPipeline` runs; the reconcile it asks for reports what the adapter would from the
  model's environment, and the step turns it into `watcher_reconciled`,
  `watcher_failed(RegistrationFailed)` or, with the root gone, `watcher_failed(RootRemoved)`
  ([ADR 0067](adr/0067-root-removal-and-watcher-review-fixes.md)). A readiness the
  coordinator refuses is a divergence.
- **Reload:** `publish` and `publishHot` (synthetic change sets per kind), `connect`, `send`,
  `disconnect`, `userReload`, `receive`, `stopServer` and `startServer` (a new instance).

### Observations

Everything is read from the real code and projected into the model's terms:

- **Lifecycle:** the `Server` state (or the `StartupError` variant), whether the listener
  still completes a TCP handshake, whether any task is still owned, how many event streams
  the driver opened have not ended (after a stop, each is read until the server closes it),
  what `Server::stop` returned at a stop (the model has no drain, so `Drained`), and whether
  a request got a `200`.
- **Watcher:** the committed revision, `dirty` and the deadline, the health, the received
  batch's `recorded` count and reconcile request, the health events returned, and the
  revision published (full or not, its paths, the rescan flag).
- **Reload:** `up`, the committed revision, the latest change set, and per open connection
  the events its body yielded (name, instance, revision, kinds, `full`; a frame with more
  than one event is malformed, a failing body a divergence). The browser's own state is the
  model's, never observed (D16): the driver reads one model path, `browsers.*.doc`, to know
  whether a client reloaded, and is handed nothing else (`Driver::MODEL_READS`, ADR 0070).

Every projected field is compared: `crates/preview-conformance/tests/field_sensitivity.rs`
edits each one at one state of a committed trace and requires the replay to stop there
(ADR 0070).

### Invariants

`val allInvariants = and { ... }` in each model is the one list that `formal-simulate`,
`formal-generate` and the trace scripts check.

| Model | Invariant | Meaning |
|:--|:--|:--|
| lifecycle | `noResponsesWhenStopped` | Requests are answered and streams are open only while serving |
| lifecycle | `singleListenerOwner` | Exactly one owner of a bound listener, in exactly the states that hold one |
| lifecycle | `tasksReleasedAfterStop` | Tasks, streams and the watcher exist only while their state holds them; a stopped server holds nothing |
| lifecycle | `stopIdempotent` | A stop is accepted in every state; a second one changes and performs nothing |
| lifecycle | `failedStartupHoldsNothing` | A failed start-up leaves nothing allocated, and `Failed` is final |
| lifecycle | `shutdownInPlanOrder` | Shutdown effects run in plan.md section 9's order: listener, streams, watcher, tasks |
| watcher | `onlyEligibleTriggerReload` | Only notifications naming a served entry, or an overflow, make a reload pending; no change of an ineligible path ever takes a slot of the bridge's channel (ADR 0075) |
| watcher | `noLostEligibleChange` | A settled pipeline has published every change it saw; once `Watching`, nothing is pending |
| watcher | `boundedReloadsPerBurst` | One revision per burst, after a quiet window, however many duplicates it absorbs |
| watcher | `failureIsExplicit` | Live reload is claimed only with every eligible directory watched; a registration failure is claimed only while a directory is outstanding; each degradation and recovery is announced once, the one escalation to a root removal included |
| watcher | `overflowRescans` | An overflow leads to a full reconcile and a full revision |
| watcher | `noLiveClaimWithoutRoot` | Once the source has delivered a root's deletion or replacement, the watcher is degraded with `RootRemoved`, and never otherwise (ADR 0067) |
| reload | `monotonicRevision` | Revisions and instances only move forward, on the server and on every connection |
| reload | `noUncommittedClaim` | No event or baseline claims a state or change set that was not committed |
| reload | `atMostOneReloadPerChange` | A client reacts at most once per change, and never to an unchanged server |
| reload | `noHotApplyAcrossGap` | A client never applies in place across a gap of revisions |
| reload | `htmlChangeIsFull` | A change to an HTML or other page is a full reload everywhere |
| reload | `hotApplyAdvancesBaseline` | A hot apply moves the baseline to what it applied; otherwise only navigation moves it |
| reload | `drainedClientIsCurrent` | A client that handled every event is at the last state its connection announced |

Some conjuncts hold by the model's own construction, and only the replay gives them teeth:
they are model-internal consistency checks, kept because the replay compares the real code
against the state they constrain after every step.

- `overflowRescans`' `last.firedRescan implies last.fullDone`: `settle` sets `fullDone` from
  the same `reconciledFully` flag that decides whether it runs the full reconcile, so a model
  cannot violate it. The driver replays the reconcile from the real `settle` outcome's
  `rescan` flag, and the `NoRescanFlag` control shows a coordinator that drops it is caught.
- `boundedReloadsPerBurst`'s `revision <= cycles`, `cycles <= revision + 1` and
  `(cycles == revision + 1) == deb.dirty`: `cycles` is counted at the debouncer's own
  clean-to-dirty transitions, and a revision cleans it. The replay compares the real
  revision and `dirty` flag with the model's, and the `LeadingEdge` and
  `NoDuplicateSuppression` controls show a real coordinator that publishes more than once per
  burst is caught. The conjunct with teeth in the model is `last.fired implies last.firedQuiet`.

The reload model states its liveness separately: `reconnectObservesNewer` under `fairness`
and `channelAvailable` (`liveness`). Simulation does not check it, and neither does the
scheduled lane: see "Verification baseline" for what was tried.

### Scope: what is not modelled

- **Lifecycle:** the drain, and the forced exit at a second signal, inside `JoinTasks`
  (ADR 0045); the domain machine's stop-during-start-up states, unreachable through `Server`
  (ADR 0051), so `lifecycleMachineScenarios` runs under `quint test` but is not replayed.
- **Lifecycle:** `Failed` is terminal. It is the state of one `Server` whose bind failed:
  `Server::start` returns the error and no handle, so nothing can restart it, and the binary
  exits 1. A library caller that calls `Server::start` again gets a new machine from
  `Stopped` with nothing held (`failedStartupHoldsNothing`), whose states are those the
  model reaches from `init`; modelling it would only lengthen traces.
- **Lifecycle:** the binary's signal path is not modelled: `preview-cli` turning SIGINT and
  SIGTERM into a stop request, its exit codes, and the second signal's forced exit. The
  model's `stopServer` is `Server::stop`; the signal path is pinned by the CLI's process tests
  (`crates/preview-cli/tests/lifecycle.rs`) and composition's
  `should_stop_at_once_when_second_stop_arrives_during_drain`.
- **Watcher:** revision exhaustion, the 64-path cap, nesting below one directory level, a
  watcher that cannot be created, and `Closed` before shutdown. How a root removal is
  detected (the root descriptor's identity against the path) is the adapter's; the model has
  the outcome only. A `stat` of the root path that fails for any reason but a missing path
  (`EIO`, `ESTALE` and the like) is not a removal and changes no health, so it is outside the
  model too (ADR 0067, amended by the OPS-04 review).
- **Watcher, macOS is a superset outside the model:** the model's OS reports a change only
  through the watch of the directory that holds it, which is Linux's behaviour. macOS watches
  the root with one recursive FSEvents stream (ADR 0075), so a change in an eligible directory
  whose registration failed is still delivered there, where Linux is blind to it. That only
  adds notifications the model would excuse as unseen: an extra, harmless reload of a change
  the model counts as pending, and the health still follows the registration failure, as the
  model has it.
- **Watcher, what the replay drives and what it does not:** it drives the real coordinator,
  debouncer, classification, change set, health and reload state through the pipeline's own
  synchronous decision step (`WatchDecisions` in `watch_pipeline.rs`: receive, settle, the
  rescan rule and the reconcile's report). It does not drive the pipeline's async shell (the
  timer and its rounding, polling the source, the order cancellation, deadline, batch,
  `block_in_place`, the warnings), the adapter's reconcile or the bridge's `next`: the driver
  restates the last two from the model's environment, and composition's `watch_pipeline`
  tests and the adapter's tests cover all three. The lifecycle driver does not cover them
  either: its observation has no revision, health or reconcile.
- **Reload:** the composition's `instance_id` (the driver numbers instances itself), and the
  browser client (its decision is read from the model; the client's `node:test` suite and the
  browser E2E tests cover it).
- **Not modelled at all:** path-serving authorization, HTML injection and other pure functions,
  which are covered by property tests, fuzzing and E2E tests (foundation.md section 6.1).

## The trace corpus

- **`formal/traces/regression/`** (committed): one trace per named run, exported from the
  scripts in `formal/{lifecycle,watcher,reload}_traces.qnt` with seed `0x5ce1`, plus
  `itf-decoder-sample.itf.json` (FORMAL-04's decoder fixture). `manifest.toml` lists how each
  trace is regenerated: model, `main`, `init`, step, `max_steps`, seed and invariant.
- **`formal/traces/generated/`** (ignored by Git): 30 traces of 25 steps per driver family,
  written by every run with a fresh seed. `manifest.json` records the seed, the Quint version
  and per family the model, step relation, invariant, count, maximum steps and files.
  `regression/` below it holds the regenerated committed traces from the last freshness check.

## The gate

`cargo xtask conformance` runs, and `cargo xtask check` includes ([ADR 0060](adr/0060-conformance-stage-and-trace-corpus.md)):

| Stage | What it does |
|:--|:--|
| `formal-typecheck` | `quint typecheck` on every `formal/*.qnt` (`--typecheck` runs only this) |
| `formal-scenarios` | `quint test` on each module of `scenarios.qnt`, counting its runs |
| `formal-simulate` | 1000 random traces of 25 steps per step relation, every invariant checked |
| `formal-freshness` | Regenerates every committed trace and compares byte for byte, Quint's creation date aside; fails with `stale regression trace <file>` |
| `formal-generate` | Writes the generated traces and their manifest |
| `conformance-replay` | Replays the committed and generated traces against the real code (`preview-conformance`'s tests, with the per-field sensitivity and model-view tests); a trace whose length is not its manifest's is refused |
| `conformance-controls` | Faulty implementations, each caught at an asserted step ([ADR 0059](adr/0059-conformance-negative-controls.md)) |

Quint 0.33.0 is locked in `formal/package-lock.json` and runs with `--backend typescript`
(ADR 0052). Install it with `npm ci --prefix formal`. One seed serves the whole run and is
printed; `QUINT_SEED=<seed> cargo xtask conformance` repeats it. The `conformance` job of
`.github/workflows/ci.yml` runs the same stages (`cargo xtask check --only <them>`) on every push
to `main` and every pull request, and `.github/workflows/formal.yml` repeats them nightly with a
fresh seed (ADR 0072). A failed run of either keeps `formal/traces/generated/` as an artifact.

## The scheduled lane: `formal-verify`

`cargo xtask formal-verify` is not part of `check` ([ADR 0061](adr/0061-apalache-formal-verify-scheduled-lane.md));
the `formal-verify` job of `.github/workflows/formal.yml` runs it on the nightly schedule and on
demand, and keeps `target/formal-verify/` (every run's log, Apalache's run directories with a
counterexample as ITF and TLA+, and the ITF of a failing simulation) when it fails.

| Stage | What it does |
|:--|:--|
| `formal-verify-bounded` | `quint verify` through Apalache 0.62.1 (the pin in `tools/versions.toml`) of every conjunct of `allInvariants`, per model and step relation, to the depth in `BOUNDED` |
| `formal-verify-simulate` | `quint run` of the same invariants: 3 seeds derived from one printed base seed, 5000 traces of 50 steps per relation (the pull-request run samples 1000 of 25) |

It needs Node and Quint as `conformance` does, a JVM of version 17 or newer and the Apalache
release, which `formal-verify` downloads, checks against the SHA-256 pinned in
`tools/versions.toml` and unpacks into `~/.quint/apalache-dist-0.62.1` before Quint runs (ADR
0061, amended), so Quint never downloads it unchecked. A missing JVM, a digest that differs
or an Apalache that cannot be downloaded fails with the fix; a failure is a counterexample only
when Quint prints `[violation]`, otherwise it is a tooling failure that verified nothing.
`QUINT_SEED=<base> cargo xtask formal-verify` repeats the simulation. A counterexample is promoted
as described below.

## Verification baseline

Measured with `cargo xtask formal-verify` on a shared 4-core, 16 GB sandbox that other jobs
were loading (load average 9 to 16), Java OpenJDK 21.0.12, Quint 0.33.0, Apalache 0.62.1, base
seed `0xf10`. Durations are Quint's own report and are an upper bound for a quiet machine.

| Model | Step relation | Depth | Result | Duration |
|:--|:--|--:|:--|--:|
| lifecycle | `step` | 4 | no violation | 292 s |
| lifecycle | `serverStep` | 2 | no violation | 228 s |
| reload | `step` | 4 | no violation | 303 s |
| watcher | `step` | 2 | no violation | 386 s (572 s after FORMAL-10's changes, re-measured alone) |

Every conjunct of the model's `allInvariants` held in every state reachable within the depth,
for every choice of every nondeterministic value (the conjuncts of `allInvariants`: lifecycle 6,
reload 7, watcher 5; Apalache splits the nested `and`s further). The whole bounded stage took 695 s with two runs at a time. The multi-seed
simulation (seeds `0xf10`, `0x9e3779b97f4a8b25`, `0x3c6ef372fe95073a`; 5000 traces of 50 steps
each, all five relations) found no violation either: 106 to 251 s per run, 638 s for the stage.
No counterexample was found, so none was promoted.

### What each bounded depth reaches

A pass at depth `n` says nothing about states first reached after `n` steps. The shortest
path to each witness (`val reaches*` in the model; the bounded relation's steps, `init` not
counted), measured by simulating at increasing depths and checked against the named runs:

| Model | Relation | Depth | Reached within the depth | Not reached |
|:--|:--|--:|:--|:--|
| lifecycle | `step` | 4 | `reachesStopAwaitingBind`, `reachesFailed` (2), `reachesServingWatched`, `reachesServingDegraded`, `reachesStopAwaitingWatcher` (3), `reachesOpenStream` (4) | none |
| lifecycle | `serverStep` | 2 | `reachesServingWatched`, `reachesServingDegraded`, `reachesFailed` (1), `reachesOpenStream` (2) | `reachesStopAwaiting*` (unreachable under `serverStep`, ADR 0051) |
| reload | `step` | 4 | `reachesRestart` (2), `reachesReload` (4) | `reachesCoalescedGap` (5), `reachesHotApply` (6) |
| watcher | `step` | 2 | `reachesOverflow`, `reachesDegraded`, `reachesExcusedChange`, `reachesCappedStartupWatching` (1), `reachesSettledWithUnseenChange`, `reachesRootDeleted` (2) | `reachesRecovered`, `reachesRootRemoved` (3), `reachesRootRemovalEscalated`, `reachesPublish`, `reachesSettledAfterPublish` (4), `reachesRecoveredCaughtUp` (6) |

So the watcher's bounded run never reaches a published revision, let alone a settled state
after one, a recovery or a caught-up recovery: `noLostEligibleChange`'s interesting cases and
every conjunct about a fired revision are checked there only on states before any publish.
They are checked by the simulation (`formal-verify-simulate`, 50 steps) and the replay, not
by Apalache. Nor does it reach a reported root removal, since `rm -r` of the root takes two
steps (ADR 0073), so the race FORMAL-10 fixed there (a reconcile between them) is found by the
simulation and the named runs only. The same holds for the reload model's coalesced gaps and
hot applies.

The depths are shallow on purpose: Apalache's search grows about tenfold per step on these
models (reload: 24 s at 3 steps, 246 s at 4; watcher: 227 s at 1 step, 483 s at 2, over 600 s
at 3; lifecycle `serverStep`: 6 minutes at 2 steps, over 10 at 3, because one step is a whole
start-up of up to four machine transitions). The simulation is what reaches depth. A bound is
raised only after its duration is measured here.

What was found on the way, all in the tooling or the model's form and none in a property:

- **Download.** Quint's fetch of the Apalache release from GitHub worked through the sandbox
  proxy (the Rust evaluator's, which ADR 0052 avoids, was not tried). No mirror was needed.
- **Stack.** On `lifecycle.qnt` Apalache stopped after its parser pass with an empty `error:`.
  The cause is a JVM stack overflow in the translation of the nested `match` of `transition`;
  `JVM_ARGS=-Xss64m` fixes it, and the xtask sets it.
- **Inlining.** `startRun` passed three nested `startTurn` calls as arguments; Apalache
  substitutes an argument at every use, so the term grew exponentially and ran a 3 GB heap
  out of memory (about 8 minutes in). `lifecycle.qnt` now binds the turns with `val`, which
  changes no behaviour (`formal-freshness` agrees with every committed trace).
- **Search mode.** Checking the invariants once after each step
  (`search.invariant.mode = after`, set by the xtask) took about a third of the default's time
  at the same depth.
- **`serverStepMostlyBound`** is not verified: it lists the bound start three more times to
  re-weight sampling and reaches the same states as `serverStep`, but Apalache tries every
  copy, and depth 2 had not finished after 33 minutes. It is simulated.
- **Parallel runs** each need their own server port (the xtask picks a free one); two runs on
  Quint's default port 8822 share or fight over one server.

### Liveness: attempted, no result

- `quint verify --temporal liveness` (Apalache): refused, `error: Handling fairness is not
  supported yet!` (`weakFair` in `fairness`).
- `--temporal reconnectObservesNewer` alone (Apalache), which is false without the fairness and
  environment assumptions anyway: `error: Input error (see the manual): SubstRule: Variable
  b_1671$1 is not assigned a value`, from the quantifier over browsers in the temporal formula.
- `--backend tlc --temporal liveness` ran (the TLC bundled in the Apalache release) but its
  state space does not close: the `stream` and history lists grow without bound, so after 6
  breadth-first levels it held 2.4 million distinct states and was still growing (a first
  attempt was killed by the memory limit after 3.6 minutes). It gave no verdict.

So `reconnectObservesNewer` stays stated and argued in `reload.qnt` (its safety half,
`drainedClientIsCurrent`, is an invariant and verified above), and is not checked by any
lane. Checking it needs a bounded variant of the model (capped streams and history) and a
fairness encoding Apalache accepts, or TLC on that variant; that is a model change for its own
task.

## Promoting a counterexample

A failing simulation, a generated trace that diverges, or a scheduled run's artifact is a
counterexample. To keep it:

1. Reproduce it: `QUINT_SEED=<seed> cargo xtask conformance`, or replay the artifact's ITF
   file with the driver's test.
2. Decide which side is wrong. If the code is wrong, fix the code. If the model is wrong,
   fix the model first (D13).
3. Reduce it to a named run in the model's module of `formal/scenarios.qnt`: the shortest
   sequence of actions that shows the problem, with `expect`s on what must hold.
4. Script the same run in `formal/<model>_traces.qnt`: one `at(k, ...)` per step, and a doc
   comment `` `<module>::<run>`: [`<init>`, ]<n> steps.``.
5. Add its entry to `formal/traces/regression/manifest.toml` (seed `0x5ce1`, the script's
   length and initial action), and export it with the command in that file's header.
6. Add the run's name to the driver test's `NAMED` list. If the trace moves a negative
   control's catch point, update the control and ADR 0059's table in the same commit.

`cargo xtask conformance` then has to pass. The new trace must be fresh, and the replay must
agree with it.
