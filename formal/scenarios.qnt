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

/// The watcher pipeline (`watcher.qnt`, FORMAL-02): the runs FORMAL-06 exports to
/// `formal/traces/regression/` and replays against the real `ChangeCoordinator`.
module watcherScenarios {
  import watcher.* from "./watcher"

  /// A burst of edits to one stylesheet, a duplicate notification and an edit of the page,
  /// received in two batches within the window: one revision, a full one (the page is a
  /// document), and nothing more once the deadline has passed.
  run burstOneReload =
    initWith(false)
      .then(modify(Css))
      .then(modify(Css))
      .then(duplicate(0))
      .expect(allInvariants and src.queue.length() == 3)
      .then(receive(1, false))
      .expect(allInvariants and deb.dirty and deb.deadline == WINDOW and ghost.cycles == 1)
      .then(tick)
      .then(modify(Index))
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        last.recorded == 3,
        deb.deadline == 1 + WINDOW,
        deb.paths == Set(Css, Index),
        deb.full,
        ghost.cycles == 1,
        not(due(deb, now)),
      })
      .then(tick)
      .then(tick)
      .expect(allInvariants and due(deb, now) and revision == 0)
      .then(settle(false))
      .expect(and {
        allInvariants,
        revision == 1,
        last.fired,
        last.firedFull,
        deb == CLEAN,
        ghost.pending == Set(),
        settled,
      })
      .then(tick)
      .then(tick)
      .expect(allInvariants and revision == 1 and not(deb.dirty))

  /// Git and editor swap-file churn is dropped by the bridge before the channel (ADR 0075):
  /// nothing is queued, so there is no batch to receive, the debouncer stays clean and no
  /// revision is ever published.
  run gitOnlyNoReload =
    initWith(false)
      .then(gitChange(true))
      .then(gitChange(true))
      .then(tempWrite)
      .then(gitChange(false))
      .expect(and {
        allInvariants,
        src.queue == [],
        not(src.overflow),
        deb == CLEAN,
        settled,
      })
      .then(tick)
      .then(tick)
      .then(tick)
      .expect(allInvariants and revision == 0 and not(due(deb, now)) and ghost.cycles == 0)

  /// Atomic saves: the temporary file's create, write and rename are dropped by the bridge
  /// (ADR 0075), the rename onto the target is queued and recorded (and asks for a reconcile,
  /// since a rename may move a directory). The page reloads in full; the stylesheet is swapped
  /// in place.
  run atomicSave =
    initWith(false)
      .then(saveAtomically(Index))
      .expect(allInvariants and src.queue == [MEvent(note(KRename, Index))])
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        last.recorded == 1,
        last.needsReconcile,
        deb.paths == Set(Index),
        deb.full,
        not(deb.rescan),
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and revision == 1 and last.firedFull and settled and ghost.pending == Set())
      .then(saveAtomically(Css))
      .then(receive(4, false))
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        revision == 2,
        not(last.firedFull),
        last.firedPaths == Set(Css),
        settled,
        ghost.pending == Set(),
      })

  /// Losses: the OS drops a notification (its queue overflowed), then the channel fills. Each
  /// loss is reported as `Overflow` before the changes queued after it, is a full rescan and
  /// reconcile, and settles in one full revision that covers the lost change.
  run overflowRescan =
    initWith(false)
      .then(kernelQueue(true))
      .then(modify(Css))
      .expect(allInvariants and src.overflow and src.queue == [])
      .then(kernelQueue(false))
      .then(modify(Index))
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        last.recorded == 0,
        deb.rescan,
        deb.full,
        reconciledFully,
        src.queue.length() == 1,
      })
      .then(receive(4, false))
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        revision == 1,
        last.firedRescan,
        last.firedFull,
        last.fullDone,
        not(reconciledFully),
        settled,
        ghost.pending == Set(),
      })
      // Five notifications into a channel of four.
      .then(modify(Css))
      .then(modify(Css))
      .then(modify(Sub))
      .then(modify(Index))
      .then(modify(Css))
      .expect(allInvariants and src.queue.length() == CAP and src.overflow)
      .then(receive(4, false))
      .expect(allInvariants and deb.rescan and last.recorded == 0)
      .then(receive(4, false))
      .expect(allInvariants and last.recorded == 4 and src.queue == [] and ghost.cycles == 2)
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and revision == 2 and last.firedFull and settled and ghost.pending == Set())

  /// `d` is deleted (one revision), then created again; a file written in the new `d` before
  /// its watch exists is not notified, but the directory change's full revision covers it. Once the reconcile
  /// has watched the new `d`, its edits are notified again and swapped in place.
  run dirRecreate =
    initWith(false)
      .then(dirDelete)
      .expect(allInvariants and src.watched == Set(Root) and src.queue.length() == 2)
      .then(receive(4, false))
      .expect(allInvariants and last.needsReconcile and deb.full and src.outstanding == Set())
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and revision == 1 and settled and ghost.pending == Set())
      .then(dirCreate(false))
      .then(createFile(Sub))
      .expect(allInvariants and src.queue.length() == 1 and ghost.pending == Set(Sub))
      .then(receive(4, false))
      .expect(allInvariants and src.watched == Set(Root, DirD) and health == HWatching)
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and revision == 2 and last.firedFull and settled and ghost.pending == Set())
      .then(modify(Sub))
      .expect(allInvariants and src.queue.length() == 1)
      .then(receive(4, false))
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        revision == 3,
        not(last.firedFull),
        last.firedPaths == Set(Sub),
        settled,
        ghost.pending == Set(),
      })

  /// A new `d` cannot be watched (the watch limit): the watcher degrades once, a change in `d`
  /// is unseen (excused, never silent), and when a watch is freed the next reconcile watches
  /// `d` and recovers once. The recovery marks the next revision full (ADR 0054), so the
  /// unseen change is caught up a window later, not left stale while live reload is claimed.
  run recoveryAfterWatchLimit =
    initWith(false)
      .then(dirDelete)
      .then(receive(4, false))
      .then(dirCreate(true))
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        health == HDegraded(RegistrationFailed),
        last.events == [EvDegraded(RegistrationFailed)],
        src.outstanding == Set(DirD),
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .then(createFile(Sub))
      .expect(allInvariants and ghost.excused == Set(Sub) and src.queue == [])
      .then(freeWatch)
      .then(createFile(Css2))
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        health == HWatching,
        last.events == [EvRecovered],
        src.watched == Set(Root, DirD),
        deb.full,
        not(deb.rescan),
        deb.paths == Set(Css2),
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        revision == 2,
        last.firedPaths == Set(Css2),
        last.firedFull,
        not(last.firedRescan),
        settled,
        ghost.pending == Set(),
        ghost.excused == Set(),
        reachesRecoveredCaughtUp,
        not(reachesRecoveredWithStaleChange),
      })

  /// The recovery alone makes the revision full: after a start-up failure, a change in `d` is
  /// unseen; the reconcile of an atomic save of a stylesheet (a `Changed` reconcile, the only
  /// notified change hot) watches `d` and recovers, and the revision a window later is full,
  /// so the unseen change is published. Through an overflow the set would be full anyway, so
  /// only this path shows the recovery's own mark (`markRecovered`; FORMAL-10).
  run recoveryAloneReloads =
    initWith(true)
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RegistrationFailed) and deb == CLEAN)
      .then(modify(Sub))
      .expect(allInvariants and ghost.excused == Set(Sub))
      .then(freeWatch)
      .then(saveAtomically(Css))
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        health == HWatching,
        last.events == [EvRecovered],
        last.needsReconcile,
        not(reconciledFully),
        deb.paths == Set(Css),
        deb.full,
        not(deb.rescan),
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and revision == 1 and last.firedFull and settled and ghost.pending == Set())

  /// Runtime failures degrade once and report once; a further failure is absorbed; the next
  /// complete reconcile recovers, once.
  run runtimeFailureDegrades =
    initWith(false)
      .then(runtimeFailure)
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RuntimeFailure) and last.events == [EvDegraded(RuntimeFailure)])
      .then(runtimeFailure)
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RuntimeFailure) and last.events == [])
      .then(createFile(Css2))
      .then(receive(4, false))
      .expect(allInvariants and health == HWatching and last.events == [EvRecovered])
      .expect(ghost.degradedEvents == 1 and ghost.recoveredEvents == 1)

  /// Start-up with `d` unwatchable: `Starting`, then the queued registration failure
  /// degrades; an overflow's full reconcile retries `d` once a watch is free and recovers.
  run startupRegistrationFailure =
    initWith(true)
      .expect(allInvariants and health == HStarting and src.outstanding == Set(DirD))
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RegistrationFailed))
      .then(freeWatch)
      .then(kernelQueue(true))
      .then(modify(Index))
      .then(receive(1, false))
      .expect(allInvariants and health == HWatching and src.outstanding == Set() and deb.rescan)

  /// The start-up failure is delivered before a loss reported after it: degraded, then the
  /// overflow's full reconcile watches `d` and recovers, once each. Delivered after it, the
  /// stale failure would degrade a watcher with every directory watched (FORMAL-10).
  run startupFailureBeforeOverflow =
    initWith(true)
      .then(freeWatch)
      .then(kernelQueue(true))
      .then(modify(Index))
      .then(kernelQueue(false))
      .expect(allInvariants and src.overflow and registrationFailureQueued(src))
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RegistrationFailed) and src.overflow)
      .then(receive(1, false))
      .expect(and {
        allInvariants,
        health == HWatching,
        last.events == [EvRecovered],
        src.outstanding == Set(),
        deb.rescan,
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        health == HWatching,
        settled,
        ghost.pending == Set(),
        ghost.degradedEvents == 1,
        ghost.recoveredEvents == 1,
      })

  /// Start-up whose registration of `d` passed the relisting cap: `Starting`, no failure, the
  /// loss first; its full reconcile watches `d` and only then is live reload claimed, with no
  /// health event (FORMAL-10).
  run startupCapDefersReadiness =
    initStartup(false, true)
      .expect(allInvariants and health == HStarting and src.overflow and src.watched == Set(Root))
      .then(modify(Css))
      .then(receive(1, false))
      .expect(and {
        allInvariants,
        health == HWatching,
        last.events == [],
        src.watched == Set(Root, DirD),
        deb.rescan,
        deb.full,
      })
      .then(receive(1, false))
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and revision == 1 and last.firedFull and settled and ghost.pending == Set())

  /// The root is deleted: every removal and the root's own (through its parent's watch, ADR
  /// 0073), more than the channel holds, so a loss too. The loss's full reconcile finds the
  /// root gone and degrades with `RootRemoved`, once; one full revision covers every removal.
  /// The root recreated at the same path is not the served one: its parent's watch reports
  /// it, and the reconcile that asks for still finds the root removed, so the watcher stays
  /// degraded and live reload is not claimed again (ADR 0067).
  run rootDeleteRecreate =
    initWith(false)
      .then(rootEmpty)
      .then(rootDelete)
      .expect(allInvariants and src.overflow and src.queue.length() == CAP and src.watched == Set())
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        health == HDegraded(RootRemoved),
        last.events == [EvDegraded(RootRemoved)],
        deb.rescan,
      })
      .then(receive(4, false))
      .expect(allInvariants and health == HDegraded(RootRemoved) and last.events == [])
      .then(rootRecreate)
      .expect(allInvariants and src.queue.length() == 1)
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RootRemoved) and last.events == [] and last.needsReconcile)
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        revision == 1,
        last.firedFull,
        settled,
        ghost.pending == Set(),
        health == HDegraded(RootRemoved),
        ghost.degradedEvents == 1,
      })

  /// A root removed while a directory could not be watched: the registration failure was
  /// reported, and the root's removal escalates once to `RootRemoved`, so the warning says why
  /// live reload cannot come back (ADR 0067).
  run rootRemovalEscalates =
    initWith(true)
      .then(receive(1, false))
      .expect(allInvariants and health == HDegraded(RegistrationFailed))
      .then(rootEmpty)
      .then(rootDelete)
      .expect(allInvariants and src.queue.length() == 4 and not(src.overflow))
      .then(receive(4, false))
      .expect(and {
        allInvariants,
        health == HDegraded(RootRemoved),
        last.events == [EvDegraded(RootRemoved)],
        ghost.escalations == 1,
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(allInvariants and settled and health == HDegraded(RootRemoved) and ghost.pending == Set())

  /// `rm -r` of the root with the pipeline between its two parts: the removals below the
  /// root are received and reconciled while the emptied root still stands, so that reconcile
  /// finds no root removal; then the root itself goes. Its own watch says nothing (the
  /// server's descriptor keeps its inode: no `IN_DELETE_SELF`), so only the watch of its
  /// parent can report it, and that report's reconcile degrades with `RootRemoved`
  /// (FORMAL-10).
  run rootRemovedAfterEmptyReconcile =
    initWith(false)
      .then(rootEmpty)
      .expect(allInvariants and src.queue.length() == 4 and not(src.overflow))
      .then(receive(4, false))
      .expect(allInvariants and fs.rootPresent and health == HWatching and src.queue == [])
      .then(rootDelete)
      .expect(allInvariants and src.queue.length() == 1)
      .then(receive(1, false))
      .expect(and {
        allInvariants,
        health == HDegraded(RootRemoved),
        last.events == [EvDegraded(RootRemoved)],
      })
      .then(tick)
      .then(tick)
      .then(settle(false))
      .expect(and {
        allInvariants,
        settled,
        revision == 1,
        ghost.pending == Set(),
        health == HDegraded(RootRemoved),
        ghost.degradedEvents == 1,
      })

  /// Shutdown drops a pending change with the task: nothing fires afterwards.
  run shutdownDropsPending =
    initWith(false)
      .then(modify(Css))
      .then(receive(1, false))
      .then(shutdown)
      .then(tick)
      .then(tick)
      .expect(allInvariants and cancelled and revision == 0 and src.queue == [])
}
