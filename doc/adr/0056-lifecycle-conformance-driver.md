# 0056. Lifecycle conformance driver: composition seams, scripted named traces, a sampling relation

Date: 2026-10-12

## Status

Accepted (owner, 2026-10-14). Amended by ADR 0070: the observation also
compares the open event streams (`res.streams`) and the outcome of each stop (`Drained`), and
the server is started through a seam a negative control can wrap.

## Context

FORMAL-05 replays `formal/lifecycle.qnt`'s `serverStep` traces against the real composition
`Server` (ADR 0051) through FORMAL-04's replay core (ADR 0053), observing
`{state, listenerOwned, tasksAlive}` after every step. Four things stood in the way:

- foundation.md section 2.1 lets `preview-conformance` depend on the application, the domain,
  the testkit and the concrete adapters; the `Server` lives in `preview-composition`, which
  `architecture.toml` did not list for it.
- The watcher factory that composition's own tests inject is `pub(crate)`, and so are the
  `WatchControl` trait and the opened root; nothing outside the crate can make the watcher
  fail. The native watcher cannot be made to fail on demand (the sandbox runs as root, and
  inotify limits are shared with the whole machine). The tasks the server owns are private
  fields, and the listener is moved into the serve task.
- `quint test --out-itf` writes the named runs of `scenarios.qnt` without `mbt::actionTaken`
  and `mbt::nondetPicks`, which the replay needs; only `quint run --mbt` writes them.
- `Failed` is terminal, and `startServer` picks a failing bind half the time, so about 60% of
  the steps of 25-step `serverStep` traces are spent in `Failed`, where nothing changes.

## Decision

- **`preview-conformance` may depend on `preview-composition`** (`architecture.toml`
  `[dependencies.normal]`; `tempfile` as its dev-dependency). It stays `test_only`: no
  production crate reaches it, which the architecture check still enforces.
- **Two `#[doc(hidden)]` seams on `Server`, outside the supported API:**
  `start_with_unavailable_watcher` starts the native watcher and reports it unable to watch
  anything (the server closes it and serves degraded, as when the operating system refuses
  it), and `owns_tasks` says whether a serve, pipeline or forwarder task is still owned, not
  yet joined. A cargo feature was rejected: feature unification would turn it on for the
  binary whenever the workspace is built together.
- **The observation is read from the real code:** `state` is `Server::state()`, or the
  `StartupError` variant when a start returns no server (`Bind` is `Failed`, a missing root
  is `Stopped`, anything else is a driver refusal); `listenerOwned` is whether the server's
  address still completes a TCP handshake (a closed loopback listener refuses it);
  `tasksAlive` is `owns_tasks()`. The driver also observes `response` (a `200` head or not),
  so the model's `noResponsesWhenStopped` is checked against real sockets. A bind failure is
  a port the driver already holds; a refused start is a root that does not exist.
- **The named traces are scripted:** `formal/lifecycle_traces.qnt` restates each `run` of
  `lifecycleScenarios` as a step relation over a program counter, exported with `quint run
  --mbt --seed 0x5ce1 --invariant allInvariants` into
  `formal/traces/regression/lifecycle-<run>.itf.json` (the command is in that file). Its
  `startServer(bind, watcher)` records the picks under `serverStep`'s names.
- **`serverStepMostlyBound`** in `lifecycle.qnt` is `serverStep` with `startServerBound`
  (`startServer` with `bindOk` true) listed three more times in its `any`: the same
  transitions, sampled so that a start fails its bind one time in eight. Generated traces
  come from it (30 traces of 25 steps, seed printed, `QUINT_SEED` repeats it), and
  `formal-simulate` checks every invariant under it too.
- The generated-trace test runs the locked Quint (`npx --prefix formal --no`), so the
  `unit-tests` stage needs `npm ci --prefix formal`; CI installs it before the gate. A
  missing Quint fails with that command.

## Consequences

- The driver observes real behaviour: a serve task that never closes its listener, or a
  stop that leaves the pipeline task unjoined, diverges at the `stopServer` step of
  `lifecycle-doubleStop` (the first named trace with a stop). A `Server::stop` that skips
  `CloseListener` altogether instead hangs in `JoinTasks` (the serve task is never told to
  stop), which only nextest's timeout ends; FORMAL-08's `LeakyLifecycle` should leak behind
  the driver rather than inside `Server::stop`.
- A script and its `run` can drift; the replay catches a drift only where the real server
  disagrees. FORMAL-09's freshness check regenerates the scripts with their seed, and a
  change to a run changes its script in the same commit.
- `listenerOwned` after a stop probes a port the server gave back; another process binding
  that exact port in the same instant would read as a leak. Loopback ports from port 0 are
  chosen at random, so this is unlikely, and it errs towards a false failure, never a pass.
- The `Stopping(Await*)` states stay unreachable through `Server` (ADR 0051); the driver
  never sees them, and `lifecycleMachineScenarios` is not replayed.
