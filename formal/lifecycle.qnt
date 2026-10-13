// -*- mode: Bluespec; -*-
/**
 * The server lifecycle (foundation.md section 6.1 item 1, plan.md section 9), retrofitted onto
 * the code that exists (ADR 0014):
 *
 * - the pure machine `LifecycleState::step` in `crates/preview-domain/src/lifecycle.rs`
 *   (DOM-09): `transition` below is that table, row for row, with the same nine states, seven
 *   inputs and nine effects in the same order;
 * - the server handle `Server` in `crates/preview-composition/src/lifecycle.rs` (ADR 0051),
 *   which feeds the machine and performs its effects.
 *
 * Two step relations share the table and the resource accounting:
 *
 * - `step` is the machine's contract: every input the table accepts, in any interleaving,
 *   including a stop that arrives while a start-up step is in flight (the `Stopping(Await*)`
 *   states, "cancellation" in foundation.md) and `InStopped` as its own step. It is what the
 *   domain machine promises any driver.
 * - `serverStep` is the composition's `Server` as it runs today: start-up is synchronous on
 *   the caller's thread, so `startServer` feeds `InStart` and then each step's outcome until
 *   the machine stops asking (`Server::start_with`'s loop), and `stopServer` feeds `InStop`
 *   then `InStopped` once nothing is awaited (`Server::stop`). A stop can therefore never
 *   meet a start-up step in flight, and the `Stopping(Await*)` states are unreachable here
 *   (ADR 0051, Consequences). A refusal before `Start` (bind scope, root, runtime) is
 *   `refuseStart`: the machine stays `Stopped` and nothing is held. FORMAL-05 replays
 *   `serverStep` traces against the real `Server`, observing `Server::state()`; its
 *   generated traces come from `serverStepMostlyBound`, the same transitions with fewer
 *   bind failures sampled, and its named ones from `lifecycle_traces.qnt`.
 *
 * Both relations also let a client send a request or open an event stream, answered only by
 * an open listener with a running serve task. Every invariant must hold under both.
 *
 * Deliberately outside the model: the drain itself. It happens inside the `JoinTasks` effect
 * (the serve task's drain, ADR 0045), so a second signal during the drain, which ends it at
 * once (`Stopped::Forced`, exit 130 or 143), never reaches the machine: the machine sees one
 * `InStop` per stop path and treats any later one as a no-op (`stopIdempotent`). That forced
 * exit is pinned by composition tests (`should_stop_at_once_when_second_stop_arrives_during_drain`).
 * A restart is a fresh `InStart` from `Stopped`; the real composition does it with a new
 * `Server` (a new process for the binary), since a stopped handle has given up its runtime.
 * `Failed` is terminal (FORMAL-10 review): `Server::start` returns the bind error and no
 * handle, and the binary exits 1, so nothing restarts a failed server; a caller that calls
 * `Server::start` again gets a new machine from `Stopped` with nothing held
 * (`failedStartupHoldsNothing`), whose states are exactly those reachable from `init`. The
 * binary's signal path (SIGINT/SIGTERM to a stop, the exit codes) is not modelled either:
 * `stopServer` is `Server::stop`.
 *
 * Where foundation.md's wording and the code differ, the model follows the code and ADR 0051:
 * a watcher failure during start-up is degraded serving (`Serving(false)`), not `Failed`,
 * which only a failed bind reaches; a watcher that fails while serving is not a lifecycle
 * input at all (the watcher pipeline reports it; `watcher.qnt`, FORMAL-02); "cancellation" is
 * a stop during start-up; and the root and runtime checks are preconditions before `Start`.
 */
module lifecycle {
  // ---------------------------------------------------------------------------------------
  // Types: mirror preview_domain::lifecycle.

  /// `StartPhase`: binding the listener, or starting the watcher with the listener bound.
  type StartPhase = Binding | StartingWatcher

