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

/// The live-reload protocol (`reload.qnt`, FORMAL-03). The server side of each run (the
/// committed state and each connection's events) is what FORMAL-07 exports and replays
/// against the real `ReloadState` and SSE handler; the browser side is pinned by
/// `reconnect.spec.ts` and `hot.spec.ts`, named on each run.
module reloadScenarios {
  import reload.* from "./reload"

  pure val B = "b1"
  pure def at(instance: int, revision: int): Stamp = { instance: instance, revision: revision }
  def page: Browser = browsers.get(B)
  def sent: List[Event] = browsers.get(B).stream

  /// A source edit lands between the HTML fetch and the event stream's connect: the first
  /// `state` differs from the render-time baseline, so the page reloads once, and the reloaded
  /// page is quiet (plan.md section 7.3, `reload.spec.ts`).
  run editDuringFetch =
    init
      .then(navigate(B))
      .expect(allInvariants and page.baseline == at(1, 0))
      .then(publishWith(Set(Css), false))
      .then(connect(B))
      .expect(allInvariants and sent == [stateEvent(at(1, 1))])
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Navigating,
        page.reloaded == 1,
        page.hotApplied == 0,
        not(page.open),
      })
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Shown,
        page.baseline == at(1, 1),
        page.reloaded == 1,
      })

  /// The stream drops, a css-only edit is published while it is down, and the stream comes
  /// back: the change set of the missed revision is gone, so the reconnect's `state` costs one
  /// full reload, never a hot apply, and an unchanged server none
  /// (`should_reload_once_when_stream_reconnects_after_an_edit_made_while_it_was_down`,
  /// `should_not_reload_when_stream_reconnects_to_an_unchanged_server`).
  run disconnectEditReconnect =
    init
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .then(disconnect(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .expect(allInvariants and page.doc == Shown and page.reloaded == 0)
      .then(disconnect(B))
      .expect(allInvariants and not(page.open) and sent == [])
      .then(publishWith(Set(Css), false))
      .then(connect(B))
      .expect(allInvariants and sent == [stateEvent(at(1, 1))])
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Navigating,
        page.reloaded == 1,
        page.hotApplied == 0,
        page.trigger == at(1, 1),
      })
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .expect(allInvariants and page.doc == Shown and page.reloaded == 1)

  /// A clean stop and a new process on the same port with unchanged files: the new instance
  /// at revision 0 makes the page reload exactly once, and the reloaded page stays
  /// (`should_reload_once_and_stay_when_server_restarts_on_the_same_port`); the revision
  /// number alone (0 on both) would not have shown it.
  run restartSamePort =
    init
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .then(stopServer)
      .expect(allInvariants and not(up) and not(page.open) and page.doc == Shown)
      .then(startServer)
      .expect(allInvariants and committed == at(2, 0))
      .then(connect(B))
      .expect(allInvariants and sent == [stateEvent(at(2, 0))])
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Navigating,
        page.reloaded == 1,
        page.trigger == at(2, 0),
      })
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Shown,
        page.baseline == at(2, 0),
        page.reloaded == 1,
        page.open,
      })

  /// A css-only revision right after the connection's baseline is sent hot and applied in
  /// place: the baseline moves to it with no navigation
  /// (`should_swap_stylesheet_without_navigation_when_css_changes`).
  run cssOnlyHot =
    init
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .then(publishWith(Set(Css), false))
      .then(send(B))
      .expect(and {
        allInvariants,
        sent == [
          stateEvent(at(1, 0)),
          { name: ReloadEvent, at: at(1, 1), kinds: Set(Css), full: false },
        ],
      })
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Shown,
        page.open,
        page.baseline == at(1, 1),
        page.hotApplied == 1,
        page.reloaded == 0,
      })

  /// A css revision and a js revision published before the connection's `Feed` wakes collapse
  /// into the newest: the css change set is gone, so the event is `full` with no changes and
  /// the page reloads once, never hot-applying either
  /// (`should_reload_once_when_css_and_script_change_in_quick_succession`,
  /// `should_send_full_when_revisions_coalesced`).
  run coalescedCssThenJsFull =
    init
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .then(publishWith(Set(Css), false))
      .then(publishWith(Set(Js), false))
      .then(send(B))
      .expect(and {
        allInvariants,
        sent == [
          stateEvent(at(1, 0)),
          { name: ReloadEvent, at: at(1, 2), kinds: Set(), full: true },
        ],
      })
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Navigating,
        page.reloaded == 1,
        page.hotApplied == 0,
      })
      .then(send(B).fail())

  /// An html change is published full and sent full even on a contiguous connection (the
  /// path is listed for information only), and the page reloads once
  /// (`should_navigate_exactly_once_and_stay_stable_when_html_changes`,
  /// `should_reload_once_when_html_and_css_change_together`).
  run htmlFull =
    init
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .then(publishWith(Set(Html, Css), false))
      .expect(allInvariants and latest == { kinds: Set(Html, Css), full: true })
      .then(send(B))
      .expect(and {
        allInvariants,
        sent[1] == { name: ReloadEvent, at: at(1, 1), kinds: Set(Html, Css), full: true },
      })
      .then(receiveWith(B, true))
      .expect(and {
        allInvariants,
        page.doc == Navigating,
        page.reloaded == 1,
        page.hotApplied == 0,
      })

  /// A script change the page has not accepted is a hot change set the client cannot apply
  /// in place: it falls back to one reload with the scroll position kept, and the hot
  /// counter stays put (`should_reload_once_when_script_change_is_not_accepted`); an
  /// accepted one is applied with no navigation
  /// (`should_run_handler_without_navigation_when_page_accepts_script_change`).
  run scriptNotAcceptedReloads =
    init
      .then(navigate(B))
      .then(connect(B))
      .then(receiveWith(B, true))
      .then(publishWith(Set(Js), false))
      .then(send(B))
      .then(receiveWith(B, true))
      .expect(allInvariants and page.hotApplied == 1 and page.baseline == at(1, 1))
      .then(publishWith(Set(Js), false))
      .then(send(B))
      .then(receiveWith(B, false))
      .expect(and {
        allInvariants,
        page.doc == Navigating,
        page.hotApplied == 1,
        page.reloaded == 1,
        page.trigger == at(1, 2),
      })
}
