// -*- mode: Bluespec; -*-
/**
 * The live-reload protocol (foundation.md section 6.1 item 3, plan.md sections 7.2 and 7.3 as
 * amended by ADR 0030 and ADR 0031), retrofitted onto the code that exists (ADR 0014):
 *
 * - the server's revision and instance: `ReloadState` in
 *   `crates/preview-application/src/reload_state.rs`, one `Published { state, changes }`
 *   value in a latest-value channel (`publish_next` advances the revision by one; a new
 *   process is a new `InstanceId` at revision 0 with a full, empty change set);
 * - one event stream per connection: `events` and `Feed` in
 *   `crates/preview-adapter-http/src/sse.rs` (a `state` baseline first, then one `reload`
 *   per value the subscription yields, coalesced to the newest; `ReloadPayload::after` sends
 *   `full` with no changes across a gap or another instance);
 * - the browser: `decide`, `strategy`, `hot`, `runAccepted` and `scriptReload` in
 *   `crates/preview-adapter-http/assets/client.js` (render-time baseline, reload once per
 *   difference, hot apply only for exactly the next revision of the same instance).
 *
 * Abstractions, each with the reason it loses nothing the invariants talk about:
 *
 * - A change set is its set of kinds and its `full` flag (`ChangeSet::is_full`). Paths, the
 *   64-path cap and the full reasons (rescan, directory, removal, undisclosed, cap) are the
 *   domain's (`change_set.rs`, DOM-10, and `watcher.qnt`); here a reason is one boolean, and
 *   an `html` or `other` kind always makes the set full, as `ChangeSet::add` does. The
 *   pipeline never publishes an empty, hot set (`Debouncer::poll` fires a full or non-empty
 *   set), so neither does `publish`.
 * - Instances are numbered 1, 2, ... in start order. Real instance ids are unordered strings;
 *   the client only tests them for equality, and the numbering only lets the invariants say
 *   "newer" (an instance id never repeats, `server::instance_id`).
 * - A connection is reliable and in order while it lasts (one TCP stream). A drop, a server
 *   stop, a navigation and a refused connect (503 at the stream limit, ADR 0044) all lose
 *   whatever was in flight: nothing assumes delivery across a disconnected interval, and a
 *   reconnect starts from a fresh `state` baseline. A refused connect is the same as not
 *   connecting yet, so it is not a separate action.
 * - The browser applies a hot change set atomically (`receive` with `applied`). In client.js a
 *   hot apply is asynchronous and events that arrive meanwhile wait in `backlog` until the
 *   baseline is known, which is the same as processing them in order afterwards. `applied`
 *   is the nondeterministic outcome of the in-place work: for css and assets, whether every
 *   path found a stylesheet or reference and every load succeeded (`apply`); for scripts,
 *   whether every changed script had an accepted handler that succeeded in time
 *   (`runAccepted`). A failure falls back to a reload (for scripts with the scroll position
 *   kept, `scriptReload`), counted as a reload.
 * - Every page is injected with a baseline (the HTML handler reads `ReloadState::current`
 *   before opening the file, plan.md section 7.3). client.js's "baseline missing" branch for
 *   old injected pages is left out.
 * - A navigation (the client's reload, or the user's) needs a running server to finish; a
 *   reload that hits a stopped server shows the browser's error page, which has no client,
 *   and is outside the protocol.
 *
 * Ghost state, never on the wire: `history` (every committed state and its change set),
 * each browser's `trigger` (the state it last reacted to), `taken` (the last action's name)
 * and `before` (what the last action started from). The predecessor of an event on its
 * connection is the event before it in `stream`.
 *
 * Conformance (D16, ADR 0017; FORMAL-07): the server side, `up`, `committed`, `latest` and
 * each connection's `open`, `lastSent` and `stream`, is what the driver observes from the real
 * `ReloadState` and SSE bodies. The browser side (`doc`, `baseline`, the counters) is the
 * client's, verified by `client.test.js` and the Playwright specs (`reconnect.spec.ts`,
 * `hot.spec.ts`), never by a Rust reimplementation.
 */
