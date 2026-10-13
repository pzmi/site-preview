// -*- mode: Bluespec; -*-
/**
 * The file watcher pipeline (foundation.md section 6.1 item 2, plan.md sections 8.2 to 8.4),
 * retrofitted onto the code that exists (ADR 0014, ADR 0054):
 *
 * - classification: `classify` and `Classification::requests_reconcile` in
 *   `crates/preview-domain/src/change.rs` (`classify` below, for the paths of the model);
 * - the change set: `ChangeSet::accumulate_classified` in `change_set.rs` (`accumulate`);
 * - the debounce: the global trailing-edge `Debouncer` in `debounce.rs` (`touch`,
 *   `markOverflow`, `markRecovered`; a fire is `settle`);
 * - the coordinator: `ChangeCoordinator::{receive, record, overflow, settle, watcher_failed,
 *   watcher_ready, watcher_reconciled}` in `crates/preview-application/src/commands/`, with
 *   the health machine `WatcherHealth::step` in `crates/preview-domain/src/watcher_health.rs`;
 * - the source: the bridge of `crates/preview-adapter-fs/src/watcher/bridge.rs` (ADR 0048): a
 *   bounded channel (`CAP` messages here, 1024 in the code), an overflow flag set when the
 *   channel is full or the OS reports a loss, a failure flag beside it, and `next`'s order
 *   (overflow, then failure, then queued messages, consecutive changes merged into one batch);
 * - the reconcile: `NotifyWatcher::reconcile` (ADR 0050) as composition reports it
 *   (`reconciled` in `bootstrap.rs`): ready only when nothing is outstanding, a directory that
 *   newly cannot be watched is a problem that degrades, and passing the relisting cap marks a
 *   loss, so the next batch is `Overflow`;
 * - the pipeline task: `WatchPipeline` in `crates/preview-composition/src/watch_pipeline.rs`
 *   (ADR 0049): cancellation first, then the debounce deadline (armed only when the coordinator
 *   has one), then the next batch; `reconcile(Changed)` at once when a batch asks for it,
 *   `reconcile(Full)` for an overflow and after a rescan revision unless one already ran.
 *
 * The site is abstract: a root holding `index.html` (`Index`, a document), `a.css` and `c.css`
 * (`Css`, `Css2`, hot) and one eligible directory `d` (`DirD`) holding `d/b.js` (`Sub`, hot);
 * plus an editor temporary name (`Temp`, `.index.html.swp`) and an ignored directory's file
 * (`Git`, `.git/HEAD`). The model's OS turns each change into the notifications a native
 * watcher emits for it, through the watch of the directory that holds the entry (`Root`, or
 * `DirD` for `Sub`): a change in a directory without a watch is never notified.
 *
 * Time is a counter of ticks; the window is `WINDOW` ticks (FORMAL-06 maps a tick to
 * 150 ms / `WINDOW`). The timer exists only while the debouncer is dirty (deadline-only), and
 * a due deadline takes priority over a ready batch.
 *
 * Deliberately outside the model (ADR 0054): revision exhaustion (2^64 publishes; pinned by
 * `should_degrade_once_and_refuse_when_revisions_are_exhausted`), the 64-path cap of a hot
 * change set, nesting below `d` (one level is enough for the reconcile cases: a reconcile of
 * changes reaches `d` either as its own entry, as the root listed one level deep, or as an
 * outstanding retry, so `Changed` and `Full` act alike on this tree), a watcher that cannot be
 * created at all, and `Closed` (only after shutdown here). A root deletion leaves the server
 * on the deleted directory, as the root capability does (plan.md section 5.3): nothing served
 * can change after it, and a directory recreated at the same path is not served. `rm -r` of
 * the root is two steps, the entries below it (`rootEmpty`) then the root (`rootDelete`), and
 * the root's own watch never reports the second, because the held descriptor keeps its inode;
 * the watch of the root's parent does (ADR 0073). The next reconcile finds the root path no
 * longer names the served directory and degrades with `RootRemoved` (ADR 0067), which nothing
 * recovers: live reload is never claimed for a root that is gone (`noLiveClaimWithoutRoot`).
 */
module watcher {
  // ---------------------------------------------------------------------------------------
  // Types.

  /// The abstract entries of the site, the root itself included.
  type Path = Index | Css | Css2 | Sub | DirD | Temp | Git | Root

  /// `ChangeKind`, as the adapter translates `notify`'s kinds (`translate.rs`). Access-only
  /// and metadata-only changes are dropped by the bridge before they are queued (ADR 0048),
  /// so they never reach the model's channel.
  type Kind = KCreate | KModify | KRemove | KRename | KDirChanged | KAmbiguous

  /// One raw change, `ChangeEvent`.
  type Notif = { kind: Kind, path: Path }

  /// `SourceFailure`: a registration failure, a runtime failure, or the served root no longer
  /// at its path (ADR 0067).
  type Reason = RegistrationFailed | RuntimeFailure | RootRemoved

  /// One message in the bridge's channel.
  type Msg = MEvent(Notif) | MFailed(Reason)

  /// One delivery of `ChangeSource::next` (`ChangeBatch`), or nothing ready.
  type Batch = BEvents(List[Notif]) | BOverflow | BFailed(Reason) | BNone

  /// `Classification`, with `Relevance` flattened (`Undisclosed` is unreachable: every
  /// eligible name of the model is eligible as a directory too).
  type Class = CIgnored | CRescan | CFile | CFileOrDir | CDirectory

  /// `WatcherHealth` (the directory count is left out: it is at least one whenever ready).
  type Health = HStarting | HWatching | HDegraded(Reason)

  /// The health events the coordinator returns.
  type HealthEvent = EvDegraded(Reason) | EvRecovered

