# 0049. The watcher pipeline task: deadline-only timer, reconcile hook, cancellation and watcher warnings

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

## Context

plan.md §8.3 asks for one global trailing-edge debounce fed by the native watcher through a
bounded bridge, §8.4 for degraded mode that keeps HTTP serving, and §9 for background tasks
that are owned, cancellable and joined or bounded on shutdown, stopped after the listener and
the event streams. The application's `ChangeCoordinator` (ADR 0040) decides everything about a
batch and a deadline but reads no clock and owns no timer; the filesystem adapter's
`NotifyChangeSource` (ADR 0047, ADR 0048) delivers batches and may only be named by
composition's bootstrap file (ARCH-fs-authority). The `ChangeSource` port is generic, not
object safe (ADR 0038).

## Decision

- **One task.** `preview-composition/src/watch_pipeline.rs` runs one spawned task per
  process. It waits, in this priority order, on a cancellation `oneshot` (a send or a dropped
  sender both stop it), the debounce timer and the source's next batch, rebuilding both inner
  futures on every turn (the port is cancellation safe). Priority keeps an always-ready source
  from holding off a stop or postponing a due revision.
- **The timer comes only from a real deadline.** The task sleeps for
  `deadline.remaining_from(now)` only when `ChangeCoordinator::deadline()` is `Some`; nothing
  pending arms no timer, and an inert coordinator (exhausted revisions) arms none either.
  `remaining_from` never ends early, so `settle` never sees the deadline in the future and the
  loop cannot spin.
- **A closed source is not polled again.** After `Closed` the task keeps settling what is
  pending and then waits for cancellation only; HTTP keeps serving (degraded mode).
- **The clock** is `TokioClock`, milliseconds since the task started on
  `tokio::time::Instant`, so paused Tokio time drives the pipeline in tests. Composition's
  tests take `tokio`'s `test-util` feature as a dev-dependency feature for that, allowed in
  `architecture.toml` beside the HTTP adapter's.
- **The source behind a composition trait.** `WatchControl` (the `ChangeSource` port plus
  `reconcile`, `failure_detail` and `close`) is implemented for `NotifyChangeSource` in
  `bootstrap.rs`, which returns it as an opaque `impl WatchControl`. The rest of composition
  never names the adapter.
- **Reconcile hook.** The task calls `reconcile(Changed)` at once when a batch's
  `needs_reconcile` is set (a new directory is watched before the edits that follow its
  creation), `reconcile(Full)` for an overflow, and `reconcile(Full)` after a revision that
  asks for a rescan unless a full reconcile already ran since the previous revision.
  `Some(dirs)` from the hook is reported to `watcher_ready` (ADR 0040). Until FS-09 the
  adapter's hook changes nothing and returns `None`.
- **Start-up.** The watcher starts after the listener is bound and before `ready` is
  reported; `watcher_ready(dirs)` is called only when every eligible directory is watched.
  `Ready.watching` carries a `WatchStatus` (`Watching`, `Partial`, `Unavailable`), which the
  human output prints as `Watching: N eligible directories`, with a degraded note, or
  `Watching: unavailable (<reason>); live reload disabled`.
- **Warnings** (ADR 0020 format, README table): `watch-registration-failed` per directory that
  cannot be watched, at most ten named and the rest counted; `watch-unavailable` when nothing
  is watched; `watch-failed` once per degradation by a runtime failure, with the adapter's
  root-relative detail; `reload-exhausted` when the revisions run out. A registration
  failure's `WatcherDegraded` adds no warning: the directories were already named.
- **Shutdown.** At the first stop signal, after the event streams are ended, the task is
  cancelled; after the connection drain it is joined within the drain bound and aborted past
  it. Cancelling closes the source, which removes every watch.

Rejected: a `tokio::select!` (composition does not take tokio's `macros` feature, and the
explicit `poll_fn` states the priority); arming a timer on every turn or a periodic tick
(wakes an idle process for nothing); reconciling only at the deadline (a directory created and
written to within one window would miss the write); a `Box<dyn>` source (the port returns an
`impl Future`).

## Consequences

- FS-09 implements `WatchControl::reconcile` in `bootstrap.rs` over the adapter's reconcile;
  the hook receives only a scope, so the adapter tracks which subtrees changed itself or the
  hook grows a parameter there.
- CLI-04 can inject a failing source through the same trait; CLI-06 maps `WatchStatus` and the
  pipeline's domain events (`WatcherDegraded`, `WatcherRecovered`) to JSON events; the pipeline
  currently turns them into warnings only.
- A pending change at shutdown is dropped with the process, which is the point of stopping.