module reload {
  // ---------------------------------------------------------------------------------------
  // Types.

  /// `AssetKind` (ADR 0031): what the browser can do with a changed path.
  type Kind = Css | Js | Html | Asset | Other

  /// `SiteState`: an instance and a revision. Instance 0 is "none" (no state yet).
  type Stamp = { instance: int, revision: int }

  /// `Published::changes`, abstracted: the kinds of its paths and `ChangeSet::is_full`.
  type Change = { kinds: Set[Kind], full: bool }

  /// The SSE event names (plan.md section 7.2). Prefixed so they do not clash with the
  /// browser's decisions.
  type EventName = StateEvent | ReloadEvent

  /// One SSE event. A `state` event carries no change set: `kinds` is empty and `full` false.
  type Event = { name: EventName, at: Stamp, kinds: Set[Kind], full: bool }

  /// The browser tab: no page yet, a navigation in flight (`reloading` in client.js; the
  /// event stream is gone with the old document), or a page with a running client.
  type Doc = NoPage | Navigating | Shown

  /// client.js `decide`: 'none', 'hot' or 'reload'.
  type Decision = Ignore | TryHot | MustReload

  type Browser = {
    doc: Doc,
    // The render-time baseline, advanced only by a successful hot apply.
    baseline: Stamp,
    // Whether an event stream is open (the server's `Feed` exists).
    open: bool,
    // `Feed::last_sent`: the state the last event on this connection announced.
    lastSent: Stamp,
    // Every event sent on the current connection, in order.
    stream: List[Event],
    // How many of them the client has handled; the rest are in flight.
    delivered: int,
    // Hot applies that succeeded (no navigation).
    hotApplied: int,
    // Reloads the client started (a full reload or a script reload).
    reloaded: int,
    // Ghost: the state the client last reacted to (hot apply or reload).
    trigger: Stamp,
  }

  // ---------------------------------------------------------------------------------------
  // Constants.

  pure val BROWSERS: Set[str] = Set("b1", "b2")

  pure val KINDS: Set[Kind] = Set(Css, Js, Html, Asset, Other)

  /// `AssetKind::is_hot`: the kinds a hot change set may hold.
  pure val HOT_KINDS: Set[Kind] = Set(Css, Js, Asset)

  pure val NONE: Stamp = { instance: 0, revision: 0 }

  /// `ReloadState::new`: revision 0, no paths, full (`FullReason::Rescan`).
  pure val INITIAL_CHANGE: Change = { kinds: Set(), full: true }

  pure val NO_BROWSER: Browser = {
    doc: NoPage,
    baseline: NONE,
    open: false,
    lastSent: NONE,
    stream: [],
    delivered: 0,
    hotApplied: 0,
    reloaded: 0,
    trigger: NONE,
  }

  // ---------------------------------------------------------------------------------------
  // Pure functions.

  /// Whether `a` was committed after `b`: a later instance, or a later revision of the same.
  pure def newer(a: Stamp, b: Stamp): bool =
    a.instance > b.instance or (a.instance == b.instance and a.revision > b.revision)

  /// The change set a debounced burst produces: `full` when a reason forced it or a path is
  /// `html` or `other` (`ChangeSet::add`, `FullReason::DocumentChanged`).
  pure def changeSet(kinds: Set[Kind], reason: bool): Change =
    { kinds: kinds, full: reason or not(kinds.subseteq(HOT_KINDS)) }

  /// The baseline event a new connection sends first (`state_event`).
  pure def stateEvent(s: Stamp): Event = { name: StateEvent, at: s, kinds: Set(), full: false }