  /// The debouncer and the change set it accumulates: `Debouncer::pending` and `changes`.
  type Deb = { dirty: bool, deadline: int, rescan: bool, full: bool, paths: Set[Path] }

  /// The site on disk.
  type Fs = {
    // The served files that exist (`Sub` only while `d` exists).
    present: Set[Path],
    // Whether `d` exists.
    dPresent: bool,
    // Whether `d` cannot be watched (watch limit reached, or no permission to read it).
    dUnwatchable: bool,
    // Whether the served root directory still exists.
    rootPresent: bool,
    // Whether a new directory was created at the root's path after its deletion.
    rootRecreated: bool,
  }

  /// The change source: the channel and flags of the bridge, the native watches, and the
  /// directories the adapter could not watch (reported once, retried at each reconcile).
  type Src = {
    queue: List[Msg],
    overflow: bool,
    failure: bool,
    // Directories with a working native watch.
    watched: Set[Path],
    // Eligible directories that could not be watched (`NotifyWatcher`'s failed set).
    outstanding: Set[Path],
    // The OS is dropping notifications (its own queue overflowed): each one dropped raises
    // the overflow flag (`notify`'s `Rescan` flag).
    osDropping: bool,
  }

  /// Bookkeeping for the invariants; never read by the pipeline.
  type Ghost = {
    // Served files whose content changed since the last revision that covers them.
    pending: Set[Path],
    // The subset changed while their directory could not be watched and its failure was
    // reported: the watcher could not see them (failureIsExplicit's domain).
    excused: Set[Path],
    // Clean-to-dirty transitions of the debouncer: one per burst.
    cycles: int,
    // When a relevant change or an overflow was last recorded.
    lastRecordAt: int,
    // Since the debouncer became dirty, a received notification named a served entry (by
    // the ground truth `SERVED_ENTRIES`, never by `classify`), or an overflow arrived.
    eligibleCause: bool,
    // A full reconcile ran since the previous revision.
    fullSinceFire: bool,
    degradedEvents: int,
    recoveredEvents: int,
    // Degradations reported while already degraded: the escalation to `RootRemoved`.
    escalations: int,
  }

  /// What the last action did.
  type Last = {
    fired: bool,
    firedFull: bool,
    firedPaths: Set[Path],
    firedRescan: bool,
    // The fired revision came a full window after the latest change recorded before it (a
    // recovery during the revision's own reconcile records after it).
    firedQuiet: bool,
    // The fired revision asked for a rescan and a full reconcile ran for it.
    fullDone: bool,
    // A batch of changes was received and every change named an ineligible entry.
    ineligibleOnly: bool,
    // How many changes of the received batch reached the debouncer.
    recorded: int,
    needsReconcile: bool,
    debBefore: Deb,
    healthBefore: Health,
    events: List[HealthEvent],
  }

  type World = {
    now: int,
    fs: Fs,
    src: Src,
    deb: Deb,
    health: Health,
    revision: int,
    reconciledFully: bool,
    cancelled: bool,
    ghost: Ghost,
    last: Last,
  }

  // ---------------------------------------------------------------------------------------
  // Constants.

  /// The debounce window in ticks.
  pure val WINDOW = 2

  /// The channel capacity in messages (`DEFAULT_CAPACITY` is 1024).
  pure val CAP = 4

  /// The served files.
  pure val SERVED: Set[Path] = Set(Index, Css, Css2, Sub)

  /// Ground truth for `onlyEligibleTriggerReload`: the entries whose change can change what
  /// is served (the files, the directory holding one, the root).
  pure val SERVED_ENTRIES: Set[Path] = Set(Index, Css, Css2, Sub, DirD, Root)

  pure val CLEAN: Deb = { dirty: false, deadline: 0, rescan: false, full: false, paths: Set() }

  // ---------------------------------------------------------------------------------------
  // The domain: eligibility, classification and the change set.

  /// `eligibility::components`: hidden names (`.git`, `.index.html.swp`) are denied as a file
  /// and as a directory; the other names are eligible both ways.
  pure def eligible(p: Path): bool = p != Temp and p != Git

  /// `classify`, row for row: ineligible first, then ambiguous, a known directory change, any
  /// other event on the root (`Rescan`), a data modification, and an entry of unknown type.
  pure def classify(n: Notif): Class =
    if (not(eligible(n.path))) CIgnored
    else match n.kind {
      | KAmbiguous => CRescan
      | KDirChanged => CDirectory
      | KModify => if (n.path == Root) CRescan else CFile
      | _ => if (n.path == Root) CRescan else CFileOrDir
    }

  /// `Classification::requests_reconcile`.
  pure def requestsReconcile(c: Class): bool =
    c == CDirectory or c == CFileOrDir or c == CRescan

  /// `AssetKind::is_hot`: css and js are applied in place, html is a full reload.
  pure def isHot(p: Path): bool = p == Css or p == Css2 or p == Sub

  /// `ChangeSet::accumulate_classified`: a rescan and a directory change mark the set full;
  /// a file adds its path, and marks the set full when it is not hot or was removed (a
  /// rename stays hot: atomic saves rename onto the target).
  pure def accumulate(d: Deb, n: Notif, c: Class): Deb = match c {
    | CIgnored => d
    | CRescan => { ...d, full: true }
    | CDirectory => { ...d, full: true }
    | _ => {
      ...d,
      paths: d.paths.union(Set(n.path)),
      full: d.full or not(isHot(n.path)) or n.kind == KRemove,
    }
  }

  /// `Debouncer::touch`: dirty, with the deadline a window after now.
  pure def touch(d: Deb, now: int, rescan: bool): Deb =
    { ...d, dirty: true, deadline: now + WINDOW, rescan: d.rescan or rescan }

