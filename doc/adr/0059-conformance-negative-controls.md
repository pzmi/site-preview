# 0059. Conformance negative controls: faults behind the drivers' seams, a publish seam in the reload driver

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

## Context

Foundation.md section 6.3 step 7 asks for a negative control per conformance driver: one
component replaced by a deliberately faulty implementation, and the checker shown to catch
the precise bad transition. FORMAL-08 names four: `NonMonotonicReloadState`,
`NoDuplicateSuppressionCoordinator`, `LeakyLifecycle` and `GapIgnoringSse`. The drivers of
ADR 0056 to ADR 0058 already have seams for two of them (`ReloadDriver::with_router`,
`WatcherDriver::with_coordinator`), but not for the other two:

- The real `ReloadState` cannot publish a smaller or repeated revision, and its
  `from_state` constructor is `pub(crate)`; `AppState::reload` is the concrete type.
- The lifecycle observation probes the server's address, which the driver kept private, and
  a `Server::stop` that skips `CloseListener` deadlocks in `JoinTasks` (ADR 0056).
- A faulty SSE handler cannot reuse the adapter's private `ReloadPayload` and `Feed`, and an
  `axum` body built lazily needs a `Stream` type; `preview-conformance` had none.

## Decision

- **No production API changes.** Every fault lives in
  `crates/preview-conformance/tests/negative_controls.rs`, built from public API only.
- **A publish seam in the reload driver:** `ReloadDriver::with_publisher(Publisher)`, a boxed
  `FnMut(&mut ReloadState, ChangeSet)` that defaults to `ReloadState::publish_next`. It gets
  the handle itself, so `NonMonotonicReloadState` replaces the channel, on its second
  publish, with a fresh one for the same instance advanced to `n - 1` (a repeated revision).
- **`LifecycleDriver::address` is public** (test-only crate). `LeakyLifecycle` is a wrapper
  `Driver` that delegates to the real driver and, after each `stopServer`, binds the stopped
  server's address again and holds it: a listener that outlives the stop, observed through
  the real socket probe.
- **`GapIgnoringSse`** is a minimal handler in the test behind `with_router`: the real wire
  format, each value sent with its own changes and `full`, no gap check. `futures-util`
  (`default-features = false`, already in `Cargo.lock` through axum) becomes a
  dev-dependency of `preview-conformance` for `stream::unfold`; `architecture.toml` lists it
  under `[external.dev]`.
- **Watcher faults** are one `FaultyCoordinator` wrapping a real `ChangeCoordinator` built
  from the same window and reload state. Beyond the task's `NoDuplicateSuppression`, four
  cheap ones are kept: a leading-edge debouncer, a dropped overflow, an overflow without the
  rescan flag and an unmarked recovery.
- **Each control asserts the catch exactly:** trace, step, action, `Cause::Mismatch`, the
  seed, and the faulty observation equal to the model's projected state with only the
  faulted fields changed. Each replays its trace twice with fresh drivers and requires the
  same `Divergence`, so the same seed reproduces the same step.

| Control | Trace | Step | Action | Field |
|:--|:--|:--|:--|:--|
| `LeakyLifecycle` | `lifecycle-doubleStop` | 3 | `stopServer` | `listener_owned` true |
| `GapIgnoringSse` | `reload-coalescedCssThenJsFull` | 6 | `send` (`b1`) | last event kinds `{Js}`, `full` false |
| `NonMonotonicReloadState` | `reload-coalescedCssThenJsFull` | 5 | `publish` | `committed.revision` 1, not 2 |
| `NoDuplicateSuppression` | `watcher-burstOneReload` | 4 | `receive` | `revision` 1, a publish of `/a.css` |
| leading-edge debouncer | `watcher-atomicSave` | 2 | `receive` | published at once, not dirty |
| dropped overflow | `watcher-overflowRescan` | 5 | `receive` | not dirty, no reconcile |
| no rescan flag | `watcher-overflowRescan` | 9 | `settle` | `published.rescan` false |
| unmarked recovery | `watcher-recoveryAfterWatchLimit` | 14 | `settle` | `published.full` false |

## Consequences

- A driver that stops comparing (a constant `observe`, a projection that drops a field)
  fails a control, in the `unit-tests` stage.
- `NonMonotonicReloadState` swaps the channel, so connections opened before the swap watch
  the old one; the control's catch is at the publish step, before any stream is polled.
- A change to a named trace that moves a catch point changes this table and the control in
  the same commit; FORMAL-09 runs the controls as part of `cargo xtask conformance`.