  /// `ReloadPayload::after(last_sent, published)`: across a gap or another instance the
  /// changes are gone, so `full` is true and the change list is empty; otherwise the change
  /// set as published, `full` as the set says.
  pure def reloadEvent(lastSent: Stamp, s: Stamp, c: Change): Event = {
    val contiguous = lastSent.instance == s.instance and s.revision == lastSent.revision + 1
    {
      name: ReloadEvent,
      at: s,
      kinds: if (contiguous) c.kinds else Set(),
      full: not(contiguous) or c.full,
    }
  }

  /// client.js `decide(baseline, kind, state, reloading)` for a page with a baseline and no
  /// reload under way (a navigating page has no stream, so it receives nothing).
  pure def decide(baseline: Stamp, e: Event): Decision =
    if (e.name == ReloadEvent and e.full) MustReload
    else if (e.at == baseline) Ignore
    else if (and {
      e.name == ReloadEvent,
      e.kinds != Set(),
      e.at.instance == baseline.instance,
      e.at.revision == baseline.revision + 1,
    }) TryHot
    else MustReload

  /// client.js `strategy`: a hot change set can be applied in place only when every kind is
  /// css, js or asset; anything else ('full') falls back to a reload.
  pure def applicable(kinds: Set[Kind]): bool = kinds != Set() and kinds.subseteq(HOT_KINDS)

  /// The connection is gone: the server's `Feed` (and its permit) is dropped with the body,
  /// and whatever was in flight is lost.
  pure def closed(br: Browser): Browser =
    { ...br, open: false, lastSent: NONE, stream: [], delivered: 0 }

  /// The events in flight to the client.
  pure def inFlight(br: Browser): List[Event] = br.stream.slice(br.delivered, br.stream.length())

  /// Hot applies and reloads together: every reaction of the client to a newer state.
  pure def reactions(br: Browser): int = br.hotApplied + br.reloaded

  // ---------------------------------------------------------------------------------------
  // State.

  /// Whether a server process runs (and its listener serves).
  var up: bool
  /// The newest committed state: `ReloadState::current()`; kept while stopped, so a
  /// restart's instance is the next one.
  var committed: Stamp
  /// The change set published with `committed` (`Published::changes`).
  var latest: Change
  /// Ghost: every committed state and its change set.
  var history: Stamp -> Change
  var browsers: str -> Browser
  /// Ghost: the name of the last action (for scenarios and the FORMAL-07 action map).
  var taken: str
  /// Ghost: what the last action started from, for the invariants about one step.
  var before: { committed: Stamp, browsers: str -> Browser }

  action init = all {
    up' = true,
    committed' = { instance: 1, revision: 0 },
    latest' = INITIAL_CHANGE,
    history' = Map({ instance: 1, revision: 0 } -> INITIAL_CHANGE),
    browsers' = BROWSERS.mapBy(_ => NO_BROWSER),
    taken' = "init",
    before' = { committed: NONE, browsers: BROWSERS.mapBy(_ => NO_BROWSER) },
  }

  /// Bookkeeping every action does.
  action record(name: str): bool = all {
    taken' = name,
    before' = { committed: committed, browsers: browsers },
  }

  /// An action that touches one browser and nothing on the server.
  action browserOnly(name: str, b: str, br: Browser): bool = all {
    up' = up,
    committed' = committed,
    latest' = latest,
    history' = history,
    browsers' = browsers.set(b, br),
    record(name),
  }

  // ---------------------------------------------------------------------------------------
  // Server actions.

  /// A debounced burst settles: `ChangeCoordinator::settle` calls `publish_next`, which
  /// advances the revision by one and stores the change set with it, atomically.
  action publishWith(kinds: Set[Kind], reason: bool): bool = {
    val s = { instance: committed.instance, revision: committed.revision + 1 }
    val c = changeSet(kinds, reason)
    all {
      up,
      kinds != Set() or reason,
      up' = up,
      committed' = s,
      latest' = c,
      history' = history.put(s, c),
      browsers' = browsers,
      record("publish"),
    }
  }

