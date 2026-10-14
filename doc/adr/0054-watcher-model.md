# 0054. The watcher model: abstraction, a settled-state encoding of no lost change, a full reload at recovery, root deletion

Date: 2026-10-12

## Status

Accepted (owner, 2026-10-14). Root deletion amended by
[ADR 0067](0067-root-removal-and-watcher-review-fixes.md): a removed root now degrades the
watcher with `RootRemoved`, for good.

## Context

FORMAL-02 asks for `formal/watcher.qnt` (foundation.md section 6.1 item 2) with the invariants
`onlyEligibleTriggerReload`, `noLostEligibleChange`, `boundedReloadsPerBurst` and
`failureIsExplicit`, retrofitted onto the code (ADR 0014): the domain's `classify`,
`ChangeSet` and `Debouncer`, the `ChangeCoordinator` and `WatcherHealth`, the bridge of
ADR 0048, the pipeline task of ADR 0049 and the reconcile of ADR 0050. Three points are not
settled by the plan:

- `noLostEligibleChange` reads as liveness ("dirty is eventually published once settled"),
  but `quint run` checks state invariants, and a change the watcher could never see (its
  directory had no watch) cannot be published by any pipeline.
- foundation.md lists root deletion and recreation, which plan.md does not describe. The
  server holds the root's descriptor from start-up (plan.md section 5.3), so after a deletion
  it serves the deleted, empty directory, and a directory recreated at the same path is not
  the served one.
- The watcher recovers (`WatcherRecovered`) at the first complete reconcile (ADR 0040,
  ADR 0050). As first built, nothing marked the revision that follows as full, so a change
  made inside a directory while it could not be watched, and never notified, was not caught
  up: the browser kept stale content while live reload was claimed again. The model found
  it (`recoveryAfterWatchLimit` settled `Watching` with the change pending).

## Decision

- **Abstraction.** The site is a root with `index.html`, `a.css`, `c.css`, one eligible
  directory `d` holding `d/b.js`, an editor swap name and `.git/HEAD`. The model's OS emits
  the notifications a native watcher emits, only through a watch of the entry's directory;
  the channel holds `CAP = 4` messages; time is a tick counter with a window of two ticks.
  Classification, the change set, the debouncer, the health machine, the bridge's `next`
  order and the pipeline's priorities (cancellation, then a due deadline, then a batch) are
  modelled row for row. Outside the model: revision exhaustion, the 64-path cap, nesting below
  `d` (on a one-level tree a `Changed` reconcile reaches `d` just as a `Full` one does), a
  watcher that cannot be created, and `Closed` before shutdown.
- **`noLostEligibleChange` over a settled condition.** Ghost state records every served file
  changed since the revision that last covered it (a full revision, or one naming its path:
  a reload fetches the current content), and separately the changes made while their
  directory's watch had failed and been reported. The invariant is: whenever the pipeline has
  settled (not shut down, nothing queued, no loss or failure flag, debouncer clean), every
  pending change is one of those unseen ones, and, once the health is `Watching`, that
  nothing is pending at all: an unseen change is excused only while live reload is not
  claimed. Every way out of an unsettled state receives
  something or fires the deadline, so under the fairness that the task keeps running a trace
  that settles has published everything the watcher saw. The unseen changes belong to
  `failureIsExplicit`: their directory is outstanding only while the health is degraded (or
  the start-up failure is still queued), and every degradation and recovery is announced once.
- **A recovery publishes a full reload.** After a reconcile, composition reports readiness
  through `ChangeCoordinator::watcher_reconciled(dirs, now)`. When that readiness is a
  recovery (`Degraded` to `Watching`), the coordinator marks the debouncer with
  `Debouncer::mark_recovered(now)`: dirty, deadline a window later, the change set full with
  the new `FullReason::Recovered`, and no rescan, since the reconcile that recovered has just
  registered every directory. A pending hot revision becomes full; with nothing pending, one
  full revision follows. Readiness from `Starting` (start-up, `watcher_ready`) or while
  already watching marks nothing, so start-up causes no reload. The model's `ready` does the
  same (`markRecovered`). Rejected: treating the recovery as an overflow (`mark_overflow`),
  which also asks for a rescan and so a second full walk of the tree for nothing; a rescan
  request from the reconcile that newly registers a directory, which needs the adapter to
  tell new registrations apart and still publishes nothing when the registration was a
  retry; giving `watcher_ready` itself a `now`, which would date the start-up report for a
  transition that cannot happen there (its coordinator is fresh).
  A recovery found by the reconcile that a rescan revision runs after its publish adds one
  more full reload a window later; it is rare and harmless, and the model allows it
  (`boundedReloadsPerBurst` dates a revision's quiet window against the changes recorded
  before it, `firedQuiet`).
- **Root deletion** is every removal plus the root's own, which classifies as `Rescan`, and
  then no watch at all. Recreation is never notified and changes nothing served. plan.md is
  silent on root deletion: section 8.1 handles removed subdirectories (drop their watches,
  rescan the nearest existing parent) and section 8.4 runtime failures, and section 5.3 serves
  through the root descriptor opened at start-up. *Superseded by ADR 0067:* as first built
  the health stayed `Watching`, which plan.md section 8.4 forbids ("must not claim live
  reload is active when notifications are unavailable"); the next reconcile now finds the
  root path no longer names the served directory and degrades with `RootRemoved`, which
  nothing recovers, a root recreated at the same path included.
- **One step relation**, `step`, picks a lane first (OS, pipeline, or the run-ending actions
  of root deletion and shutdown, each lane able to tick), so random traces publish, fail and
  recover instead of ending early.

## Consequences

- The recovery gap is closed in the model and the code. `noLostEligibleChange` now fails on
  the old model: a random run (`quint run --invariant noLostEligibleChange --max-steps 40
  --max-samples 5000 --seed 7`) finds a settled `Watching` state with `d/b.js` pending, and
  `recoveryAfterWatchLimit` reached it by hand. With the fix the scenario settles with
  nothing pending after a full revision (`reachesRecoveredCaughtUp`), and
  `reachesRecoveredWithStaleChange` is witnessed by no trace. `recoveryAloneReloads` pins a
  recovery with nothing else pending. In Rust, `should_publish_full_revision_when_watcher_recovers`,
  `should_make_pending_hot_revision_full_when_watcher_recovers` and the pipeline's
  `should_publish_full_reload_when_watcher_recovers` fail without the fix;
  `should_mark_nothing_when_reconciled_without_recovery` and
  `should_keep_revision_hot_when_reconcile_is_ready_without_recovery` fail if every readiness
  marked a full reload.
- After a root deletion the health is `Degraded(RootRemoved)` (ADR 0067), and the model's
  `noLiveClaimWithoutRoot` checks it.
- FORMAL-06 replays these traces against the real `ChangeCoordinator` and `ReloadState`: the
  model's `receive` maps to `ChangeCoordinator::receive`, `settle` to `settle`, a reconcile's
  outcome to `watcher_reconciled` or `watcher_failed(RegistrationFailed)`; the OS actions and the
  queue are the driver's scripted source.
