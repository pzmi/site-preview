# 0070. Conformance harness review fixes: a declared model view, the pipeline's shared decision step, the stop outcome, per-field sensitivity

Date: 2026-10-13

## Status

Proposed (planner default — owner may override)

Amends [ADR 0056](0056-lifecycle-conformance-driver.md) (the lifecycle observation),
[ADR 0057](0057-reload-conformance-driver.md) (`Driver::apply_with_model`),
[ADR 0058](0058-watcher-conformance-driver.md) (what the watcher driver runs) and
[ADR 0059](0059-conformance-negative-controls.md) (the controls and what they prove).

## Context

The FORMAL-10 checkpoint review found, in `preview-conformance`:

- **The lifecycle stop outcome was discarded.** The driver called `Server::stop` and dropped
  the `Stopped` it returned, and observed nothing of the event streams it held. A server whose
  `CloseStreams` ended nothing still replayed every trace: its drain waited out the bound,
  returned `DrainTimedOut`, and then dropped the connections, so state, listener and tasks
  all matched the model. `noResponsesWhenStopped` and `tasksReleasedAfterStop` had no
  negative control.
- **`Driver::apply_with_model` handed the driver the whole model state** (ADR 0057). Only
  review stood between a driver and reading the variable it then "observes", which makes the
  comparison vacuous.
- **The watcher driver restated the pipeline.** ADR 0058's driver re-implemented
  `WatchPipeline`'s receive, settle and reconcile decisions (about 60 lines that had to be
  kept equal by hand), swallowed a refused `watcher_reconciled` as the pipeline does, and ADR
  0058 and `doc/formal-models.md` said the lifecycle driver covers the pipeline, which it
  does not: the lifecycle observation sees no revision, health or reconcile.
- **The controls' claim was wider than their evidence.** ADR 0059 said a driver that stops
  comparing fails a control, but the controls fault only some fields: lifecycle `state`,
  `tasks_alive` and `response`, reload `up` and `latest`, and watcher `health`, `events` and
  `recorded` could be dropped from the comparison without any control failing.
- Smaller gaps: no check that a script restates its run; the reload driver read only the
  first event of a frame and took a failing body for a closed one; the lifecycle probe could
  wait for the operating system's connect timeout; a replay accepted a trace cut short;
  ADR 0057's claim about a stream that ignores the shutdown had no control.

## Decision

- **The lifecycle observation gains `streams_open` and `stopped`.** `streams_open` is how
  many event streams the driver opened that have not ended; after a stop each held socket is
  read until the server closes it (end of file or reset, within the reply deadline), and it
  projects to the model's `res.streams`. `stopped` is what `Server::stop` returned at a
  `stopServer` step and `None` at any other; the model has no drain (its `CloseStreams` ends
  every stream before `JoinTasks`), so its stop projects to `Drained`, and a stop of no server
  drains nothing and is `Drained` too. Probes connect with `connect_timeout`.
- **A server seam in the lifecycle driver:** `LifecycleDriver::with_server` takes a
  `StartServer` function returning a `ServerHandle` (`state`, `owns_tasks`, `local_addr`,
  `stop`), the real `Server` by default. Two controls wrap the real server behind it:
  `StreamOutlivesShutdown` (a response that never finishes, as an event stream that
  `CloseStreams` failed to end, so the drain times out) and `TaskOutlivesStop` (a thread the
  stop never joins). A third, `AnswersAfterStop`, wraps the driver as `LeakyLifecycle` does
  and answers a request after the stop. The production server cannot be made to skip its
  own effects from outside, so the faults are emulated behind the seam; the mutation that
  makes `CloseStreams` and the serve task's trigger no-ops in composition was also run and
  diverges at the same step (`lifecycle-doubleStop`, step 3, `Drained` against
  `DrainTimedOut`).
- **A declared model view.** `Driver::MODEL_READS` (an associated constant, empty by
  default) lists the model paths a driver may read, as patterns: a variable, then record
  fields and string map keys, `*` for every key of a map. `replay` builds a `ModelView` from
  each state that holds those paths and nothing else, and `apply_with_model` receives the
  view, not the `State`. `ModelView::read` refuses (`DriverError::Refused`) a path no pattern
  covers. The reload driver declares `browsers.*.doc` only. ADR 0057's "review is the guard"
  is replaced by the type.