  /// `Stopping { awaiting }`: `NoStep` is `None`; the others are the step still in flight.
  type Awaiting = NoStep | AwaitBind | AwaitWatcher

  /// `LifecycleState`. `Serving(true)` owns a watcher; `Serving(false)` is degraded.
  type LifecycleState =
    | Stopped
    | Starting(StartPhase)
    | Serving(bool)
    | Stopping(Awaiting)
    | Failed

  /// `LifecycleInput`, each prefixed `In` (the state `Stopped` and the input `Stopped` would
  /// otherwise share a constructor name).
  type Input =
    | InStart
    | InBound
    | InBindFailed
    | InWatcherReady
    | InWatcherFailed
    | InStop
    | InStopped

  /// `LifecycleEffect`, in declaration order.
  type Effect =
    | BindListener
    | StartWatcher
    | AnnounceReady
    | Degraded
    | AbortStartup
    | CloseListener
    | CloseStreams
    | StopWatcher
    | JoinTasks

  /// The result of `LifecycleState::step`: `accepted` false is a `LifecycleRefusal`, and then
  /// `next` is the unchanged state and `effects` is empty.
  type Transition = { accepted: bool, next: LifecycleState, effects: List[Effect] }

  /// What the server owns. Each count is changed only by the input that acquires it (a step's
  /// outcome) or the effect that releases it, never derived from the state, so the invariants
  /// below check that the table's effects really release what its inputs acquire.
  type Resources = {
    // Bound TCP listeners: acquired by `InBound` (also a late one), released by `CloseListener`.
    listeners: int,
    // Running watchers: acquired by `InWatcherReady`, released by `StopWatcher`.
    watchers: int,
    // The serve task: spawned by `AnnounceReady`, joined by `JoinTasks`.
    serveTasks: int,
    // The watcher pipeline (debounce) task: spawned with the watcher, joined by `JoinTasks`.
    debounceTasks: int,
    // Open SSE event streams: opened by a client while serving, ended by `CloseStreams`.
    streams: int,
  }

  /// What a client saw for the last action: no request, an answer, or none (refused or never
  /// accepted).
  type Response = NoRequest | Answered | Unanswered

  /// Before-and-after bookkeeping: the state and resources the last action started from.
  type Snapshot = { state: LifecycleState, res: Resources }

  // ---------------------------------------------------------------------------------------
  // Constants.

  pure val NOTHING: Resources =
    { listeners: 0, watchers: 0, serveTasks: 0, debounceTasks: 0, streams: 0 }

  /// `DEFAULT_MAX_STREAMS` in preview-adapter-http (ADR 0044); a further stream is refused.
  pure val MAX_STREAMS = 64

  /// `LifecycleState::ALL`.
  pure val ALL_STATES: Set[LifecycleState] = Set(
    Stopped,
    Starting(Binding),
    Starting(StartingWatcher),
    Serving(true),
    Serving(false),
    Stopping(NoStep),
    Stopping(AwaitBind),
    Stopping(AwaitWatcher),
    Failed,
  )

  /// `LifecycleInput::ALL`.
  pure val ALL_INPUTS: Set[Input] =
    Set(InStart, InBound, InBindFailed, InWatcherReady, InWatcherFailed, InStop, InStopped)

  /// plan.md section 9's shutdown order: listener, SSE streams, watcher, tasks.
  pure def shutdownRank(e: Effect): int = match e {
    | CloseListener => 1
    | CloseStreams => 2
    | StopWatcher => 3
    | JoinTasks => 4
    | _ => 0
  }

  // ---------------------------------------------------------------------------------------
  // The machine: `LifecycleState::step` and its predicates.

  pure def accept(target: LifecycleState, effects: List[Effect]): Transition =
    { accepted: true, next: target, effects: effects }

  pure def refuse(s: LifecycleState): Transition =
    { accepted: false, next: s, effects: [] }