  /// `Debouncer::mark_overflow`: dirty, a rescan, and a full set.
  pure def markOverflow(d: Deb, now: int): Deb = { ...touch(d, now, true), full: true }

  /// `Debouncer::poll` at `now` would fire.
  pure def due(d: Deb, now: int): bool = d.dirty and now >= d.deadline

  // ---------------------------------------------------------------------------------------
  // The health machine (`WatcherHealth::step`) as the coordinator applies it.

  pure def isDegraded(h: Health): bool = match h {
    | HDegraded(_) => true
    | _ => false
  }

  /// `ChangeCoordinator::degrade` (`WatcherHealth::step`): the first failure degrades and
  /// reports once; a further failure while degraded is absorbed, except a root removal, which
  /// escalates once (the user learns why live reload cannot come back) and then absorbs
  /// everything (ADR 0067).
  pure def degrade(w: World, r: Reason): World =
    if (w.health == HDegraded(RootRemoved)) w
    else if (isDegraded(w.health) and r != RootRemoved) w
    else {
      ...w,
      health: HDegraded(r),
      ghost: {
        ...w.ghost,
        degradedEvents: w.ghost.degradedEvents + 1,
        escalations: w.ghost.escalations + (if (isDegraded(w.health)) 1 else 0),
      },
      last: { ...w.last, events: w.last.events.append(EvDegraded(r)) },
    }

  /// `Debouncer::mark_recovered`: dirty with a full set (`FullReason::Recovered`), no rescan.
  pure def markRecovered(d: Deb, now: int): Deb = { ...touch(d, now, false), full: true }

  /// `ChangeCoordinator::watcher_reconciled`: a recovery after a failure is reported once,
  /// and the next revision is full (ADR 0054): a change made in a directory while it could
  /// not be watched was never notified, so only a full reload catches it up. Readiness from
  /// `Starting` marks nothing (no spurious reload at start-up). Readiness after a root
  /// removal is refused (`WatcherHealth::step`): the health is unchanged.
  pure def ready(w: World): World =
    if (w.health == HDegraded(RootRemoved)) w
    else if (isDegraded(w.health)) {
      ...w,
      health: HWatching,
      deb: markRecovered(w.deb, w.now),
      ghost: {
        ...w.ghost,
        recoveredEvents: w.ghost.recoveredEvents + 1,
        cycles: w.ghost.cycles + (if (w.deb.dirty) 0 else 1),
        lastRecordAt: w.now,
        eligibleCause: true,
      },
      last: { ...w.last, events: w.last.events.append(EvRecovered) },
    }
    else { ...w, health: HWatching }

  // ---------------------------------------------------------------------------------------
  // The source: the bridge's offer and `next`.

  /// The watch whose notifications report a change of `p`.
  pure def watchOf(p: Path): Path = if (p == Sub) DirD else Root

  /// `Bridge::offer`: `try_send`, and on a full channel the overflow flag (with the failure
  /// flag for a failure).
  pure def offer(s: Src, m: Msg): Src =
    if (s.queue.length() < CAP) { ...s, queue: s.queue.append(m) }
    else match m {
      | MEvent(_) => { ...s, overflow: true }
      | MFailed(_) => { ...s, overflow: true, failure: true }
    }

  /// One notification of the OS: none without a watch on its directory (or after shutdown),
  /// a loss while the OS drops, else offered to the channel.
  pure def emitOne(w: World, n: Notif): World =
    if (w.cancelled or not(w.src.watched.contains(watchOf(n.path)))) w
    else if (w.src.osDropping) { ...w, src: { ...w.src, overflow: true } }
    else { ...w, src: offer(w.src, MEvent(n)) }

  pure def emit(w: World, ns: List[Notif]): World = ns.foldl(w, (acc, n) => emitOne(acc, n))

  /// One notification through the watch of the root's parent directory (ADR 0073): the root
  /// entry itself removed or created there. That watch is not a site directory, so it is not
  /// in `watched`, and nothing but shutdown removes it; what it reports about other entries
  /// of the parent is outside the root and dropped by the bridge, so the model omits it.
  pure def emitParent(w: World, n: Notif): World =
    if (w.cancelled) w
    else if (w.src.osDropping) { ...w, src: { ...w.src, overflow: true } }
    else { ...w, src: offer(w.src, MEvent(n)) }

  /// A registration failure reported at start-up that the pipeline has not received yet. Only
  /// start-up queues one, first, and nothing moves a failure (`duplicate` and `reorder` touch
  /// changes only), so it is always the head of the queue.
  pure def registrationFailureQueued(s: Src): bool =
    s.queue.length() > 0 and s.queue[0] == MFailed(RegistrationFailed)

  /// How many event messages head the queue before a failure.
  pure def leadingEvents(q: List[Msg]): int =
    q.foldl({ n: 0, open: true }, (acc, m) => match m {
      | MEvent(_) => if (acc.open) { n: acc.n + 1, open: true } else acc
      | MFailed(_) => { ...acc, open: false }
    }).n

  pure def notifsOf(q: List[Msg]): List[Notif] =
    q.foldl([], (acc, m) => match m {
      | MEvent(n) => acc.append(n)
      | MFailed(_) => acc
    })

