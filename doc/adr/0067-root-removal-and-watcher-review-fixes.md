# 0067. A removed root degrades the watcher for good; start-up failures first; start-up readiness only from `Starting`; composition's hidden seams

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Amended by ADR 0073: the root's removal
reaches a reconcile through a watch on the root's parent (Linux).

Amends [ADR 0054](0054-watcher-model.md) (its root-deletion decision) and
[ADR 0048](0048-watcher-callback-bridge.md) (the order of `next`).

## Context

The FORMAL-10 checkpoint review found, in the watcher and its models:

- **Root deletion.** After `rm -r <root>` every watch is gone, but the health stayed `Watching`
  and no `degraded` event was emitted. ADR 0054 kept that as built and left a distinct report
  to the owner. plan.md section 8.4 already decides it: the tool "must not claim live reload is
  active when notifications are unavailable". The server serves through the root descriptor it
  opened at start-up (plan.md section 5.3), so after a deletion it serves the deleted, empty
  directory; a directory moved away is still served under its new name; and a new directory
  made at the same path is not what is served.
- **A stale start-up failure.** With a directory unwatchable at start-up the source holds
  `Failed(RegistrationFailed)` for its first batch. `next` read the overflow flag first, so a
  loss reported before that batch ran a full reconcile, which could watch everything and move
  `Starting` to `Watching`; the held failure then degraded a watcher with nothing outstanding.
- **`StartReport::is_complete`** ignored a registration stopped at its relisting cap, while
  `ReconcileReport::complete` counts it, so start-up could report readiness with directories
  left for a reconcile.
- **`ChangeCoordinator::watcher_ready`** is public and, on a degraded coordinator, recovered
  without the full reload that `watcher_reconciled` marks (ADR 0054).
- **Composition's `#[doc(hidden)]` items** (`Server::start_with_unavailable_watcher`,
  `Server::owns_tasks`) exist for the conformance drivers but nothing stopped another crate
  from calling them.

## Decision

- **A removed root is a permanent degradation, `RootRemoved`.** Every reconcile first checks
  that the root path still names the directory the root capability holds: the descriptor's
  device and inode against a `stat` of the path, and the descriptor's link count (a deleted
  directory has none left; a file system may give a new directory the freed inode number at
  once, so the pair alone could take a recreated root for the original). When it does not, the
  reconcile walks and registers nothing (a watch on the path would follow a directory that is
  not served) and reports `root_removed`; the pipeline calls
  `ChangeCoordinator::watcher_failed(SourceFailure::RootRemoved)`. The health machine treats
  `DegradeReason::RootRemoved` like revision exhaustion: entered once, with one
  `WatcherDegraded` event, escalated to once from another degradation (so the warning says why
  live reload cannot come back), refusing every later `Ready`, and absorbing every further
  failure but exhaustion. The warning is `root-removed`, the `--json` watcher event's reason
  `root-removed`.
- **A root recreated at the same path does not recover.** Nothing served follows the new
  directory: the descriptor still names the old one, so "live reload" would watch a tree that
  is not served. The watcher stays degraded and the warning says to restart site-preview.
  Re-opening the root at run time (a second use of ambient authority, against plan.md section
  5.3) or exiting on a root removal are rejected; the owner may choose either later.
- **The start-up failure is delivered first.** `NotifyChangeSource` keeps it apart from the
  channel and `next` delivers it before the overflow flag, so a loss's full reconcile can only
  recover after it (degraded, then recovered, each once).
- **A deferred start-up is incomplete, not failed.** `StartReport` gains `deferred` (the
  relisting cap was passed); `is_complete` is false then, as for `ReconcileReport`, but no
  failure is held: the coordinator stays `Starting` until the `Overflow`'s full reconcile
  reports readiness. The start-up output still reports the watch as complete, since nothing
  failed and the reconcile follows at once.
- **`watcher_ready` is the start-up report only.** It refuses (`WatcherHealthRefusal`) in any
  health but `Starting`; readiness after a reconcile goes through `watcher_reconciled`. The
  negative control that needs an unmarked recovery builds it from public commands.
- **`ARCH-hidden-seam`.** `cargo xtask architecture` rejects any crate other than
  `preview-composition` and `preview-conformance` that names a `#[doc(hidden)]` item of
  `preview-composition` (by method or path name). Preferred over a cargo feature, which feature
  unification could turn on for the shipped binary.
- **Models first.** `watcher.qnt` gains the reason `RootRemoved`, the escalation, the order
  of `nextBatch`, a capped start-up in `init`, the invariant `noLiveClaimWithoutRoot`, and a
  `failureIsExplicit` conjunct that a `RegistrationFailed` degradation needs a directory
  outstanding; `lifecycle.qnt` keeps `Failed` terminal (see Consequences).

## Consequences

- `noLiveClaimWithoutRoot` fails when the model's reconcile ignores a missing root (a random
  run finds it, and the four root scenarios fail); the strengthened `failureIsExplicit` fails
  with the old `nextBatch` order (the reviewer's probe, and a random run with seed 7). In Rust,
  `should_report_root_removed_when_root_is_deleted`,
  `should_report_root_removed_when_root_is_replaced_at_its_path`,
  `should_degrade_once_when_reconcile_finds_root_removed`,
  `should_report_root_removed_once_when_site_root_is_deleted` (the binary),
  `should_deliver_startup_failure_before_a_loss_when_both_are_pending`,
  `should_report_incomplete_start_without_failure_when_registration_passes_the_relisting_cap` and
  `should_refuse_startup_ready_when_not_starting` fail without their fixes.
- A user who deletes and recreates the root must restart site-preview; the README's known
  limitations and the runbook say so.
- `Failed` stays terminal in `lifecycle.qnt`: it is the state of one `Server` whose start-up
  failed, and `Server::start` then returns the error and no handle, so there is nothing to
  restart; the binary exits 1. A library caller may call `Server::start` again, which builds a
  new machine from `Stopped` with nothing held (`failedStartupHoldsNothing`), so every state it
  reaches is one the model reaches from `init`.