  /// `LifecycleState::step`, row for row (the 19 declared transitions of DOM-09's
  /// `DECLARED` table); everything else is refused and leaves the state unchanged.
  pure def transition(s: LifecycleState, i: Input): Transition = match s {
    | Stopped => match i {
      | InStart => accept(Starting(Binding), [BindListener])
      | InStop => accept(Stopped, [])
      | _ => refuse(s)
    }
    | Starting(phase) => match phase {
      | Binding => match i {
        | InBound => accept(Starting(StartingWatcher), [StartWatcher])
        | InBindFailed => accept(Failed, [AbortStartup])
        | InStop => accept(Stopping(AwaitBind), [])
        | _ => refuse(s)
      }
      | StartingWatcher => match i {
        | InWatcherReady => accept(Serving(true), [AnnounceReady])
        | InWatcherFailed => accept(Serving(false), [Degraded, AnnounceReady])
        | InStop => accept(Stopping(AwaitWatcher), [CloseListener])
        | _ => refuse(s)
      }
    }
    | Serving(watched) => match i {
      | InStop =>
        if (watched) accept(Stopping(NoStep), [CloseListener, CloseStreams, StopWatcher, JoinTasks])
        else accept(Stopping(NoStep), [CloseListener, CloseStreams, JoinTasks])
      | _ => refuse(s)
    }
    | Stopping(awaiting) => match awaiting {
      | NoStep => match i {
        | InStopped => accept(Stopped, [])
        | InStop => accept(s, [])
        | _ => refuse(s)
      }
      | AwaitBind => match i {
        | InBound => accept(Stopping(NoStep), [CloseListener])
        | InBindFailed => accept(Stopping(NoStep), [])
        | InStop => accept(s, [])
        | _ => refuse(s)
      }
      | AwaitWatcher => match i {
        | InWatcherReady => accept(Stopping(NoStep), [StopWatcher, JoinTasks])
        | InWatcherFailed => accept(Stopping(NoStep), [])
        | InStop => accept(s, [])
        | _ => refuse(s)
      }
    }
    | Failed => match i {
      | InStop => accept(Failed, [])
      | _ => refuse(s)
    }
  }

  /// `LifecycleState::accepts_requests`: only a serving server answers.
  pure def isServing(s: LifecycleState): bool = match s {
    | Serving(_) => true
    | _ => false
  }

  /// `LifecycleState::holds_listener`.
  pure def holdsListener(s: LifecycleState): bool = match s {
    | Starting(phase) => phase == StartingWatcher
    | Serving(_) => true
    | _ => false
  }

  /// `LifecycleState::holds_watcher`.
  pure def holdsWatcher(s: LifecycleState): bool = s == Serving(true)

  /// `LifecycleState::holds_tasks`.
  pure def holdsTasks(s: LifecycleState): bool = isServing(s)

  /// The states in which a stop has already been asked for, or the server never ran.
  pure def isStoppedOrStopping(s: LifecycleState): bool = match s {
    | Stopped => true
    | Stopping(_) => true
    | Failed => true
    | _ => false
  }

  // ---------------------------------------------------------------------------------------
  // Resources: what the input acquires, then what each effect does, in order.

  pure def acquire(r: Resources, i: Input): Resources = match i {
    | InBound => { ...r, listeners: r.listeners + 1 }
    | InWatcherReady => { ...r, watchers: r.watchers + 1, debounceTasks: r.debounceTasks + 1 }
    | _ => r
  }

  pure def perform(r: Resources, e: Effect): Resources = match e {
    | AnnounceReady => { ...r, serveTasks: r.serveTasks + 1 }
    | CloseListener => { ...r, listeners: r.listeners - 1 }
    | CloseStreams => { ...r, streams: 0 }
    | StopWatcher => { ...r, watchers: r.watchers - 1 }
    | JoinTasks => { ...r, serveTasks: 0, debounceTasks: 0 }
    | _ => r
  }