- **The pipeline's synchronous decision step is shared.** `WatchDecisions<C>` in
  `preview-composition`'s `watch_pipeline.rs` owns the coordinator and the `reconciled_fully`
  flag and does what the pipeline did inline: `receive` (hand the batch over at the clock's
  time, reconcile at once when asked, `Full` after an overflow), `settle` (hand the deadline
  over; a full reconcile for a rescan revision unless one ran since the previous revision)
  and the reconcile report (root removal, problems then a registration failure, or
  readiness). It takes the reconcile as a closure and returns a `StepReport` of what happened
  in order (domain events, problems, a refused readiness). `WatchPipeline` keeps the async
  loop, the timer, the source, `block_in_place` and the warnings and calls it; the watcher
  driver calls the same step with the reconcile the model's environment reports. A
  `PipelineCoordinator` trait (the four commands the step issues) lets a negative control's
  coordinator stand behind it; the driver's `Coordinator` extends it with `watcher_ready`,
  `health` and `deadline`. All of these are `#[doc(hidden)]`, so `ARCH-hidden-seam` keeps
  them to composition and conformance. A refused readiness is reported, not swallowed: the
  pipeline still carries on (only revision exhaustion, already reported, causes it), and the
  driver turns it into `DriverError::Refused`, since the model never refuses one. Behaviour is
  unchanged: the model is untouched, every committed trace is fresh, and the replays and
  controls pass.
- **Per-field sensitivity** (`tests/field_sensitivity.rs`): for each driver, each projected
  field is edited at one state of a committed trace on the model's side, and the replay must
  stop at exactly that state with a mismatch in that field alone. Lifecycle: `state`,
  `listener_owned`, `tasks_alive`, `streams_open`, `response` (`stopped` is projected from the
  step's action and pinned by `StreamOutlivesShutdown` instead). Reload: `up`, `committed`,
  `latest`'s kinds and fullness, a stream's presence and an event. Watcher: `revision`,
  `dirty`, the deadline, `health`, `recorded`, `needs_reconcile`, `events` and each part of
  `published`.
- **Smaller fixes.** An xtask unit test compares each script of a `*_traces.qnt` file with
  its run in `scenarios.qnt`, action by action (the model's qualifier dropped, each
  pick-recording wrapper named after the action it wraps, `.fail()` steps left out). The
  reload driver's parse refuses a frame holding more than one event, and a body that fails is
  a `DriverError::Refused`, never an ended connection. The replay refuses a trace whose state
  count is not its manifest's `max_steps` plus one (`Trace::with_max_steps`; the committed
  traces read theirs from `manifest.toml`, the generated ones from their family). A control
  pins ADR 0057's shutdown claim: a stream that ignores the shutdown diverges at step 4 of
  `reload-restartSamePort`, `stopServer`.

## Consequences

- What the conformance replay tests of the watcher is now the coordinator and the
  pipeline's decision step; what it does not test is the async shell (the timer and its
  rounding, polling the source, cancellation, `block_in_place`, warnings), the adapter's
  reconcile and the bridge's `next`, which the driver restates from the model's environment.
  Those are covered by composition's and the adapter's own tests, not by the lifecycle
  driver.
- A new driver that needs the model declares the paths in `MODEL_READS`, where review sees
  them in one line; a read outside them fails the replay.
- The lifecycle stop is now compared, so a drain that times out on a correct trace fails the
  replay. The drain bound is 3 s and every connection the driver opens ends at the stop, so
  only a server fault (or a machine too loaded to close a loopback socket in 3 s) does that.
- `StreamOutlivesShutdown` writes a 64 MiB file into the control's temporary site, as
  composition's own drain test sends a 64 MiB body.
- Changing a script without its run, or a run without its script, fails `cargo xtask
  check`'s unit tests before any trace is regenerated.
- The pipeline reports a step after it has run, so the warnings and
  `WatcherEvents::changed` for the events the coordinator returns on receiving a batch now
  come after the reconcile that batch asked for, not before it as the inline pipeline
  emitted them. Every
  event still comes once and in the coordinator's order, and the reconcile changes nothing a
  user sees in between (the revision waits for the window), so the order was kept as is
  (OPS-04 review); restoring it would need a reporting callback inside `WatchDecisions`.