  /// `NotifyChangeSource::next`: the start-up registration failure first (held apart from
  /// the channel in the code; here the head of the queue, the only place it can be), then the
  /// overflow flag, then the failure flag, then a queued failure alone or up to `k`
  /// consecutive queued changes as one batch (fewer than are queued when the rest has not
  /// arrived yet). The start-up failure goes first so that an overflow's reconcile cannot
  /// report readiness before it and leave it to degrade a watcher with every directory
  /// watched (FORMAL-10).
  pure def nextBatch(s: Src, k: int): (Batch, Src) =
    if (registrationFailureQueued(s))
      (BFailed(RegistrationFailed), { ...s, queue: s.queue.slice(1, s.queue.length()) })
    else if (s.overflow) (BOverflow, { ...s, overflow: false })
    else if (s.failure) (BFailed(RuntimeFailure), { ...s, failure: false })
    else if (s.queue.length() == 0) (BNone, s)
    else match s.queue[0] {
      | MFailed(r) => (BFailed(r), { ...s, queue: s.queue.slice(1, s.queue.length()) })
      | MEvent(_) => {
        val lead = leadingEvents(s.queue)
        val n = if (k < lead) k else lead
        (
          BEvents(notifsOf(s.queue.slice(0, n))),
          { ...s, queue: s.queue.slice(n, s.queue.length()) },
        )
      }
    }

  // ---------------------------------------------------------------------------------------
  // The reconcile (ADR 0050) and how composition reports it.

  /// Applies a reconcile's outcome: a new problem degrades (`RegistrationFailed`); a
  /// complete watch set is readiness, which recovers a degraded watcher; passing the
  /// relisting cap is incomplete and raises the overflow flag.
  pure def reconciled(w: World, watched: Set[Path], outstanding: Set[Path], problem: bool, capHit: bool): World = {
    val w2 = { ...w, src: { ...w.src, watched: watched, outstanding: outstanding, overflow: w.src.overflow or capHit } }
    if (problem) degrade(w2, RegistrationFailed)
    else if (outstanding == Set() and not(capHit)) ready(w2)
    else w2
  }

  /// `WatchControl::reconcile`: `d` is dropped when gone, registered when it exists without
  /// a watch, kept outstanding (reported once) when it cannot be watched; `capped` is a
  /// registration that passed the relisting cap. With the root gone (deleted, or replaced by
  /// another directory at its path) nothing is walked or registered, and the watcher degrades
  /// with `RootRemoved` (ADR 0067): the served descriptor is no longer the root path, so no
  /// watch could ever report what is served again.
  pure def reconcile(w0: World, full: bool, capped: bool): World = {
    val w = {
      ...w0,
      reconciledFully: w0.reconciledFully or full,
      ghost: { ...w0.ghost, fullSinceFire: w0.ghost.fullSinceFire or full },
    }
    val s = w.src
    if (not(w.fs.rootPresent)) degrade(w, RootRemoved)
    else if (not(w.fs.dPresent))
      reconciled(w, s.watched.exclude(Set(DirD)), s.outstanding.exclude(Set(DirD)), false, false)
    else if (s.watched.contains(DirD))
      reconciled(w, s.watched, s.outstanding.exclude(Set(DirD)), false, false)
    else if (w.fs.dUnwatchable)
      reconciled(w, s.watched, s.outstanding.union(Set(DirD)), not(s.outstanding.contains(DirD)), false)
    else if (capped) reconciled(w, s.watched, s.outstanding, false, true)
    else reconciled(w, s.watched.union(Set(DirD)), s.outstanding.exclude(Set(DirD)), false, false)
  }

  // ---------------------------------------------------------------------------------------
  // The coordinator's commands and the pipeline's steps.

  /// `ChangeCoordinator::record` for one change at `now`.
  pure def recordOne(w: World, n: Notif): World = {
    val c = classify(n)
    if (c == CIgnored) w
    else {
      val wasClean = not(w.deb.dirty)
      val served = SERVED_ENTRIES.contains(n.path)
      {
        ...w,
        deb: accumulate(touch(w.deb, w.now, c == CRescan), n, c),
        ghost: {
          ...w.ghost,
          cycles: w.ghost.cycles + (if (wasClean) 1 else 0),
          lastRecordAt: w.now,
          eligibleCause: if (wasClean) served else w.ghost.eligibleCause or served,
        },
        last: {
          ...w.last,
          recorded: w.last.recorded + 1,
          needsReconcile: w.last.needsReconcile or requestsReconcile(c),
        },
      }
    }
  }

  /// `WatchPipeline::receive` of one batch.
  pure def receiveBatch(w: World, b: Batch, capped: bool): World = match b {
    | BEvents(ns) => {
      val w1 = ns.foldl(w, (acc, n) => recordOne(acc, n))
      val w2 = {
        ...w1,
        last: { ...w1.last, ineligibleOnly: ns.indices().forall(i => not(SERVED_ENTRIES.contains(ns[i].path))) },
      }
      if (w2.last.needsReconcile) reconcile(w2, false, capped) else w2
    }
    | BOverflow => {
      val w1 = {
        ...w,
        deb: markOverflow(w.deb, w.now),
        ghost: {
          ...w.ghost,
          cycles: w.ghost.cycles + (if (w.deb.dirty) 0 else 1),
          lastRecordAt: w.now,
          eligibleCause: true,
        },
        last: { ...w.last, needsReconcile: true },
      }
      reconcile(w1, true, capped)
    }
    | BFailed(r) => degrade(w, r)
    | BNone => w
  }

  /// `WatchPipeline::settle`: publish the next revision with the accumulated change set,
  /// then a full reconcile when the revision asks for a rescan and none ran since the
  /// previous revision.
  pure def settleAt(w: World, capped: bool): World = {
    val d = w.deb
    val covered = if (d.full) SERVED else d.paths.intersect(SERVED)
    val w1 = {
      ...w,
      revision: w.revision + 1,
      deb: CLEAN,
      ghost: {
        ...w.ghost,
        pending: w.ghost.pending.exclude(covered),
        excused: w.ghost.excused.exclude(covered),
        eligibleCause: false,
        fullSinceFire: false,
      },
      last: {
        ...w.last,
        fired: true,
        firedFull: d.full,
        firedPaths: d.paths,
        firedRescan: d.rescan,
        firedQuiet: w.now >= w.ghost.lastRecordAt + WINDOW,
      },
    }
    val w2 = if (d.rescan and not(w.reconciledFully)) reconcile(w1, true, capped) else w1
    {
      ...w2,
      reconciledFully: false,
      ghost: { ...w2.ghost, fullSinceFire: false },
      last: { ...w2.last, fullDone: d.rescan and (w.ghost.fullSinceFire or w2.ghost.fullSinceFire) },
    }
  }

