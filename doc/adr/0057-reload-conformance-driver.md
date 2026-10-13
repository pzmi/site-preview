# 0057. Reload conformance driver: in-process SSE bodies, the client's choice read from the model

Date: 2026-10-12

## Status

Proposed (planner default — owner may override). Amended by ADR 0070: `apply_with_model`
receives a `ModelView` holding only the paths the driver declares in `Driver::MODEL_READS`
(`browsers.*.doc` here), not the whole state.

## Context

FORMAL-07 replays `formal/reload.qnt`'s `step` traces against the real `ReloadState` and the
real event-stream handler, in process with a paused clock (ADR 0017, D16), through
FORMAL-04's replay core (ADR 0053). Three things stood in the way:

- The driver needs a runtime with a paused clock, the adapter's `Router` as a `tower` service
  and its response body as an `http_body::Body`. `architecture.toml` allowed
  `preview-conformance` only `serde_json` as a normal external dependency.
- Some of the model's browser actions reach the server and some do not. `connect`, `send` and
  `disconnect` are the stream's own; `navigate` never touches an open stream. But `receive`
  closes the stream exactly when the client decides to reload (`decide` in client.js), and
  D16 forbids reimplementing that decision in Rust. `Driver::apply` gets the action and its
  picks only, and the picks (`b`, `applied`) do not say what the client decided.
- `quint test --out-itf` writes the runs of `reloadScenarios` without `mbt::actionTaken` and
  `mbt::nondetPicks` (ADR 0056).

## Decision

- **New normal dependencies of `preview-conformance`**, each already in `Cargo.lock` through
  the HTTP adapter and composition: `tokio` (`rt`, `time`, `test-util`, for a current-thread
  runtime started paused), `axum` (no features: `Body`, `Router`, `HttpBody`) and `tower`
  (`util`: `ServiceExt::oneshot`). `architecture.toml` lists them under
  `[external.normal]`. The crate stays `test_only`; no workspace edge changes.
- **`Driver::apply_with_model`**, a provided method of the replay core's `Driver` trait,
  receives the model's state after the step (since ADR 0070, only the paths the driver
  declares); `replay` calls it, and its default calls
  `apply`, so the lifecycle driver is unchanged. A driver reads from it only a choice the
  environment made outside the implementation. The reload driver reads one thing: after a
  `receive`, whether the model's page is `Navigating` (the client reloaded), and if so drops
  that browser's body. Everything it then observes is read from the real code.
- **The stimulus map:** `publish` and `publishHot` call `ReloadState::publish_next` with a
  `ChangeSet` accumulated from synthetic `ChangeEvent`s (one `Modify` per kind, `a.css`,
  `a.js`, `a.html`, `a.png`, `a.txt`; the full reason is an `Ambiguous` event on the root,
  a rescan that records no path); `connect` sends `GET /__site_preview/events` through the
  real `build_router` and polls the first event; `send` polls the body once without waiting;
  `disconnect` and `userReload` drop the body; `stopServer` fires the `Shutdown` and polls
  every open body once (an ended body is a closed connection); `startServer` builds a new
  `ReloadState` for a new instance id. A body is polled only at those steps, so publishes in
  between coalesce in the real `Feed` where the model's `send` coalesces them. The router is
  built by a `fn(AppState) -> Router` the driver takes (`ReloadDriver::with_router`), so a
  negative control can put a faulty handler behind the same seam (FORMAL-08).
- **The observation** is `up` (the driver's own record: in process there is no listener),
  `committed` (`ReloadState::current`), `latest` (the kinds and `is_full` of a new
  subscription's initial value) and, per open connection, the events its body yielded,
  parsed from the wire into name, instance, revision, kinds and `full`. Instances are
  numbered by creation order, as the model numbers them; an id the driver never created is
  instance 0. The browser's own state (`doc`, `baseline`, counters) and the ghosts are left
  out (D16).
- **The named traces are scripted** in `formal/reload_traces.qnt`, one per `run` of
  `reloadScenarios`, exported with `quint run --mbt --seed 0x5ce1` into
  `formal/traces/regression/reload-<run>.itf.json` (the command is in that file). Its
  wrappers record fixed arguments as `step`'s picks under the model's action names.
  Generated traces come from `step` itself: 30 traces of 25 steps, seed printed,
  `QUINT_SEED` repeats it.

## Consequences

- The server layer of `noHotApplyAcrossGap` is checked against the real handler: with
  `ReloadPayload::after` ignoring the revision gap, the replay of
  `reload-coalescedCssThenJsFull` diverges at step 6, `send` (a hot `js` event where the
  model sends `full` with no changes). A stream that ignores the shutdown signal diverges at
  step 4 of `reload-restartSamePort`, `stopServer` (pinned by a negative control since
  ADR 0070).
- `apply_with_model` makes the comparison vacuous if a driver reads from the model what it
  then observes. Since ADR 0070 the type guards it: the driver gets a view of the paths it
  declares and nothing else. Only the reload driver overrides it, for the one client
  decision.
- `up` and the instance numbering are the driver's records, so a process that never stops or
  reuses an instance id is outside this driver: the lifecycle driver (ADR 0056) observes the
  listener, while the composition's `server::instance_id` (process id and start time) is
  not exercised here: the driver's ids come from the testkit's `FixedInstanceIds`.
- A script and its `run` can drift; as with the lifecycle scripts (ADR 0056), a change to a
  run changes its script in the same commit, and FORMAL-09's freshness check regenerates them.
