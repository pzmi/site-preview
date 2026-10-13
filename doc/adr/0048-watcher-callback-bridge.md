# 0048. The watcher's callback bridge: bounded queue, loss and failure flags, adapter-side filtering

Date: 2026-10-12

## Status

Proposed (planner default — owner may override). The order of `next` is amended by
[ADR 0067](0067-root-removal-and-watcher-review-fixes.md): the start-up registration failure
comes first, before `Overflow`.

## Context

plan.md §8.3 (last paragraph) bridges the synchronous `notify` callback into the async watcher
task with a bounded channel and an overflow flag, and forbids blocking the native callback or
silently losing all change notification. The `ChangeSource` contract (ADR 0038 and its
amendment) requires `Overflow` before any change observed after a loss, `Failed` with a
`SourceFailure` category for a registration or runtime failure, `Closed` as final, and a
cancellation-safe `next`. inotify reports every `open` and `close` of a file in a watched
directory, so serving a page produces access-only notifications; FSEvents and inotify also
report attribute changes. The domain ignores both kinds (plan.md §8.2).

## Decision

- `NotifyChangeSource` holds the `NotifyWatcher`, the receiving half of a bounded
  `tokio::sync::mpsc` channel (default capacity 1024, configurable with
  `start_with_capacity`), and two flags shared with the callback: overflow and failure, plus a
  `tokio::sync::Notify` that wakes the consumer. `preview-adapter-fs` takes `tokio` with the
  `sync` feature only; no runtime is needed to poll the source.
- The callback (`Bridge`) translates each event and drops access-only and metadata-only
  changes before queueing, so they never use capacity. It sends with `try_send`. A full
  channel, or a notification carrying `notify`'s `Rescan` flag (an OS queue overflow), sets
  the overflow flag and wakes the consumer. A runtime error is queued as
  `Failed(RuntimeFailure)` with a `RuntimeError` detail (category and root-relative path);
  if the channel is full it sets the failure flag as well as the overflow flag.
- `next` reports, in order: `Closed` once closed; `Overflow` while the overflow flag is set;
  `Failed(RuntimeFailure)` while the failure flag is set; then queued messages, consecutive
  changes merged into one `Events` batch (at most one channel's worth) and a queued failure
  delivered alone. A message received while the overflow flag is set is held back until the
  `Overflow` has been delivered, so a change is never delivered ahead of the report of a loss
  that preceded it; changes queued before the loss may then follow the `Overflow`, which only
  repeats what the rescan covers. State lives in the source, never in the future, so dropping
  a pending `next` loses nothing.
- `start` never fails. An incomplete registration (a walk or watch error, or no watcher at
  all) is reported in `StartReport` and delivered first as `Failed(RegistrationFailed)`, so
  the coordinator degrades while HTTP keeps serving. Without a watcher the channel's producer
  is gone, so `Closed` follows. `close` removes every watch; every later `next` is `Closed`.
- `NotifyWatcher` and `translate` are crate-private: `NotifyChangeSource`, `StartReport`,
  `RegistrationError`, `RuntimeError` and `WatchFault` are the adapter's watcher API, and no
  `notify` type appears in it.

## Consequences

- A failure that found the channel full is delivered without its detail
  (`last_failure()` is `None`), and possibly before changes queued ahead of it; the category
  is what the coordinator needs.
- CLI-03 consumes the source in the watch task, prints `StartReport` and `last_failure` as
  warnings, and treats `Closed` after a failed start as "watching is off for good".
- TEST-05 runs the shared contract against this source; FS-09 adds reconciliation through the
  watcher the source holds.