  /// The served files `ps` changed: they are pending until a revision covers them; those in
  /// a directory whose watch failed (and was reported) cannot be seen by the watcher.
  pure def changeServed(w: World, ps: Set[Path]): World = {
    val unseen = ps.filter(p => w.src.outstanding.contains(watchOf(p)) and not(w.src.watched.contains(watchOf(p))))
    { ...w, ghost: { ...w.ghost, pending: w.ghost.pending.union(ps), excused: w.ghost.excused.union(unseen) } }
  }

  // ---------------------------------------------------------------------------------------
  // State.

  var now: int
  var fs: Fs
  var src: Src
  /// The debouncer: FORMAL-06 observes `deb.dirty`.
  var deb: Deb
  /// `ChangeCoordinator::health()`.
  var health: Health
  /// The latest published revision (0 is the initial state).
  var revision: int
  /// `WatchPipeline::reconciled_fully`.
  var reconciledFully: bool
  /// The pipeline task was cancelled (shutdown).
  var cancelled: bool
  var ghost: Ghost
  var last: Last

  val world: World = {
    now: now,
    fs: fs,
    src: src,
    deb: deb,
    health: health,
    revision: revision,
    reconciledFully: reconciledFully,
    cancelled: cancelled,
    ghost: ghost,
    last: last,
  }

  /// A fresh `last` for an action starting from `w`.
  pure def begin(w: World): World = {
    ...w,
    last: {
      fired: false,
      firedFull: false,
      firedPaths: Set(),
      firedRescan: false,
      firedQuiet: false,
      fullDone: false,
      ineligibleOnly: false,
      recorded: 0,
      needsReconcile: false,
      debBefore: w.deb,
      healthBefore: w.health,
      events: [],
    },
  }

  action commit(w: World): bool = all {
    now' = w.now,
    fs' = w.fs,
    src' = w.src,
    deb' = w.deb,
    health' = w.health,
    revision' = w.revision,
    reconciledFully' = w.reconciledFully,
    cancelled' = w.cancelled,
    ghost' = w.ghost,
    last' = w.last,
  }

  /// After start-up (`start_watcher`, then `watcher_ready` when complete): the root and `d`
  /// are watched and the health is `Watching`; or `d` could not be watched, the health is
  /// still `Starting` and the source's first message is `Failed(RegistrationFailed)` (held
  /// ahead of the channel in the code; here it takes one of the `CAP` slots); or (`capped`)
  /// the registration of `d` passed the relisting cap, so `d` is left to the full reconcile
  /// of the `Overflow` the source reports first, and the health stays `Starting` until that
  /// reconcile reports readiness: no failure, and no claim of live reload before every
  /// directory is watched (`StartReport::is_complete`, FORMAL-10).
  action initStartup(dUnwatchable: bool, capped: bool): bool = {
    val deferred = capped and not(dUnwatchable)
    val w0: World = {
      now: 0,
      fs: {
        present: Set(Index, Css, Sub),
        dPresent: true,
        dUnwatchable: dUnwatchable,
        rootPresent: true,
        rootRecreated: false,
      },
      src: {
        queue: if (dUnwatchable) [MFailed(RegistrationFailed)] else [],
        overflow: deferred,
        failure: false,
        watched: if (dUnwatchable or deferred) Set(Root) else Set(Root, DirD),
        outstanding: if (dUnwatchable) Set(DirD) else Set(),
        osDropping: false,
      },
      deb: CLEAN,
      health: if (dUnwatchable or deferred) HStarting else HWatching,
      revision: 0,
      reconciledFully: false,
      cancelled: false,
      ghost: {
        pending: Set(),
        excused: Set(),
        cycles: 0,
        lastRecordAt: 0,
        eligibleCause: false,
        fullSinceFire: false,
        degradedEvents: 0,
        recoveredEvents: 0,
        escalations: 0,
      },
      last: {
        fired: false,
        firedFull: false,
        firedPaths: Set(),
        firedRescan: false,
        firedQuiet: false,
        fullDone: false,
        ineligibleOnly: false,
        recorded: 0,
        needsReconcile: false,
        debBefore: CLEAN,
        healthBefore: HStarting,
        events: [],
      },
    }
    commit(w0)
  }

  /// Start-up without a relisting cap.
  action initWith(dUnwatchable: bool): bool = initStartup(dUnwatchable, false)

  action init = {
    nondet dUnwatchable = oneOf(Set(false, true))
    nondet capped = oneOf(Set(false, true))
    initStartup(dUnwatchable, capped)
  }

  // ---------------------------------------------------------------------------------------
  // The OS: changes to the site and the notifications they cause.

  pure def note(k: Kind, p: Path): Notif = { kind: k, path: p }

  /// Whether `p` can be changed now: the root exists, and `d` for `Sub`.
  def reachable(p: Path): bool = fs.rootPresent and (p != Sub or fs.dPresent)

  /// A served file's data is written in place.
  action modify(p: Path): bool = all {
    fs.present.contains(p),
    reachable(p),
    commit(begin(world).changeServed(Set(p)).emit([note(KModify, p)])),
  }

  /// A served file is created.
  action createFile(p: Path): bool = all {
    SERVED.contains(p),
    not(fs.present.contains(p)),
    reachable(p),
    {
      val w = begin(world).changeServed(Set(p)).emit([note(KCreate, p)])
      commit({ ...w, fs: { ...w.fs, present: fs.present.union(Set(p)) } })
    },
  }

