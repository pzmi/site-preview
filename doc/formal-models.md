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
   `formal/<model>_traces.qnt` in the same commit.
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
| `formal/watcher.qnt` | `watcher` | `step` (an OS lane, a pipeline lane, an ending lane) | `drivers/watcher.rs` | the real `ChangeCoordinator`: classification, change set, debouncer, health, with a real `ReloadState` and a fake clock ([ADR 0054](adr/0054-watcher-model.md), [ADR 0058](adr/0058-watcher-conformance-driver.md)) |
| `formal/reload.qnt` | `reload` | `step` | `drivers/reload.rs` | the real `ReloadState` and SSE handler through the real router, in process with paused time ([ADR 0057](adr/0057-reload-conformance-driver.md)) |

### Stimuli

- **Lifecycle:** `startServer(bindOk, watcherOk)`, `refuseStart`, `request`, `openStream` and
  `stopServer`. A bind failure is a port the driver already holds. A refused start is a root
  that does not exist. A watcher failure is the `start_with_unavailable_watcher` seam.
- **Watcher:** the operating system's notifications (modify, create, delete, rename, atomic
  save, temp-file writes, `.git` changes, directory and root deletion and recreation,
  duplicates, reordering, queue overflow, runtime failure) come from the trace's own
  environment. The driver turns the bridge's delivery into `receive(k, capped)`, a due
  deadline into `settle`, and a reconcile into `watcher_reconciled` or `watcher_failed`.
- **Reload:** `publish` and `publishHot` (synthetic change sets per kind), `connect`, `send`,
  `disconnect`, `userReload`, `receive`, `stopServer` and `startServer` (a new instance).

### Observations

Everything is read from the real code and projected into the model's terms:

- **Lifecycle:** the `Server` state (or the `StartupError` variant), whether the listener
  still completes a TCP handshake, whether any task is still owned, and whether a request
  got a `200`.
- **Watcher:** the committed revision, `dirty` and the deadline, the health, the received
  batch's `recorded` count and reconcile request, the health events returned, and the
  revision published (full or not, its paths, the rescan flag).
- **Reload:** `up`, the committed revision, the latest change set, and per open connection
  the events its body yielded (name, instance, revision, kinds, `full`). The browser's own
  state is the model's, never observed (D16).

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
| watcher | `onlyEligibleTriggerReload` | Only notifications naming a served entry, or an overflow, make a reload pending |
| watcher | `noLostEligibleChange` | A settled pipeline has published every change it saw; once `Watching`, nothing is pending |
| watcher | `boundedReloadsPerBurst` | One revision per burst, after a quiet window, however many duplicates it absorbs |
| watcher | `failureIsExplicit` | Live reload is claimed only with every eligible directory watched; each degradation and recovery is announced once |
| watcher | `overflowRescans` | An overflow leads to a full reconcile and a full revision |
| reload | `monotonicRevision` | Revisions and instances only move forward, on the server and on every connection |
| reload | `noUncommittedClaim` | No event or baseline claims a state or change set that was not committed |
| reload | `atMostOneReloadPerChange` | A client reacts at most once per change, and never to an unchanged server |
| reload | `noHotApplyAcrossGap` | A client never applies in place across a gap of revisions |
| reload | `htmlChangeIsFull` | A change to an HTML or other page is a full reload everywhere |
| reload | `hotApplyAdvancesBaseline` | A hot apply moves the baseline to what it applied; otherwise only navigation moves it |
| reload | `drainedClientIsCurrent` | A client that handled every event is at the last state its connection announced |

The reload model states its liveness separately: `reconnectObservesNewer` under `fairness`
and `channelAvailable` (`liveness`). Simulation does not check it. FORMAL-10's `quint verify`
lane is where it belongs.

### Scope: what is not modelled

- **Lifecycle:** the drain, and the forced exit at a second signal, inside `JoinTasks`
  (ADR 0045); the domain machine's stop-during-start-up states, unreachable through `Server`
  (ADR 0051), so `lifecycleMachineScenarios` runs under `quint test` but is not replayed.
- **Watcher:** revision exhaustion, the 64-path cap, nesting below one directory level, a
  watcher that cannot be created, and `Closed` before shutdown. The adapter, the bridge and
  the pipeline task are restated by the driver, not driven. Their own tests and the
  lifecycle driver cover them.
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
| `conformance-replay` | Replays the committed and generated traces against the real code (`preview-conformance`'s tests) |
| `conformance-controls` | Faulty implementations, each caught at an asserted step ([ADR 0059](adr/0059-conformance-negative-controls.md)) |

Quint 0.33.0 is locked in `formal/package-lock.json` and runs with `--backend typescript`
(ADR 0052). Install it with `npm ci --prefix formal`. One seed serves the whole run and is
printed; `QUINT_SEED=<seed> cargo xtask conformance` repeats it. `.github/workflows/formal.yml`
runs the same command on every push to `main` and every pull request, and nightly with a fresh seed. A
failed run keeps `formal/traces/generated/` as an artifact.

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