  action publish = {
    nondet kinds = KINDS.powerset().oneOf()
    nondet reason = Set(true, false).oneOf()
    publishWith(kinds, reason)
  }

  /// A hot-only publish (css, js and assets, no full reason): a subset of `publish`, listed
  /// in `step` beside it to weight the simulator toward the hot path.
  action publishHot = {
    nondet kinds = HOT_KINDS.powerset().exclude(Set(Set())).oneOf()
    publishWith(kinds, false)
  }

  /// The server stops (a clean stop or a kill): every event stream ends, and what was in
  /// flight is lost. Pages stay as they are.
  action stopServer: bool = all {
    up,
    up' = false,
    committed' = committed,
    latest' = latest,
    history' = history,
    browsers' = browsers.keys().mapBy(b => closed(browsers.get(b))),
    record("stopServer"),
  }

  /// A new process (on the same port, or another one the page cannot reach anyway): a new
  /// instance at revision 0 with `ReloadState::new`'s full, empty change set. Edits made while
  /// stopped are in the files it serves; the new instance alone makes every page reload.
  action startServer: bool = {
    val s = { instance: committed.instance + 1, revision: 0 }
    all {
      not(up),
      up' = true,
      committed' = s,
      latest' = INITIAL_CHANGE,
      history' = history.put(s, INITIAL_CHANGE),
      browsers' = browsers,
      record("startServer"),
    }
  }

  // ---------------------------------------------------------------------------------------
  // Connection actions.

  /// The client opens `/__site_preview/events` (the first connect, the browser's own retry,
  /// or client.js's re-creation after a refusal): the server subscribes, then reads the
  /// current value and sends it as the `state` baseline (subscribe before read, ADR 0013).
  action connect(b: str): bool = {
    val br = browsers.get(b)
    all {
      up,
      br.doc == Shown,
      not(br.open),
      browserOnly("connect", b, {
        ...br,
        open: true,
        lastSent: committed,
        stream: [stateEvent(committed)],
        delivered: 0,
      }),
    }
  }

  /// The connection drops (network, proxy, sleep): the page keeps its baseline.
  action disconnect(b: str): bool = {
    val br = browsers.get(b)
    all {
      br.open,
      browserOnly("disconnect", b, closed(br)),
    }
  }

  /// The connection's `Feed` wakes on a newer value and sends one `reload` event for the
  /// newest value only: publishes since the last send collapse into it.
  action send(b: str): bool = {
    val br = browsers.get(b)
    all {
      up,
      br.open,
      newer(committed, br.lastSent),
      browserOnly("send", b, {
        ...br,
        lastSent: committed,
        stream: br.stream.append(reloadEvent(br.lastSent, committed, latest)),
      }),
    }
  }

  // ---------------------------------------------------------------------------------------
  // Browser actions.

  /// A navigation finishes: the page is fetched with the current state injected as its
  /// baseline (`ReloadState::current` read before the file is opened). From `NoPage` this
  /// opens the page, from `Navigating` it completes the client's reload.
  action navigate(b: str): bool = {
    val br = browsers.get(b)
    all {
      up,
      br.doc != Shown,
      browserOnly("navigate", b, { ...closed(br), doc: Shown, baseline: committed }),
    }
  }

  /// The user reloads a shown page (or follows a link): a new render-time baseline, a new
  /// stream later; not a reaction of the client.
  action userReload(b: str): bool = {
    val br = browsers.get(b)
    all {
      up,
      br.doc == Shown,
      browserOnly("userReload", b, { ...closed(br), baseline: committed }),
    }
  }