  pure def account(r: Resources, i: Input, effects: List[Effect]): Resources =
    effects.foldl(acquire(r, i), (acc, e) => perform(acc, e))

  /// A request is answered only by a bound listener whose accept loop (the serve task) runs.
  pure def answers(r: Resources): bool = r.listeners > 0 and r.serveTasks > 0

  // ---------------------------------------------------------------------------------------
  // A pure run of the machine, for the composite `serverStep` actions.

  /// The machine plus what a sequence of inputs fed it: every input, every effect in order,
  /// and the effects of the latest transition (which say what to do next). `ok` turns false
  /// at the first refused input; the composition treats that as a programming error.
  type Run = {
    state: LifecycleState,
    res: Resources,
    inputs: List[Input],
    effects: List[Effect],
    latest: List[Effect],
    ok: bool,
  }

  pure def feedRun(m: Run, i: Input): Run = {
    val t = transition(m.state, i)
    if (not(m.ok) or not(t.accepted)) { ...m, ok: false }
    else {
      state: t.next,
      res: account(m.res, i, t.effects),
      inputs: m.inputs.append(i),
      effects: m.effects.concat(t.effects),
      latest: t.effects,
      ok: true,
    }
  }

  /// The outcome `Startup::perform` returns for the latest effects: `BindListener` binds
  /// (`InBound` or `InBindFailed`), `StartWatcher` starts the watcher (`InWatcherReady` or
  /// `InWatcherFailed`); the last effect with an outcome wins, as in the real loop.
  pure def outcome(effects: List[Effect], bindOk: bool, watcherOk: bool): List[Input] =
    effects.foldl([], (acc, e) => match e {
      | BindListener => [if (bindOk) InBound else InBindFailed]
      | StartWatcher => [if (watcherOk) InWatcherReady else InWatcherFailed]
      | _ => acc
    })

  /// One turn of `Server::start_with`'s loop: feed the outcome of the latest effects, if any.
  pure def startTurn(m: Run, bindOk: bool, watcherOk: bool): Run = {
    val pending = outcome(m.latest, bindOk, watcherOk)
    if (pending == []) m else feedRun(m, pending[0])
  }

  pure def runFrom(s: LifecycleState, r: Resources): Run =
    { state: s, res: r, inputs: [], effects: [], latest: [], ok: true }

  /// `Server::start_with` from `s`: `InStart`, then the outcomes until none is pending. Three
  /// turns cover the longest start-up (bind, then watcher, then nothing). The turns are
  /// bound one by one, not nested as arguments: Apalache inlines a pure definition by
  /// substituting its argument at every use, so three nested turns grow exponentially and
  /// run it out of memory in `quint verify` (`cargo xtask formal-verify`, FORMAL-10).
  pure def startRun(s: LifecycleState, r: Resources, bindOk: bool, watcherOk: bool): Run = {
    val m0 = feedRun(runFrom(s, r), InStart)
    val m1 = startTurn(m0, bindOk, watcherOk)
    val m2 = startTurn(m1, bindOk, watcherOk)
    startTurn(m2, bindOk, watcherOk)
  }

  /// `Server::stop`: `InStop` and its effects, then `InStopped` once nothing is awaited.
  pure def stopRun(s: LifecycleState, r: Resources): Run = {
    val m0 = feedRun(runFrom(s, r), InStop)
    if (m0.state == Stopping(NoStep)) feedRun(m0, InStopped) else m0
  }

  // ---------------------------------------------------------------------------------------
  // State.

  /// `Server::state()`.
  var state: LifecycleState
  /// What the server owns now.
  var res: Resources
  /// The inputs the last action fed the machine, in order (empty for a client action).
  var inputs: List[Input]
  /// The effects the last action performed, in order.
  var effects: List[Effect]
  /// The state and resources the last action started from.
  var before: Snapshot
  /// What a client saw in the last action.
  var response: Response