  /// A served file is deleted.
  action deleteFile(p: Path): bool = all {
    fs.present.contains(p),
    reachable(p),
    {
      val w = begin(world).changeServed(Set(p)).emit([note(KRemove, p)])
      commit({ ...w, fs: { ...w.fs, present: fs.present.exclude(Set(p)) } })
    },
  }

  /// A stylesheet is renamed: both names change, and the OS reports each side.
  action renameFile(oldName: Path, newName: Path): bool = all {
    fs.rootPresent,
    fs.present.contains(oldName),
    not(fs.present.contains(newName)),
    {
      val w = begin(world).changeServed(Set(oldName, newName)).emit([note(KRename, oldName), note(KRename, newName)])
      commit({ ...w, fs: { ...w.fs, present: fs.present.exclude(Set(oldName)).union(Set(newName)) } })
    },
  }

  /// An editor's atomic save: write a temporary file beside the target, then rename it onto
  /// the target.
  action saveAtomically(p: Path): bool = all {
    fs.present.contains(p),
    reachable(p),
    p != Sub,
    commit(begin(world).changeServed(Set(p)).emit([
      note(KCreate, Temp),
      note(KModify, Temp),
      note(KRename, Temp),
      note(KRename, p),
    ])),
  }

  /// An editor writes its swap file; nothing served changes.
  action tempWrite: bool = all {
    fs.rootPresent,
    commit(begin(world).emit([note(KModify, Temp)])),
  }

  /// Git writes `.git/HEAD`. `.git` is never watched; a backend whose streams are recursive
  /// by nature (FSEvents) may still report it (`seen`), and then classification drops it.
  action gitChange(seen: bool): bool = all {
    fs.rootPresent,
    commit(if (seen) begin(world).emit([note(KModify, Git)]) else begin(world)),
  }

  /// `rm -r d`: its file, then the directory, reported to the parent's watch; the kernel
  /// drops `d`'s watch.
  action dirDelete: bool = all {
    fs.dPresent,
    {
      val gone = fs.present.intersect(Set(Sub))
      val w0 = begin(world).changeServed(gone)
      val w1 = if (gone == Set()) w0 else w0.emit([note(KRemove, Sub)])
      val w2 = w1.emit([note(KDirChanged, DirD)])
      commit({
        ...w2,
        fs: { ...w2.fs, present: fs.present.exclude(Set(Sub)), dPresent: false, dUnwatchable: false },
        src: { ...w2.src, watched: w2.src.watched.exclude(Set(DirD)) },
      })
    },
  }

  /// `mkdir d`, which may be impossible to watch (`unwatchable`: the watch limit).
  action dirCreate(unwatchable: bool): bool = all {
    fs.rootPresent,
    not(fs.dPresent),
    {
      val w = begin(world).emit([note(KDirChanged, DirD)])
      commit({ ...w, fs: { ...w.fs, dPresent: true, dUnwatchable: unwatchable } })
    },
  }

  /// A watch is freed elsewhere (or `d` becomes readable): `d` could be watched now, but
  /// nothing reports it until a reconcile retries it.
  action freeWatch: bool = all {
    fs.dUnwatchable,
    commit({ ...begin(world), fs: { ...fs, dUnwatchable: false } }),
  }

  /// The first part of `rm -r` of the site root: every entry below it goes, each reported
  /// through the watch of the directory that held it, and the kernel drops `d`'s watch. The
  /// root itself still stands, empty, until `rootDelete`: the pipeline may receive and
  /// reconcile in between (FORMAL-10).
  action rootEmpty: bool = all {
    fs.rootPresent,
    fs.present != Set() or fs.dPresent,
    {
      val w0 = begin(world).changeServed(fs.present)
      val rootFiles = [Index, Css, Css2].select(p => fs.present.contains(p))
      val w1 = w0.emit(rootFiles.foldl([], (acc, p) => acc.append(note(KRemove, p))))
      val w2 = if (fs.present.contains(Sub)) w1.emit([note(KRemove, Sub)]) else w1
      val w3 = if (fs.dPresent) w2.emit([note(KDirChanged, DirD)]) else w2
      commit({
        ...w3,
        fs: { ...w3.fs, present: Set(), dPresent: false, dUnwatchable: false },
        src: { ...w3.src, watched: w3.src.watched.exclude(Set(DirD)) },
      })
    },
  }

  /// The end of `rm -r` of the site root: the emptied root is removed. Its own watch reports
  /// nothing: the server holds the root's descriptor (plan.md section 5.3), which keeps the
  /// directory's inode, and inotify reports `IN_DELETE_SELF` only when the inode goes, after
  /// the descriptor is closed (FORMAL-10). The watch of the root's parent reports it, as the
  /// removal of a directory entry: `DirectoryChanged` on the root (ADR 0073).
  action rootDelete: bool = all {
    fs.rootPresent,
    fs.present == Set(),
    not(fs.dPresent),
    {
      val w = begin(world).emitParent(note(KDirChanged, Root))
      commit({ ...w, fs: { ...w.fs, rootPresent: false }, src: { ...w.src, watched: Set() } })
    },
  }

  /// A new directory at the root's path. It is not the served root (the server holds the
  /// deleted one's descriptor); the watch of the root's parent reports its creation as
  /// `DirectoryChanged` on the root (ADR 0073), whose reconcile still finds the root removed.
  action rootRecreate: bool = all {
    not(fs.rootPresent),
    not(fs.rootRecreated),
    {
      val w = begin(world).emitParent(note(KDirChanged, Root))
      commit({ ...w, fs: { ...w.fs, rootRecreated: true } })
    },
  }