  /// The client handles the next event in flight. `applied` is whether a hot apply worked
  /// in place (css/asset swaps loaded, every script accepted); otherwise it falls back to a
  /// reload.
  action receiveWith(b: str, applied: bool): bool = {
    val br = browsers.get(b)
    val e = br.stream[br.delivered]
    val handled = { ...br, delivered: br.delivered + 1 }
    val reload = {
      ...closed(handled),
      doc: Navigating,
      reloaded: br.reloaded + 1,
      trigger: e.at,
    }
    val after = match decide(br.baseline, e) {
      | Ignore => handled
      | TryHot =>
        if (applied and applicable(e.kinds))
          { ...handled, baseline: e.at, hotApplied: br.hotApplied + 1, trigger: e.at }
        else reload
      | MustReload => reload
    }
    all {
      br.doc == Shown,
      br.delivered < br.stream.length(),
      browserOnly("receive", b, after),
    }
  }

  action receive(b: str): bool = {
    nondet applied = Set(true, false).oneOf()
    receiveWith(b, applied)
  }

  // ---------------------------------------------------------------------------------------
  // The step relation.

  /// A browser's protocol makes progress: its stream opens, the server sends, the client
  /// handles an event, a navigation finishes.
  action progress = {
    nondet b = BROWSERS.oneOf()
    any { connect(b), send(b), receive(b), navigate(b) }
  }

  /// A browser is interrupted: its stream drops, or the user reloads the page.
  action interrupt = {
    nondet b = BROWSERS.oneOf()
    any { disconnect(b), userReload(b) }
  }

  /// A stop, or the start of a new process after one.
  action restart = any { stopServer, startServer }

  /// Every action of the model. `progress` is listed several times, and `publishHot` beside
  /// `publish`, only to weight the simulator, which picks one of the enabled disjuncts
  /// uniformly: without it a restart or a drop interrupts nearly every hot apply before it
  /// completes. Neither changes the relation (`publishHot` is a subset of `publish`).
  action step = any {
    publish,
    publishHot,
    restart,
    interrupt,
    progress,
    progress,
    progress,
    progress,
    progress,
    progress,
    progress,
    progress,
  }

  // ---------------------------------------------------------------------------------------
  // Safety invariants (foundation.md section 6.1 item 3, FORMAL-03). Each is checked by
  // `quint run --invariant` under `step`.

  /// Every stamp the system holds: baselines, triggers, what each connection last sent and
  /// every event sent.
  def claims(br: Browser): Set[Stamp] =
    Set(br.baseline, br.trigger, br.lastSent)
      .union(br.stream.foldl(Set(), (acc, e) => acc.union(Set(e.at))))
      .exclude(Set(NONE))

  /// Whether the events of `l` announce strictly newer states, one after the other.
  pure def increasing(l: List[Event]): bool =
    l.indices().forall(k => k == 0 or newer(l[k].at, l[k - 1].at))

  /// The revision only moves forward: a publish adds exactly one, a restart only ever moves
  /// to a newer instance, and on every connection the events announce strictly newer states,
  /// the last one being what the connection's `Feed` remembers.
  val monotonicRevision = and {
    not(newer(before.committed, committed)),
    (committed.instance == before.committed.instance) implies
      (committed.revision == before.committed.revision
        or committed.revision == before.committed.revision + 1),
    BROWSERS.forall(b => {
      val br = browsers.get(b)
      and {
        increasing(br.stream),
        br.open implies (br.stream != [] and br.stream[br.stream.length() - 1].at == br.lastSent),
        not(br.open) implies br.stream == [],
      }
    }),
  }

  /// No notification (and no baseline) claims a state that was never committed, and no event
  /// claims a change set other than the one committed with its revision: a hot `reload`
  /// event lists exactly the committed kinds, a committed full set is never sent hot, and a
  /// gap's event lists nothing.
  val noUncommittedClaim = BROWSERS.forall(b => {
    val br = browsers.get(b)
    and {
      claims(br).forall(s => history.keys().contains(s) and not(newer(s, committed))),
      br.stream.indices().forall(k => {
        val e = br.stream[k]
        val c = history.get(e.at)
        e.name == ReloadEvent implies and {
          e.kinds == Set() or e.kinds == c.kinds,
          c.full implies e.full,
          not(e.full) implies e.kinds == c.kinds,
        }
      }),
    }
  })

