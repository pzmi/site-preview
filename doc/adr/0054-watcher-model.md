# 0054. The watcher model: abstraction, a settled-state encoding of no lost change, root deletion

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

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
  ADR 0050). Nothing marks the revision that follows as full, so a change made inside a
  directory while it could not be watched, and never notified, is not caught up.

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
  pending change is one of those unseen ones. Every way out of an unsettled state receives
  something or fires the deadline, so under the fairness that the task keeps running a trace
  that settles has published everything the watcher saw. The unseen changes belong to
  `failureIsExplicit`: their directory is outstanding only while the health is degraded (or
  the start-up failure is still queued), and every degradation and recovery is announced once.
- **Root deletion** is every removal plus the root's own, which classifies as `Rescan`, and
  then no watch at all. Recreation is never notified and changes nothing served.
- **One step relation**, `step`, picks a lane first (OS, pipeline, or the run-ending actions
  of root deletion and shutdown, each lane able to tick), so random traces publish, fail and
  recover instead of ending early.

## Consequences

- The recovery gap is visible: `recoveryAfterWatchLimit` in `formal/scenarios.qnt` settles
  with the health `Watching` and a change still unpublished (`reachesRecoveredWithStaleChange`).
  It is allowed by the invariants as stated, since the loss was reported while it happened.
  Closing it (for example a full revision at `WatcherRecovered`, or a rescan request from the
  reconcile that newly registers a directory) is a code change for a later task, together
  with tightening the model's invariant.
- After a root deletion the health stays `Watching` with no watch: nothing served can change
  any more, so the claim is vacuous; the model does not flag it.
- FORMAL-06 replays these traces against the real `ChangeCoordinator` and `ReloadState`: the
  model's `receive` maps to `ChangeCoordinator::receive`, `settle` to `settle`, a reconcile's
  outcome to `watcher_ready` or `watcher_failed(RegistrationFailed)`; the OS actions and the
  queue are the driver's scripted source.