  /// The OS's own notification queue overflows (`dropping`) or drains again.
  action kernelQueue(dropping: bool): bool = all {
    src.osDropping != dropping,
    commit({ ...begin(world), src: { ...src, osDropping: dropping } }),
  }

  /// The native watcher reports a runtime error: queued as `Failed(RuntimeFailure)`.
  action runtimeFailure: bool = all {
    not(cancelled),
    commit({ ...begin(world), src: offer(src, MFailed(RuntimeFailure)) }),
  }

  /// The OS reports a queued change twice.
  action duplicate(i: int): bool = all {
    not(cancelled),
    i < src.queue.length(),
    match src.queue[i] {
      | MEvent(_) => true
      | MFailed(_) => false
    },
    {
      val q = src.queue
      val copy = q.slice(0, i + 1).append(q[i]).concat(q.slice(i + 1, q.length()))
      commit({
        ...begin(world),
        src: if (q.length() < CAP) { ...src, queue: copy } else { ...src, overflow: true },
      })
    },
  }

  /// The OS reports two changes out of order.
  action reorder(i: int): bool = all {
    not(cancelled),
    i + 1 < src.queue.length(),
    match src.queue[i] {
      | MEvent(_) => true
      | MFailed(_) => false
    },
    match src.queue[i + 1] {
      | MEvent(_) => true
      | MFailed(_) => false
    },
    {
      val q = src.queue
      commit({ ...begin(world), src: { ...src, queue: q.replaceAt(i, q[i + 1]).replaceAt(i + 1, q[i]) } })
    },
  }

  // ---------------------------------------------------------------------------------------
  // The pipeline task.

  /// The next batch is ready and taken: up to `k` queued changes; `capped` is whether a
  /// reconcile it causes passes the relisting cap. A due deadline goes first.
  action receive(k: int, capped: bool): bool = all {
    not(cancelled),
    not(due(deb, now)),
    {
      val taken = nextBatch(src, k)
      all {
        taken._1 != BNone,
        commit(receiveBatch({ ...begin(world), src: taken._2 }, taken._1, capped)),
      }
    },
  }

  /// The debounce deadline has passed: `settle` publishes one revision.
  action settle(capped: bool): bool = all {
    not(cancelled),
    due(deb, now),
    commit(settleAt(begin(world), capped)),
  }

  /// Time passes.
  action tick: bool = commit({ ...begin(world), now: now + 1 })

  /// Shutdown cancels the task, which closes the source; a pending change is dropped with
  /// the process (ADR 0049).
  action shutdown: bool = all {
    not(cancelled),
    commit({
      ...begin(world),
      cancelled: true,
      src: { ...src, queue: [], overflow: false, failure: false, watched: Set(), osDropping: false },
    }),
  }

  // ---------------------------------------------------------------------------------------
  // The step relation.

  /// The OS changing the site, its notifications misbehaving, or the watcher failing.
  action osStep = any {
    { nondet p = oneOf(SERVED) any { modify(p), createFile(p), deleteFile(p), saveAtomically(p) } },
    renameFile(Css, Css2),
    renameFile(Css2, Css),
    tempWrite,
    { nondet seen = oneOf(Set(true, false)) gitChange(seen) },
    dirDelete,
    rootEmpty,
    { nondet unwatchable = oneOf(Set(true, false)) dirCreate(unwatchable) },
    freeWatch,
    { nondet dropping = oneOf(Set(true, false)) kernelQueue(dropping) },
    runtimeFailure,
    { nondet i = oneOf(0.to(CAP - 1)) duplicate(i) },
    { nondet i = oneOf(0.to(CAP - 1)) reorder(i) },
  }

  /// The pipeline: a batch, the deadline, or time passing.
  action pipelineStep = any {
    { nondet k = oneOf(1.to(CAP)) nondet capped = oneOf(Set(false, true)) receive(k, capped) },
    { nondet capped = oneOf(Set(false, true)) settle(capped) },
    tick,
  }

  /// The actions that end most of a run's interest: deleting the (emptied) root, recreating
  /// it, shutting down.
  action endStep = any { rootDelete, rootRecreate, shutdown, tick }

  /// Everything that can happen. A roll picks the lane first (half the OS, nine in twenty
  /// the pipeline, one in twenty the end lane), so a 25-step trace publishes and fails
  /// often and rarely ends early; every lane can tick, so no step is ever disabled.
  action step = {
    nondet lane = oneOf(0.to(19))
    if (lane == 0) endStep
    else if (lane < 10) any { osStep, tick }
    else pipelineStep
  }

  // ---------------------------------------------------------------------------------------
  // Invariants (foundation.md section 6.1 item 2). Each is checked under `step` by
  // `quint run --invariant`.

  /// Only eligible changes trigger a reload: a revision names served files only; the
  /// debouncer is dirty only after a notification naming a served entry (ground truth, not
  /// `classify`) or an overflow; a batch of ineligible changes leaves the debouncer exactly
  /// as it was; and a clean debouncer holds no change.
  val onlyEligibleTriggerReload = and {
    last.fired implies last.firedPaths.subseteq(SERVED),
    deb.dirty implies ghost.eligibleCause,
    last.ineligibleOnly implies deb == last.debBefore,
    not(deb.dirty) implies deb == CLEAN,
  }

  /// The pipeline has settled: shutdown has not happened, every notification and loss the
  /// source holds has been received, and the debouncer is clean (its deadline fired).
  val settled = and {
    not(cancelled),
    src.queue == [],
    not(src.overflow),
    not(src.failure),
    not(deb.dirty),
  }

