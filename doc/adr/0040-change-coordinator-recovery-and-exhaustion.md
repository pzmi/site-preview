# 0040. Change coordinator: recovery signal, reconcile requests and revision exhaustion

Date: 2026-10-11

## Status

Accepted (owner, 2026-10-14). Amended by ADR 0054: after a reconcile,
readiness is reported through `watcher_reconciled(dirs, now)`, and a recovery makes the next
revision a full reload.

## Context

APP-04 builds `ChangeCoordinator`, which turns change batches into published revisions and
health events. Three questions are left open by plan.md sections 8.3 and 8.4 and by the
ports of ADR 0038:

- The `ChangeSource` port reports `Failed(reason)` but has no "recovered" batch, while the
  domain's `WatcherHealth` needs `Ready { dirs }` to recover. The task text names a
  `watcher_recovered()` command with no directory count.
- plan.md section 8.3 reconciles directory watches before a revision is published, but the
  debounce only knows a directory changed when an event says so.
- The revision counter can run out (`RevisionExhausted`). The domain makes that degradation
  absorbing; the application must not keep trying to publish.

## Decision

- **Readiness is reported by the caller, not by the source.** `watcher_ready(dirs)` is the
  only way into `Watching`: composition calls it with the size of the watch set after the
  watcher adapter registered its directories, at startup and after a successful reconcile
  or bounded retry. After a failure it emits `WatcherRecovered` once. Failures still arrive
  as `ChangeBatch::Failed` and go through `receive`. The port gains no variant: a
  re-registration is something composition drives (it owns the reconcile hook, CLI-03,
  FS-09), so it already knows when one succeeded and how many directories it covers.
- **Reconcile is requested at record time.** `RecordOutcome::needs_reconcile` is set by a
  relevant directory change, by an event too coarse to attribute (`Rescan`, which may hide
  one) and by an overflow, so the caller reconciles while the window is still open, before
  `settle` publishes. `Settled::rescan` still tells the caller a full rescan is due.
- **Exhaustion degrades once and makes the coordinator inert.** When `publish_next` returns
  `RevisionExhausted`, nothing is published, the health moves to
  `Degraded { RevisionExhausted }` with one `WatcherDegraded` event, and from then on
  `record`, `overflow` and `settle` do nothing and return no deadline, so the publish is
  never retried and the pipeline never wakes for it. A later `watcher_ready` is refused with
  `WatcherHealthRefusal`, which the caller logs and ignores.
- **One dispatch.** `receive(batch, now)` maps every `ChangeBatch` to its command, so the
  pipeline in composition makes no decision of its own; `Closed` tells it to stop.

Rejected: a `ChangeBatch::Recovered { dirs }` variant (every source, fake and contract
harness would carry a state the adapter cannot know better than composition); recovering on
the first event batch after a failure (a partial watch delivers events while still broken);
retrying the publish on exhaustion (it can never succeed).

## Consequences

- CLI-03 calls `watcher_ready(watch_set.len())` after startup registration and after each
  successful reconcile, reconciles whenever `needs_reconcile` or `Settled::rescan` is set,
  arms its timer from `next_deadline`, and stops consuming on `Received::Closed`.
- FS-06 reports an initialization failure as a `Failed` batch; it does not report recovery.
- FORMAL-02 models readiness as an input of the pipeline, not of the source, and models
  exhaustion as absorbing.

## Amendments

- **2026-10-11 (APP-04 checkpoint review).** Exhaustion is reachable from `settle` alone:
  `ChangeBatch::Failed` carries a `SourceFailure` (ADR 0038, amended) that cannot name
  `RevisionExhausted`, and `watcher_failed` takes that type, so a source cannot make the
  coordinator inert or drop its pending changes. Once exhausted, "return no deadline" covers
  every path: `deadline()` and the outcome of `receive(Failed)` return `None` as well as
  `record`, `overflow` and `settle`, so the caller's timer is never armed for a past
  deadline. `watcher_ready(0)` is refused like a `Ready` after exhaustion: the root is
  always watched, so zero registered directories is not a working watch and must not claim
  live reload.
- **2026-10-11 (APP-04 checkpoint review).** Reconcile is also requested for a `Create`,
  `Remove` or `Rename` whose name is eligible as a directory (ADR 0031, amended), since the
  watcher cannot tell a moved-in directory from a file.