  /// Change sets are pages ahead or not at all: an `html` or `other` kind makes the set full
  /// (`FullReason::DocumentChanged`), on the server and on the wire, and no browser ever
  /// applies one in place.
  val htmlChangeIsFull = and {
    history.keys().forall(s => {
      val c = history.get(s)
      not(c.kinds.subseteq(HOT_KINDS)) implies c.full
    }),
    BROWSERS.forall(b => {
      val br = browsers.get(b)
      and {
        br.stream.indices().forall(k => {
          val e = br.stream[k]
          not(e.kinds.subseteq(HOT_KINDS)) implies e.full
        }),
        br.hotApplied > before.browsers.get(b).hotApplied implies
          history.get(br.trigger).kinds.subseteq(HOT_KINDS),
      }
    }),
  }

  /// A client reacts (hot apply or reload) at most once per step, and only to a state newer
  /// than both its baseline and everything it reacted to before: one change (or one restart)
  /// never costs a page two reloads, and an unchanged server none.
  val atMostOneReloadPerChange = BROWSERS.forall(b => {
    val was = before.browsers.get(b)
    val now = browsers.get(b)
    and {
      reactions(now) >= reactions(was),
      reactions(now) <= reactions(was) + 1,
      reactions(now) > reactions(was) implies and {
        was.doc == Shown,
        newer(now.trigger, was.baseline),
        newer(now.trigger, was.trigger),
      },
      reactions(now) == reactions(was) implies now.trigger == was.trigger,
    }
  })

  /// Never hot-apply across a gap: a client hot-applies only the event whose revision is its
  /// baseline + 1 on the same instance, and the server never offers a hot (`full: false`)
  /// `reload` that is not the revision after the one the connection sent before it.
  ///
  /// The two layers are each sufficient: with a correct server the client's `isNext` check
  /// in `decide` never decides anything (removing it alone violates nothing), and with a
  /// server that ignores the gap the client check alone keeps the client clause; only both
  /// faults together hot-apply across a gap. FORMAL-07 checks the server layer against the
  /// real SSE handler, the client tests check the other.
  val noHotApplyAcrossGap = BROWSERS.forall(b => {
    val was = before.browsers.get(b)
    val now = browsers.get(b)
    and {
      now.hotApplied > was.hotApplied implies and {
        now.trigger.instance == was.baseline.instance,
        now.trigger.revision == was.baseline.revision + 1,
      },
      now.stream.indices().forall(k => {
        val e = now.stream[k]
        (e.name == ReloadEvent and not(e.full)) implies and {
          k > 0,
          e.at.instance == now.stream[k - 1].at.instance,
          e.at.revision == now.stream[k - 1].at.revision + 1,
        }
      }),
    }
  })

  /// A hot apply moves the baseline to the state it applied, with no navigation; otherwise
  /// the baseline changes only by a navigation, to the state current at that moment.
  val hotApplyAdvancesBaseline = BROWSERS.forall(b => {
    val was = before.browsers.get(b)
    val now = browsers.get(b)
    and {
      now.hotApplied > was.hotApplied implies and {
        now.baseline == now.trigger,
        now.doc == Shown,
        now.reloaded == was.reloaded,
      },
      (now.baseline != was.baseline and now.hotApplied == was.hotApplied) implies
        (now.baseline == committed and taken.in(Set("navigate", "userReload"))),
    }
  })

  /// A connected client that has handled every event sent to it is at the state its
  /// connection last announced: nothing it was told is left unanswered (the safety half of
  /// `reconnectObservesNewer`).
  val drainedClientIsCurrent = BROWSERS.forall(b => {
    val br = browsers.get(b)
    (br.doc == Shown and br.open and br.delivered == br.stream.length()) implies
      br.baseline == br.lastSent
  })