  /// No eligible change is lost after coalescing. The liveness property "every change the
  /// watcher saw is eventually published once the pipeline settles" is stated as a safety
  /// invariant over the settled condition: in every settled state, each served file changed
  /// since the revision that last covered it is one the watcher could not see (its
  /// directory's watch had failed, which `failureIsExplicit` covers). Every way out of an
  /// unsettled state either receives something or fires the deadline, so a trace that
  /// reaches a settled state has published every change it saw; a lost change shows up as a
  /// settled state with it still pending. A change covered by a full revision or by its own
  /// path counts as published: a reload fetches the current content. An unseen change is
  /// excused only while live reload is not claimed: once the watcher is `Watching` again, a
  /// settled state holds nothing pending, so the recovery must be followed by a revision that
  /// covers what changed while it was blind (ADR 0054; the old recovery, which published only
  /// the paths notified after it, violates this conjunct).
  val noLostEligibleChange = settled implies and {
    ghost.pending.subseteq(ghost.excused),
    health == HWatching implies ghost.pending == Set(),
  }

  /// Duplicates and bursts cannot cause a reload storm: exactly one revision per burst (a
  /// clean-to-dirty cycle of the debouncer, however many changes, duplicates and reorderings
  /// it absorbs), and a revision only after a quiet window since the latest change. The three
  /// `cycles` conjuncts hold by construction (`cycles` counts the debouncer's own clean-to-dirty
  /// transitions): model-internal checks that the replay gives teeth (doc/formal-models.md).
  val boundedReloadsPerBurst = and {
    revision <= ghost.cycles,
    ghost.cycles <= revision + 1,
    (ghost.cycles == revision + 1) == deb.dirty,
    last.fired implies last.firedQuiet,
  }

  /// A watcher failure goes through explicit error and recovery handling: live reload is
  /// claimed only with every eligible directory watched; a directory that cannot be watched
  /// is reported (degraded, or the start-up failure is queued); a registration failure is
  /// claimed only while a directory is outstanding (never a stale one, delivered after a
  /// reconcile that watched everything); every degradation and every recovery is announced
  /// exactly once (no silent change of health, no event storm), the one escalation to a root
  /// removal included.
  val failureIsExplicit = and {
    health == HWatching implies src.outstanding == Set(),
    (not(cancelled) and src.outstanding != Set()) implies
      (isDegraded(health) or registrationFailureQueued(src)),
    health == HDegraded(RegistrationFailed) implies src.outstanding != Set(),
    ghost.degradedEvents == ghost.recoveredEvents + ghost.escalations + (if (isDegraded(health)) 1 else 0),
    ghost.escalations <= 1,
    (isDegraded(health) and not(isDegraded(last.healthBefore))) implies
      last.events.indices().exists(i => match last.events[i] { | EvDegraded(_) => true | _ => false }),
    (isDegraded(last.healthBefore) and not(isDegraded(health))) implies last.events.indices().exists(i => last.events[i] == EvRecovered),
    last.events.length() <= 1,
  }

  /// An overflow means a full rescan and at least one reload: a rescan request keeps the
  /// debouncer dirty with a full set, and its revision is full and preceded by a full
  /// reconcile. `fullDone` is set from the flag that decides the reconcile, so its conjunct
  /// holds by construction and has teeth only in the replay (doc/formal-models.md).
  val overflowRescans = and {
    deb.rescan implies (deb.full and deb.dirty),
    last.firedRescan implies (last.firedFull and last.fullDone),
  }

  /// Live reload is never claimed for a root that is gone (plan.md section 8.4: the tool
  /// "must not claim live reload is active when notifications are unavailable"): once the
  /// source has delivered everything (the root's removal, which its parent's watch reports, or
  /// the loss that hid it), a deleted or replaced root has degraded the watcher with
  /// `RootRemoved`, and it stays so.
  val noLiveClaimWithoutRoot = and {
    (not(cancelled) and not(fs.rootPresent) and src.queue == [] and not(src.overflow) and not(src.failure))
      implies health == HDegraded(RootRemoved),
    fs.rootPresent implies health != HDegraded(RootRemoved),
  }

  /// Every invariant at once.
  val allInvariants = and {
    onlyEligibleTriggerReload,
    noLostEligibleChange,
    boundedReloadsPerBurst,
    failureIsExplicit,
    overflowRescans,
    noLiveClaimWithoutRoot,
  }

  // ---------------------------------------------------------------------------------------
  // Witnesses: states a run should reach (`quint run --witnesses ...`).

  val reachesPublish = revision > 0
  /// `noLostEligibleChange` is not vacuous: settled states after a revision.
  val reachesSettledAfterPublish = settled and revision > 0
  val reachesSettledWithUnseenChange = settled and ghost.pending != Set()
  val reachesOverflow = deb.rescan
  val reachesDegraded = isDegraded(health)
  val reachesRecovered = ghost.recoveredEvents > 0
  val reachesExcusedChange = ghost.excused != Set()
  /// A recovery caught up: live again, settled, after the full revision that followed it.
  val reachesRecoveredCaughtUp = settled and health == HWatching and ghost.recoveredEvents > 0 and last.firedFull
  /// The gap ADR 0054 closed: live again with a change made while unwatched still unpublished.
  /// No reachable state satisfies it now (`noLostEligibleChange` forbids it).
  val reachesRecoveredWithStaleChange = settled and health == HWatching and ghost.pending != Set()
  val reachesRootDeleted = not(fs.rootPresent)
  /// A root removal reported as such.
  val reachesRootRemoved = health == HDegraded(RootRemoved)
  /// A root removal reported after another degradation (the escalation).
  val reachesRootRemovalEscalated = ghost.escalations > 0
  /// Live after a full reconcile with no degradation since start-up: a start-up whose
  /// registration passed the relisting cap reaches it in one step (its `Overflow`).
  val reachesCappedStartupWatching = health == HWatching and ghost.degradedEvents == 0 and reconciledFully
}
