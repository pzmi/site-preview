# 0058. Watcher conformance driver: the coordinator in process, the environment from the trace

Date: 2026-10-12

## Status

Proposed (planner default — owner may override). Amended by ADR 0070: the driver runs
composition's `WatchDecisions`, the pipeline's own synchronous decision step, instead of
restating it; a refused readiness is a divergence; the claim that the lifecycle driver covers
the pipeline is withdrawn.

## Context

FORMAL-06 replays `formal/watcher.qnt` (ADR 0054) against the real `ChangeCoordinator` and
`ReloadState` with a `FakeClock` and the real classification, through the replay core of
ADR 0053, observing `{revision, dirty, health}` after every step. The model holds more than
the coordinator: an operating system that turns site changes into notifications, the
bridge's bounded channel with its loss and failure flags (ADR 0048), the adapter's watches
and its reconcile (ADR 0050), and the pipeline task between them (ADR 0049).

- The pipeline (`WatchPipeline`), its `WatchControl` seam and composition's `reconciled` are
  `pub(crate)` and async; the native watcher cannot be made to lose notifications, fill its
  channel or refuse one directory on demand, and its timing is the operating system's.
- `Driver::apply` receives an action and its picks only. Most of the model's actions
  (`modify`, `dirDelete`, `kernelQueue`, `duplicate`, ...) record no picks, and the batch a
  `receive(k, capped)` takes depends on everything queued before it.
- The task names five scenarios; `watcherScenarios` has eleven runs.

## Decision

- **The system under test is the coordinator, in process.** `WatcherDriver` drives the real
  `ChangeCoordinator` (its `Debouncer`, `classify`, `ChangeSet` and `WatcherHealth`)
  publishing to a real `ReloadState`, on a `FakeClock` where one model tick is 75 ms (the
  model's window is two ticks, the debounce window 150 ms). The driver plays the pipeline's
  part through the pipeline's own decision step (`WatchDecisions`, ADR 0070): a batch to
  `receive`, a due deadline to `settle`, a reconcile when the outcome asks for it (`Full` for
  an overflow), a full reconcile after a rescan revision unless one ran since the previous
  revision, and the reconcile's outcome as `watcher_reconciled(dirs, now)` (readiness) or
  `watcher_failed(RegistrationFailed)` (a new problem). The start-up report is
  `watcher_ready`.
- **The environment comes from the trace.** `WatcherDriver::new(&trace)` decodes each
  state's `src` and `fs` (the bridge's channel and flags, the watches, the site) and nothing
  else. A step starts from the previous state's environment: `receive(k, capped)` hands the
  coordinator the batch the bridge's `next` delivers from it (`Overflow`, `Failed`, or up to
  `k` changes with real names: `index.html`, `a.css`, `c.css`, `d/b.js`, `d`,
  `.index.html.swp`, `.git/HEAD`, the root), and a reconcile reports what the adapter would
  from it and the step's `capped`. The OS actions are no-ops for the driver. The model's
  debouncer, health, revision, `reconciledFully` and `last` are never read for a stimulus;
  they are only projected for the comparison.
- **The observation is wider than the task's three fields**, all read from the real code:
  the revision (`ReloadState::current`), `dirty` and the deadline in milliseconds
  (`ChangeCoordinator::deadline`, which is the pipeline's timer), the health, the received
  batch's `recorded` count and `needs_reconcile`, the health events the step returned, and
  the revision the step published as the `ReloadState` carries it (full or not, its URL
  paths) with `settle`'s rescan flag. Without the deadline, a debouncer that fired on the
  leading edge would pass: the driver settles only when the model does.
- **A `Coordinator` trait** in the driver lists the commands and queries the driver uses,
  implemented for `ChangeCoordinator`; `WatcherDriver::with_coordinator` takes a constructor,
  so FORMAL-08's faulty coordinators replace it behind the same driver.
- **All eleven runs are scripted** in `formal/watcher_traces.qnt` (module `watcherTraces`),
  as `lifecycle_traces.qnt` does (ADR 0056), with an `init` or `initUnwatchable` initial
  action, and exported with `quint run --mbt --seed 0x5ce1` into
  `formal/traces/regression/watcher-<run>.itf.json` (the command is in that file). Generated
  traces sample the model's own `step`: 30 traces of 25 steps, seed printed, `QUINT_SEED`
  repeats it.

## Consequences

- The coordinator, debouncer, classification, change set, health machine and reload state
  are checked against the model after every step; a leading-edge debouncer, a dropped
  overflow, an overflow without a rescan and the recovery without a full reload (ADR 0054)
  each diverge at the step that shows them.
- The adapter, the bridge and the pipeline task's async shell are not under test here: the
  driver restates the bridge's `next` order and the reconcile's report (a few lines each),
  and the pipeline's priorities (cancellation, then the deadline, then a batch) are taken
  from the model's schedule. Their own tests (ADR 0048 to ADR 0050, composition's
  `watch_pipeline` tests) cover them; the lifecycle driver does not (its observation has no
  revision, health or reconcile). A driver over the native watcher would need the seams above
  and would make the OS's timing part of the comparison. Since ADR 0070 the pipeline's
  decisions themselves are the shared `WatchDecisions`, which the driver runs.
- A script and its run can drift; FORMAL-09's freshness check regenerates them with their
  seed, and a change to a run changes its script in the same commit.