  /// Every invariant at once.
  val allInvariants = and {
    monotonicRevision,
    noUncommittedClaim,
    atMostOneReloadPerChange,
    noHotApplyAcrossGap,
    htmlChangeIsFull,
    hotApplyAdvancesBaseline,
    drainedClientIsCurrent,
  }

  // ---------------------------------------------------------------------------------------
  // Liveness (foundation.md section 6.1 item 3): stated, not simulated. `quint run` checks
  // only the safety invariants above; FORMAL-10's Apalache lane is where a temporal property
  // can be checked over bounded behaviours.

  /// A shown or navigating page whose baseline is older than the server's state.
  def stale(b: str): bool = {
    val br = browsers.get(b)
    br.doc != NoPage and newer(committed, br.baseline)
  }

  /// The actions that make the channel unavailable or move the target: a publish, a drop, a
  /// stop or a restart.
  pure val DISRUPTIVE: Set[str] = Set("publish", "disconnect", "stopServer", "startServer")

  /// Fairness: the protocol's own steps are not postponed for ever while they stay enabled.
  /// The browser keeps retrying a closed stream (its native retry, or client.js's backoff
  /// after a refusal, capped at 30 s), the server's `Feed` sends once it has a newer value
  /// (the watch channel wakes it), TCP delivers what was sent, and a started navigation
  /// finishes while the server runs. Opening a page and the user's reloads are not assumed.
  temporal fairness = BROWSERS.forall(b => and {
    weakFair(connect(b), (up, committed, latest, history, browsers)),
    weakFair(send(b), (up, committed, latest, history, browsers)),
    weakFair(receive(b), (up, committed, latest, history, browsers)),
    weakFair(navigate(b), (up, committed, latest, history, browsers)),
  })

  /// The environment's assumption: from some point on the server keeps running, the
  /// connection is not dropped and the site is not edited (foundation.md: "if the server and
  /// channel remain available"). Without it no liveness is possible: a stream that is
  /// dropped before every delivery, or edits faster than any page loads, starve the page.
  temporal channelAvailable = eventually(always(up and not(taken.in(DISRUPTIVE))))

  /// A page that is behind, whether connected, disconnected or reloading, eventually catches
  /// up: it reconnects, gets the authoritative `state`, and reloads once or hot-applies.
  ///
  /// Why it holds under the assumptions: once the environment is quiet, a stale page always
  /// has a fair action enabled. Navigating: `navigate` (the server is up). Shown and closed:
  /// `connect`. Shown and open with events in flight: `receive`, which reacts to any newer
  /// state (`decide`). Shown, open and drained: `drainedClientIsCurrent` makes the baseline
  /// the connection's `lastSent`, so the page is stale only if `send` is enabled. Each of
  /// these moves the page to the committed state in at most four steps.
  temporal reconnectObservesNewer =
    BROWSERS.forall(b => always(stale(b) implies eventually(not(stale(b)))))

  /// The liveness claim, under its assumptions.
  temporal liveness = (fairness and channelAvailable) implies reconnectObservesNewer

  // ---------------------------------------------------------------------------------------
  // Witnesses: states a simulation should reach (`quint run --witnesses ...`).

  val reachesHotApply = BROWSERS.exists(b => browsers.get(b).hotApplied > 0)
  val reachesReload = BROWSERS.exists(b => browsers.get(b).reloaded > 0)
  val reachesCoalescedGap = BROWSERS.exists(b => {
    val s = browsers.get(b).stream
    s.indices().exists(k => k > 0 and s[k].name == ReloadEvent and s[k].full and s[k].kinds == Set()
      and s[k].at.revision > s[k - 1].at.revision + 1)
  })
  val reachesRestart = committed.instance > 1
}
