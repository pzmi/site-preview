// -*- mode: Bluespec; -*-
/**
 * Named runs of the models (foundation.md section 6.2 (c), section 6.4). One module per
 * model, so each model's state variables stay apart; run one with
 * `quint test --main <module> --match '.*' scenarios.qnt`.
 *
 * Every step of every run is followed by `allInvariants`, then by what that step must show.
 */

/// The composition's `Server` (`serverStep` actions only): the runs FORMAL-05 exports to
/// `formal/traces/regression/` and replays against the real server.
module lifecycleScenarios {
  import lifecycle.* from "./lifecycle"


  /// Start with a working bind and watcher, answer a request and hold an event stream, stop
  /// in plan.md section 9's order, then refuse requests.
  run startServeStop =
    init
      .then(startServerWith(true, true))
      .expect(and {
        allInvariants,
        state == Serving(true),
        inputs == [InStart, InBound, InWatcherReady],
        effects == [BindListener, StartWatcher, AnnounceReady],
        res == { ...NOTHING, listeners: 1, watchers: 1, serveTasks: 1, debounceTasks: 1 },
      })
      .then(request)
      .expect(allInvariants and response == Answered)
      .then(openStream)
      .expect(allInvariants and response == Answered and res.streams == 1)
      .then(stopServer)
      .expect(and {
        allInvariants,
        state == Stopped,
        inputs == [InStop, InStopped],
        effects == [CloseListener, CloseStreams, StopWatcher, JoinTasks],
        res == NOTHING,
      })
      .then(request)
      .expect(allInvariants and response == Unanswered)

  /// The bind fails: start-up aborts, nothing is held, nothing answers, and `Failed` is
  /// terminal (a stop is a no-op, a start is refused).
  run bindFailure =
    init
      .then(startServerWith(false, true))
      .expect(and {
        allInvariants,
        state == Failed,
        inputs == [InStart, InBindFailed],
        effects == [BindListener, AbortStartup],
        res == NOTHING,
      })
      .then(request)
      .expect(allInvariants and response == Unanswered)
      .then(stopServer)
      .expect(allInvariants and state == Failed and effects == [])
      .expect(not(transition(state, InStart).accepted))

  /// The watcher cannot start: the server still serves, degraded, with no watcher and no
  /// debounce task; its stop has no `StopWatcher`.
  run watcherFailsDuringStart =
    init
      .then(startServerWith(true, false))
      .expect(and {
        allInvariants,
        state == Serving(false),
        inputs == [InStart, InBound, InWatcherFailed],
        effects == [BindListener, StartWatcher, Degraded, AnnounceReady],
        res == { ...NOTHING, listeners: 1, serveTasks: 1 },
      })
      .then(request)
      .expect(allInvariants and response == Answered)
      .then(stopServer)
      .expect(and {
        allInvariants,
        state == Stopped,
        effects == [CloseListener, CloseStreams, JoinTasks],
        res == NOTHING,
      })

  /// A second `stop()` does nothing: no input past `InStop`, no effect, same state.
  run doubleStop =
    init
      .then(startServerWith(true, true))
      .then(openStream)
      .then(stopServer)
      .expect(allInvariants and state == Stopped and res == NOTHING)
      .then(stopServer)
      .expect(and {
        allInvariants,
        state == Stopped,
        inputs == [InStop],
        effects == [],
        res == NOTHING,
      })

  /// A refusal before `Start` (missing or reserved root, remote bind, runtime) leaves the
  /// machine `Stopped` with nothing held; a later start works.
  run refusedBeforeStart =
    init
      .then(refuseStart)
      .expect(allInvariants and state == Stopped and inputs == [] and res == NOTHING)
      .then(startServerWith(true, true))
      .expect(allInvariants and state == Serving(true))

  /// A clean stop, then a new start (a new `Server` in the composition): the same start-up.
  run restartAfterStop =
    init
      .then(startServerWith(true, true))
      .then(stopServer)
      .then(startServerWith(true, true))
      .expect(and {
        allInvariants,
        state == Serving(true),
        effects == [BindListener, StartWatcher, AnnounceReady],
        res == { ...NOTHING, listeners: 1, watchers: 1, serveTasks: 1, debounceTasks: 1 },
      })
}

/// The domain machine's contract (`step` actions, one input each), including the stops that
/// meet a start-up step in flight. The composition cannot reach these today (start-up is
/// synchronous, ADR 0051), so they are not replayed against the `Server`; the domain tests
/// in `preview-domain/src/lifecycle.rs` pin the same rows.
module lifecycleMachineScenarios {
  import lifecycle.* from "./lifecycle"


  /// A stop while binding: nothing is closed yet; the late listener is closed when the bind
  /// reports, and only then does the shutdown complete.
  run stopDuringBind =
    init
      .then(start)
      .then(stop)
      .expect(allInvariants and state == Stopping(AwaitBind) and effects == [] and res == NOTHING)
      .expect(not(transition(state, InStopped).accepted))
      .then(bound)
      .expect(and {
        allInvariants,
        state == Stopping(NoStep),
        effects == [CloseListener],
        res == NOTHING,
      })
      .then(stopped)
      .expect(allInvariants and state == Stopped)

  /// A stop while the watcher starts: the listener closes at once; a late watcher is stopped
  /// and its debounce task joined when it reports.
  run stopDuringWatcherStart =
    init
      .then(start)
      .then(bound)
      .then(stop)
      .expect(and {
        allInvariants,
        state == Stopping(AwaitWatcher),
        effects == [CloseListener],
        res == NOTHING,
      })
      .then(watcherReady)
      .expect(and {
        allInvariants,
        state == Stopping(NoStep),
        effects == [StopWatcher, JoinTasks],
        res == NOTHING,
      })
      .then(stopped)
      .expect(allInvariants and state == Stopped)

  /// A second stop while the first one's shutdown has not completed is a no-op for the
  /// machine (the composition's forced drain happens inside `JoinTasks`, below the model).
  run doubleStopDuringDrain =
    init
      .then(start)
      .then(bound)
      .then(watcherReady)
      .then(stop)
      .expect(allInvariants and state == Stopping(NoStep) and res == NOTHING)
      .then(stop)
      .expect(allInvariants and state == Stopping(NoStep) and effects == [])
      .then(stopped)
      .expect(allInvariants and state == Stopped)
}