  action init = all {
    state' = Stopped,
    res' = NOTHING,
    inputs' = [],
    effects' = [],
    before' = { state: Stopped, res: NOTHING },
    response' = NoRequest,
  }

  /// Commits a run of the machine.
  action commit(m: Run): bool = all {
    m.ok,
    state' = m.state,
    res' = m.res,
    inputs' = m.inputs,
    effects' = m.effects,
    before' = { state: state, res: res },
    response' = NoRequest,
  }

  /// A client action: the machine is untouched.
  action client(r: Resources, seen: Response): bool = all {
    state' = state,
    res' = r,
    inputs' = [],
    effects' = [],
    before' = { state: state, res: res },
    response' = seen,
  }

  // ---------------------------------------------------------------------------------------
  // Actions of the machine's contract (`step`): one input each, enabled when accepted.

  action feed(i: Input): bool = commit(feedRun(runFrom(state, res), i))

  /// Begin start-up; from `Stopped` after a clean stop this is a restart.
  action start = feed(InStart)
  /// The bind succeeded (possibly late, after a stop).
  action bound = feed(InBound)
  /// The bind failed.
  action bindFailed = feed(InBindFailed)
  /// The watcher is running (possibly late, after a stop).
  action watcherReady = feed(InWatcherReady)
  /// The watcher could not start: degraded serving, or nothing when stopping.
  action watcherFailed = feed(InWatcherFailed)
  /// A stop (a signal or a caller's `stop()`), in any state, including during start-up.
  action stop = feed(InStop)
  /// The shutdown effects have all completed.
  action stopped = feed(InStopped)

  // ---------------------------------------------------------------------------------------
  // Actions of the composition's `Server` (`serverStep`).

  /// `Server::start` with the bind and the watcher succeeding or not.
  action startServerWith(bindOk: bool, watcherOk: bool): bool = all {
    state == Stopped,
    commit(startRun(state, res, bindOk, watcherOk)),
  }

  action startServer = {
    nondet bindOk = oneOf(Set(true, false))
    nondet watcherOk = oneOf(Set(true, false))
    startServerWith(bindOk, watcherOk)
  }

  /// `startServer` with a bind that succeeds: the same transitions, sampled more often by
  /// `serverStepMostlyBound` (FORMAL-05).
  action startServerBound = {
    nondet bindOk = Set(true).oneOf()
    nondet watcherOk = oneOf(Set(true, false))
    startServerWith(bindOk, watcherOk)
  }

  /// `Server::start` refused before `Start`: a remote bind without `--allow-remote`, a missing
  /// or reserved root, or no runtime. Nothing was started, nothing is held.
  action refuseStart = all {
    state == Stopped,
    client(res, NoRequest),
  }

  /// `Server::stop` (also what the binary does at the first signal). Idempotent.
  action stopServer = commit(stopRun(state, res))

  // ---------------------------------------------------------------------------------------
  // Client actions, in both relations.

  /// A plain request: answered only by a listener with a running accept loop.
  action request = client(res, if (answers(res)) Answered else Unanswered)

  /// An SSE connection: it stays open until `CloseStreams`; the 65th is refused (503).
  action openStream = {
    val admitted = answers(res) and res.streams < MAX_STREAMS
    client(
      if (admitted) { ...res, streams: res.streams + 1 } else res,
      if (admitted) Answered else Unanswered,
    )
  }

  // ---------------------------------------------------------------------------------------
  // Step relations.

  /// The domain machine's contract: any accepted input in any order.
  action step = any {
    start,
    bound,
    bindFailed,
    watcherReady,
    watcherFailed,
    stop,
    stopped,
    request,
    openStream,
  }

  /// The composition's `Server`: synchronous start-up, blocking stop.
  action serverStep = any {
    startServer,
    refuseStart,
    stopServer,
    request,
    openStream,
  }

