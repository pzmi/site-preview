# 0051. The server handle: start-up and shutdown driven by the domain lifecycle machine

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

## Context

plan.md §9 fixes the start-up order (validate the root, open it, bind, initialize the reload
state, build the routes, start the watcher, report readiness, serve) and the shutdown order
(listener, event streams, watcher, tasks), and foundation.md §6.1 item 1 asks that a failed
start-up leave nothing allocated, that shutdown be idempotent and that every owned task be
released after a stop. DOM-09 wrote these rules as the pure `Lifecycle` machine
(`LifecycleState`, `LifecycleInput`, `LifecycleEffect`), but composition's `run` performed the
steps as one straight-line function that the machine never saw, so nothing observable tied
the running server to the machine `lifecycle.qnt` will model (FORMAL-01, FORMAL-05). Start-up
refusals were one `Root` variant, so a reserved `__site_preview` entry could not be told from
a missing root by a caller. A watcher that panicked while starting would have unwound through
`run` and ended the process, although plan.md §3 and §8.4 make a watcher failure degraded
mode, never an exit.

## Decision

- **`Server` in `preview-composition/src/lifecycle.rs` owns the machine.** `Server::start`
  feeds `Start` and performs each effect the machine returns, in order; each step's outcome is
  the next input (`Bound`/`BindFailed`, `WatcherReady`/`WatcherFailed`). `Server::stop` feeds
  `Stop` and performs `CloseListener`, `CloseStreams`, `StopWatcher`, `JoinTasks` as returned,
  then `Stopped`; a later call gets no effects and returns the first outcome. `state()`
  exposes the machine's state for conformance. The binary's `run` is `Server::start` with the
  signal handlers installed, then a stop at the first signal.
- **Preconditions come before `Start`.** The bind scope, the root and the runtime are checked
  or acquired before the machine starts, so a refusal there leaves it `Stopped`: nothing was
  started. The signal handlers and the listener are acquired by `BindListener`; any failure
  there is `BindFailed`, whose `AbortStartup` returns the error. Every resource is an owned
  value of the start-up frame, so an early return drops (releases) all of them.
- **State and routes are built once the listener is held**, before the watcher, from the
  address actually bound.
- **A watcher that cannot watch anything, or panics while starting, is `WatcherFailed`.** The
  factory call is wrapped in `catch_unwind`; the panic payload is not shown (it may hold an
  absolute path). The source is closed, no pipeline task is spawned (the machine's
  `Serving { watcher: false }` owns no watcher and no debounce task), `Degraded` warns
  `watch-unavailable`, and the server serves.
- **A caller's `stop()` asks as SIGTERM does.** It sends `StopSignal::Terminate` to the serve
  task's stop channel unless a signal already closed the listener, so one stop path serves
  both; a second signal during the drain still forces the exit (ADR 0045).
- **`StartupError` names each refusal:** `RemoteBind`, `InvalidRoot`, `ReservedPath`,
  `Runtime`, `Signal`, `Bind`, each keeping its `io::Error` as `source()`. The CLI exits 2 for
  `RemoteBind` and 1 for every other (exit codes unchanged, ADR 0045); each message says what
  to do next.
- **Blocking watcher work stays off the async workers** (plan.md §9). Starting the watcher
  (the initial walk and every registration) runs on the caller's thread with the runtime
  entered, never inside `block_on` or a task. The pipeline's reconcile (ADR 0050), a walk
  plus watch calls that wait on the native watcher's thread, runs under `block_in_place` on
  the multi-thread runtime: the worker hands its other tasks to the pool first, and the
  pipeline still awaits the result before its next batch or deadline, so a new directory is
  watched before the revision announcing it is published. `spawn_blocking` was rejected: the
  source would have to move into the closure and back, and a panicked reconcile would lose
  it. A current-thread runtime (the paused-clock tests) runs the reconcile in line.
- **The watcher factory is injectable inside composition** (`Server::start_with`), so
  composition tests drive a failing or panicking watcher without the native one. `tempfile`
  becomes a composition dev-dependency for those tests (`architecture.toml` `[external.dev]`).

## Consequences

- The running server and the domain machine cannot drift silently: an input the machine
  refuses is a programming error that panics with the DOM-09 citation, and FORMAL-05 can
  observe `Server::state()` after each step.
- An M3 review finding (blocking reconcile on an async worker) is closed by the bullet above,
  pinned by `should_run_other_tasks_when_reconcile_blocks`, which fails (a 20 s
  bounded wait) when the reconcile runs in line on a one-worker runtime.
- A watcher start-up panic no longer ends the process; it prints the panic hook's message on
  stderr, then the `watch-unavailable` warning.
- `Server` is public, so a later task (CLI-05's signal handling, CLI-07's opener) can drive
  start and stop without the binary. Dropping a running `Server` stops it, waiting at most the
  drain bound.
- A `Stop` during start-up is not reachable yet: start-up runs on the caller's thread, and a
  signal received meanwhile is queued and stops the server once it serves. The machine's
  `Stopping { awaiting }` states stay for the model.