  /// `serverStep` sampled for conformance traces (FORMAL-05, ADR 0056): the same transitions
  /// (`startServerBound` is `startServer` with `bindOk` true), but the simulator picks an
  /// enabled branch of `any` uniformly, so listing the bound start three more times makes a
  /// start fail its bind one time in eight rather than one in two. `Failed` is terminal, and
  /// under `serverStep` about 60% of the steps of 25-step traces are spent there; here about
  /// a third are, and restarts and degraded serving are reached more often.
  action serverStepMostlyBound = any {
    startServer,
    startServerBound,
    startServerBound,
    startServerBound,
    refuseStart,
    stopServer,
    request,
    openStream,
  }

  // ---------------------------------------------------------------------------------------
  // Invariants (foundation.md section 6.1 item 1). Each is checked under `step`,
  // `serverStep` and `serverStepMostlyBound` by `quint run --invariant`.

  /// No responses in a stopped state: a request is answered, or an event stream is open, only
  /// while serving.
  val noResponsesWhenStopped =
    (response == Answered or res.streams > 0) implies isServing(state)

  /// Exactly one active owner of a bound listener: never two, never a negative count, and one
  /// exactly in the states `holds_listener` names.
  val singleListenerOwner = and {
    res.listeners >= 0,
    res.listeners <= 1,
    (res.listeners == 1) == holdsListener(state),
  }

  /// All owned tasks (and the watcher and streams they serve) are released after a stop:
  /// tasks and streams exist only while serving, a watcher only in `holds_watcher` states,
  /// and a stopped server holds nothing at all.
  val tasksReleasedAfterStop = and {
    res.serveTasks >= 0,
    res.debounceTasks >= 0,
    res.watchers >= 0,
    res.serveTasks <= 1,
    res.debounceTasks <= 1,
    (res.serveTasks + res.debounceTasks > 0) == holdsTasks(state),
    res.streams > 0 implies holdsTasks(state),
    (res.watchers == 1) == holdsWatcher(state),
    state == Stopped implies res == NOTHING,
  }

  /// Shutdown is idempotent: `InStop` is accepted in every state, and once a stop was asked
  /// for (or the server never ran) another one changes nothing and performs nothing.
  val stopIdempotent = and {
    ALL_STATES.forall(s => transition(s, InStop).accepted),
    (inputs == [InStop] and isStoppedOrStopping(before.state)) implies and {
      state == before.state,
      res == before.res,
      effects == [],
    },
  }

  /// A failed start-up leaves nothing allocated, and `Failed` is never left.
  val failedStartupHoldsNothing = and {
    state == Failed implies res == NOTHING,
    before.state == Failed implies state == Failed,
  }

  /// The shutdown effects of one action run in plan.md section 9's order: listener, SSE
  /// streams, watcher, tasks (so the streams end before the drain wait in `JoinTasks`).
  val shutdownInPlanOrder = {
    val ranks = effects.select(e => shutdownRank(e) > 0).foldl([], (acc, e) => acc.append(shutdownRank(e)))
    ranks.indices().forall(k => k == 0 or ranks[k - 1] < ranks[k])
  }

  /// Every invariant at once.
  val allInvariants = and {
    noResponsesWhenStopped,
    singleListenerOwner,
    tasksReleasedAfterStop,
    stopIdempotent,
    failedStartupHoldsNothing,
    shutdownInPlanOrder,
  }

  // ---------------------------------------------------------------------------------------
  // Witnesses: states a run should reach, so a simulation that never gets there is visible
  // (`quint run --witnesses ...` counts the traces that do).

  val reachesServingWatched = state == Serving(true)
  val reachesServingDegraded = state == Serving(false)
  val reachesStopAwaitingBind = state == Stopping(AwaitBind)
  val reachesStopAwaitingWatcher = state == Stopping(AwaitWatcher)
  val reachesFailed = state == Failed
  val reachesOpenStream = res.streams > 0
}
