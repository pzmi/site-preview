//! Conformance negative controls (FORMAL-08, foundation.md section 6.3 step 7): each test
//! replaces one component with a deliberately faulty implementation behind the same driver
//! seam, replays a named committed trace, and asserts that the checker stops at the exact
//! step and action where the fault first shows, with the exact field that differs.
//!
//! A driver that silently stopped comparing (an `observe` returning a constant, a projection
//! that ignored a field) would let a faulty implementation pass, so these tests fail with it.
//! They run in the `unit-tests` stage with the rest of the workspace.
//!
//! Every fault lives here, in test code: the production crates are unchanged. The seams are
//! the drivers' own: a wrapper [`Driver`] around [`LifecycleDriver`], the reload driver's
//! [`ReloadDriver::with_router`] and [`ReloadDriver::with_publisher`], and the watcher
//! driver's [`WatcherDriver::with_coordinator`].
//!
//! The named traces are deterministic scripts (seed `0x5ce1`, ADR 0056 to ADR 0058); every
//! control replays its trace twice with fresh drivers and asserts the same divergence, so a
//! rerun with the same seed reproduces the same step.

use std::convert::Infallible;
use std::net::TcpListener;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use futures_util::StreamExt;
use futures_util::stream;
use preview_adapter_http::AppState;
use preview_application::{
    ChangeBatch, ChangeCoordinator, Received, RecordOutcome, ReloadState, SettleOutcome,
    SourceFailure,
};
use preview_conformance::drivers::lifecycle::{self, LifecycleDriver, LifecycleObs};
use preview_conformance::drivers::reload::{self, Kind, Publisher, ReloadDriver, ReloadObs};
use preview_conformance::drivers::watcher::{
    self, Coordinator, Publish, WatcherDriver, WatcherObs,
};
use preview_conformance::generate::formal_dir;
use preview_conformance::{Cause, Divergence, Driver, DriverError, Params, Trace, replay};
use preview_domain::{
    AssetKind, ChangeEvent, ChangeSet, Deadline, DebounceWindow, DomainEvent, SiteState, Tick,
    WatcherHealth, WatcherHealthRefusal,
};
use tempfile::TempDir;

/// The regression traces' recorded seed (the scripts are deterministic).
const REGRESSION_SEED: &str = "0x5ce1";

/// The event stream's path (plan.md section 7.2).
const EVENTS: &str = "/__site_preview/events";

/// The committed regression trace `formal/traces/regression/<name>.itf.json`.
fn regression(name: &str) -> Trace {
    let path = formal_dir()
        .join("traces/regression")
        .join(format!("{name}.itf.json"));
    Trace::load(&path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .with_seed(REGRESSION_SEED)
}

/// What a control caught: the divergence, the model's projected state at that step and the
/// faulty implementation's observation there (the replay stops at the divergence, so the
/// driver still holds it).
struct Caught<O> {
    divergence: Box<Divergence>,
    expected: O,
    actual: O,
}

/// Replays `trace` with a fresh driver from `make`, twice, and returns what the first run
/// caught. Panics if a run passes (the fault went unnoticed) or if the two runs disagree
/// (the catch does not reproduce).
fn catch<D, P>(trace: &Trace, make: impl Fn() -> D, project: P) -> Caught<D::Obs>
where
    D: Driver,
    P: Fn(&preview_conformance::State) -> Result<D::Obs, preview_conformance::ProjectionError>
        + Copy,
{
    let run = || {
        let mut driver = make();
        let divergence = replay(trace, &mut driver, project)
            .err()
            .unwrap_or_else(|| panic!("{}: the faulty implementation went unnoticed", trace.id));
        let actual = driver.observe();
        (divergence, actual)
    };
    let (divergence, actual) = run();
    let (again, _) = run();
    assert_eq!(
        divergence, again,
        "the same trace and seed reproduce the catch"
    );
    println!("{divergence}");
    let state = trace
        .states
        .iter()
        .find(|s| s.index == divergence.step)
        .expect("the diverging state is in the trace");
    let expected = project(state).expect("the model state projects");
    Caught {
        divergence,
        expected,
        actual,
    }
}

/// Asserts the catch point: the trace, the step, the action and a plain mismatch (not a
/// driver refusal or an unusable trace).
fn assert_caught_at<O>(caught: &Caught<O>, trace: &str, step: usize, action: &str) {
    let d = &caught.divergence;
    assert_eq!(
        (d.trace_id.as_str(), d.step, d.action.as_str(), &d.cause),
        (
            format!("{trace}.itf.json").as_str(),
            step,
            action,
            &Cause::Mismatch
        ),
        "{d}"
    );
    assert_eq!(d.seed.as_deref(), Some(REGRESSION_SEED));
}

// ------------------------------------------------------------------------------------------
// Lifecycle: `LeakyLifecycle` keeps the listener after a stop.
// ------------------------------------------------------------------------------------------

/// The real server, with a listener that outlives its stop: after each `stopServer` the
/// server's address is bound again and held, as a serve task that never closed its listener
/// would hold it. A wrapper rather than a change inside `Server::stop`: skipping its
/// `CloseListener` there leaves the serve task running and `JoinTasks` waiting for it
/// (ADR 0056).
struct LeakyLifecycle {
    inner: LifecycleDriver,
    leaked: Vec<TcpListener>,
}

impl Driver for LeakyLifecycle {
    type Obs = LifecycleObs;

    fn apply(&mut self, action: &str, params: &Params) -> Result<(), DriverError> {
        let address = self.inner.address();
        self.inner.apply(action, params)?;
        if action == "stopServer"
            && let Some(address) = address
            && !self
                .leaked
                .iter()
                .any(|l| l.local_addr().ok() == Some(address))
        {
            let listener = TcpListener::bind(address)
                .map_err(|e| DriverError::Refused(format!("cannot leak {address}: {e}")))?;
            self.leaked.push(listener);
        }
        Ok(())
    }

    fn observe(&self) -> LifecycleObs {
        self.inner.observe()
    }
}

fn site() -> TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><title>Conformance</title><h1>Hello</h1>",
    )
    .expect("an index page");
    dir
}

#[test]
fn should_catch_listener_leak_at_first_stop_when_lifecycle_keeps_listener() {
    // given
    let trace = regression("lifecycle-doubleStop");
    let root = site();
    let make = || LeakyLifecycle {
        inner: LifecycleDriver::new(root.path()),
        leaked: Vec::new(),
    };

    // when
    let caught = catch(&trace, make, lifecycle::project);

    // then: stopped, tasks joined, but the listener still answers
    assert_caught_at(&caught, "lifecycle-doubleStop", 3, "stopServer");
    assert!(!caught.expected.listener_owned);
    assert_eq!(
        caught.actual,
        LifecycleObs {
            listener_owned: true,
            ..caught.expected
        }
    );
}

// ------------------------------------------------------------------------------------------
// Reload: `GapIgnoringSse` sends a hot update across a coalesced gap.
// ------------------------------------------------------------------------------------------

/// An event stream that sends each value its subscription yields with that value's own
/// changes and `full`, ignoring whether revisions were skipped since the last event: across a
/// coalesced gap it offers a hot update built from the newest change set alone, violating
/// `noHotApplyAcrossGap`. Otherwise it speaks the real wire format (a `state` baseline, then
/// `reload` events), with no admission checks, keepalive or shutdown: the trace it replays
/// needs none.
struct GapIgnoringSse;

impl GapIgnoringSse {
    fn router(state: AppState) -> Router {
        Router::new()
            .route(EVENTS, get(Self::events))
            .with_state(state)
    }

    async fn events(State(state): State<AppState>) -> Response {
        let subscription = state.reload.subscribe();
        let baseline = Self::frame("state", &subscription.initial().state, None);
        let reloads = stream::unfold(subscription, |mut subscription| async move {
            let published = subscription.changed().await.ok()?;
            let frame = Self::frame("reload", &published.state, Some(&published.changes));
            Some((frame, subscription))
        });
        let frames = stream::once(std::future::ready(baseline))
            .chain(reloads)
            .map(Ok::<_, Infallible>);
        Response::new(Body::from_stream(frames))
    }

    /// One SSE frame: `event:`, `id:` (the revision) and the JSON `data:`.
    fn frame(name: &str, state: &SiteState, changes: Option<&ChangeSet>) -> String {
        let revision = state.revision().get();
        let mut data = serde_json::json!({
            "instance_id": state.instance_id().as_str(),
            "revision": revision,
        });
        if let Some(changes) = changes {
            data["changes"] = changes
                .changes()
                .iter()
                .map(|c| serde_json::json!({"path": c.url_path(), "kind": wire(c.kind())}))
                .collect();
            // The fault: the change set's own verdict, with no gap check.
            data["full"] = changes.is_full().into();
        }
        format!("event: {name}\nid: {revision}\ndata: {data}\n\n")
    }
}

const fn wire(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Css => "css",
        AssetKind::Js => "js",
        AssetKind::Html => "html",
        AssetKind::Asset => "asset",
        AssetKind::Other => "other",
    }
}

#[test]
fn should_catch_hot_update_across_gap_at_send_when_sse_ignores_gap() {
    // given: b1 holds revision 0; a css then a js revision coalesce before the send
    let trace = regression("reload-coalescedCssThenJsFull");

    // when
    let caught = catch(
        &trace,
        || ReloadDriver::with_router(GapIgnoringSse::router),
        reload::project,
    );

    // then: the model sends (1,2) full with no changes; the fault sends a hot js update
    assert_caught_at(&caught, "reload-coalescedCssThenJsFull", 6, "send");
    assert_eq!(caught.divergence.params, r#"b = "b1""#);
    let mut faulty = caught.expected.clone();
    let event = faulty
        .streams
        .get_mut("b1")
        .and_then(|events| events.last_mut())
        .expect("b1 received the reload");
    assert_eq!(
        (event.at.revision, event.full, event.kinds.is_empty()),
        (2, true, true)
    );
    event.full = false;
    event.kinds = [Kind::Js].into();
    assert_eq!(caught.actual, faulty);
}

// ------------------------------------------------------------------------------------------
// Reload: `NonMonotonicReloadState` announces revision n - 1 on its second publish.
// ------------------------------------------------------------------------------------------

/// A reload state whose second publish announces revision `n - 1` where it should announce
/// `n`: it repeats the revision already committed, with the new changes, so a reader cannot
/// tell the new value from the one it has. Built from [`ReloadState`]'s public API only (the
/// real one never goes back): a fresh channel for the same instance is advanced to `n - 1`
/// and replaces the driver's.
struct NonMonotonicReloadState;

impl NonMonotonicReloadState {
    fn publisher() -> Publisher {
        let mut publishes = 0_u32;
        Box::new(move |reload: &mut ReloadState, changes: ChangeSet| {
            publishes += 1;
            if publishes != 2 {
                return reload
                    .publish_next(changes)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
            }
            let current = reload.current();
            let repeated = ReloadState::new(current.instance_id().clone());
            for _ in 1..current.revision().get() {
                repeated
                    .publish_next(ChangeSet::new())
                    .map_err(|e| e.to_string())?;
            }
            repeated.publish_next(changes).map_err(|e| e.to_string())?;
            *reload = repeated;
            Ok(())
        })
    }
}

#[test]
fn should_catch_repeated_revision_at_second_publish_when_reload_state_is_not_monotonic() {
    // given
    let trace = regression("reload-coalescedCssThenJsFull");

    // when
    let caught = catch(
        &trace,
        || ReloadDriver::new().with_publisher(NonMonotonicReloadState::publisher()),
        reload::project,
    );

    // then: the model commits (1,2); the faulty state announces (1,1) again
    assert_caught_at(&caught, "reload-coalescedCssThenJsFull", 5, "publish");
    assert_eq!(caught.expected.committed.revision, 2);
    let mut faulty: ReloadObs = caught.expected.clone();
    faulty.committed.revision = 1;
    assert_eq!(caught.actual, faulty);
}

// ------------------------------------------------------------------------------------------
// Watcher: faulty coordinators around the real `ChangeCoordinator`.
// ------------------------------------------------------------------------------------------

/// Which fault a [`FaultyCoordinator`] has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatcherFault {
    /// `NoDuplicateSuppressionCoordinator`: publishes a revision for every event it receives,
    /// instead of one per settled burst (duplicates and bursts each cost a reload).
    NoDuplicateSuppression,
    /// A leading-edge debouncer: the first change of a quiet period publishes at once.
    LeadingEdge,
    /// An overflow batch is dropped: no change is recorded and no reconcile is asked for.
    DropOverflow,
    /// An overflow marks the revision full but `settle` never asks for the rescan.
    NoRescanFlag,
    /// A recovery reports readiness without marking the next revision a full reload.
    UnmarkedRecovery,
}

/// The real [`ChangeCoordinator`], built from the same window and reload state, with one
/// deliberate fault in the commands that [`WatcherFault`] names.
struct FaultyCoordinator {
    fault: WatcherFault,
    inner: ChangeCoordinator,
    reload: ReloadState,
    window: DebounceWindow,
    /// The change batches received since the debouncer was last clean, with their times.
    pending: Vec<(ChangeBatch<ChangeEvent>, Tick)>,
}

impl FaultyCoordinator {
    fn new(fault: WatcherFault, window: DebounceWindow, reload: ReloadState) -> Self {
        Self {
            fault,
            inner: ChangeCoordinator::new(window, reload.clone()),
            reload,
            window,
            pending: Vec::new(),
        }
    }

    /// A recovery that marks nothing. `ChangeCoordinator::watcher_ready` accepts start-up
    /// readiness only (FORMAL-10), so the fault is built from the public commands: a fresh
    /// coordinator on the same reload state reports start-up readiness, which marks
    /// nothing, and is given again the changes the degraded one held, at their times, so it
    /// holds exactly them; the recovery event is reported as the real one would.
    fn recover_unmarked(&mut self, dirs: usize) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        let mut fresh = ChangeCoordinator::new(self.window, self.reload.clone());
        fresh.watcher_ready(dirs)?;
        for (batch, at) in &self.pending {
            let _received = fresh.receive(batch.clone(), *at);
        }
        self.inner = fresh;
        Ok(vec![DomainEvent::WatcherRecovered])
    }

    /// Forgets the changes received once the debouncer is clean again.
    fn forget_settled(&mut self) {
        if self.inner.deadline().is_none() {
            self.pending.clear();
        }
    }
}

impl Coordinator for FaultyCoordinator {
    fn receive(&mut self, batch: ChangeBatch<ChangeEvent>, now: Tick) -> Received {
        match (self.fault, &batch) {
            (WatcherFault::NoDuplicateSuppression, ChangeBatch::Events(events)) => {
                for event in events {
                    let mut changes = ChangeSet::new();
                    changes.accumulate(event);
                    // Exhaustion is outside the traces; the comparison would show it anyway.
                    let _published = self.reload.publish_next(changes);
                }
                self.inner.receive(batch, now)
            }
            (WatcherFault::LeadingEdge, _) => {
                let quiet = self.inner.deadline().is_none();
                let received = self.inner.receive(batch, now);
                if quiet && let Some(deadline) = self.inner.deadline() {
                    let _settled = self.inner.settle(deadline.at());
                }
                received
            }
            (WatcherFault::DropOverflow, ChangeBatch::Overflow) => {
                Received::Recorded(RecordOutcome {
                    next_deadline: self.inner.deadline(),
                    needs_reconcile: false,
                    recorded: 0,
                    events: Vec::new(),
                })
            }
            (WatcherFault::UnmarkedRecovery, ChangeBatch::Events(_) | ChangeBatch::Overflow) => {
                self.pending.push((batch.clone(), now));
                self.inner.receive(batch, now)
            }
            _ => self.inner.receive(batch, now),
        }
    }

    fn settle(&mut self, now: Tick) -> SettleOutcome {
        let mut outcome = self.inner.settle(now);
        self.forget_settled();
        if self.fault == WatcherFault::NoRescanFlag
            && let Some(settled) = &mut outcome.settled
        {
            settled.rescan = false;
        }
        outcome
    }

    fn watcher_ready(&mut self, dirs: usize) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        self.inner.watcher_ready(dirs)
    }

    fn watcher_reconciled(
        &mut self,
        dirs: usize,
        now: Tick,
    ) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        if self.fault == WatcherFault::UnmarkedRecovery
            && matches!(self.inner.health(), WatcherHealth::Degraded { .. })
        {
            return self.recover_unmarked(dirs);
        }
        self.inner.watcher_reconciled(dirs, now)
    }

    fn watcher_failed(&mut self, failure: SourceFailure) -> Vec<DomainEvent> {
        self.inner.watcher_failed(failure)
    }

    fn health(&self) -> WatcherHealth {
        self.inner.health()
    }

    fn deadline(&self) -> Option<Deadline> {
        self.inner.deadline()
    }
}

/// Replays the named watcher trace `name` over a [`FaultyCoordinator`] with `fault`.
fn catch_watcher(
    name: &str,
    make: fn(DebounceWindow, ReloadState) -> FaultyCoordinator,
) -> Caught<WatcherObs> {
    let trace = regression(name);
    catch(
        &trace,
        || {
            WatcherDriver::with_coordinator(&trace, make)
                .unwrap_or_else(|err| panic!("{name}: environment: {err}"))
        },
        watcher::project,
    )
}

#[test]
fn should_catch_reload_per_event_at_first_receive_when_coordinator_suppresses_no_duplicate() {
    // given: two modifies and a duplicate of index.html, received as one batch
    let make = |w, r| FaultyCoordinator::new(WatcherFault::NoDuplicateSuppression, w, r);

    // when
    let caught = catch_watcher("watcher-burstOneReload", make);

    // then
    // then: the model has nothing published before the burst settles; the fault published
    // the batch's change at once
    assert_caught_at(&caught, "watcher-burstOneReload", 4, "receive");
    assert_eq!(
        (caught.expected.revision, &caught.expected.published),
        (0, &None)
    );
    let faulty = WatcherObs {
        revision: 1,
        published: Some(publish(false, false, &["/a.css"])),
        ..caught.expected.clone()
    };
    assert_eq!(caught.actual, faulty);
}

#[test]
fn should_catch_publish_at_receive_when_debouncer_fires_on_leading_edge() {
    // given
    let make = |w, r| FaultyCoordinator::new(WatcherFault::LeadingEdge, w, r);

    // when
    let caught = catch_watcher("watcher-atomicSave", make);

    // then
    // then: the model waits out the window; the fault published at the first change
    assert_caught_at(&caught, "watcher-atomicSave", 2, "receive");
    assert_eq!(caught.expected.deadline_ms, Some(150));
    let faulty = WatcherObs {
        revision: 1,
        dirty: false,
        deadline_ms: None,
        published: Some(publish(true, false, &["/index.html"])),
        ..caught.expected.clone()
    };
    assert_eq!(caught.actual, faulty);
}

#[test]
fn should_catch_lost_overflow_at_receive_when_coordinator_drops_overflow() {
    // given
    let make = |w, r| FaultyCoordinator::new(WatcherFault::DropOverflow, w, r);

    // when
    let caught = catch_watcher("watcher-overflowRescan", make);

    // then
    // then: the model is dirty and reconciles after the overflow; the fault noticed nothing
    assert_caught_at(&caught, "watcher-overflowRescan", 5, "receive");
    assert!(caught.expected.dirty && caught.expected.needs_reconcile);
    let faulty = WatcherObs {
        dirty: false,
        deadline_ms: None,
        needs_reconcile: false,
        ..caught.expected.clone()
    };
    assert_eq!(caught.actual, faulty);
}

#[test]
fn should_catch_missing_rescan_at_settle_when_overflow_sets_no_rescan_flag() {
    // given
    let make = |w, r| FaultyCoordinator::new(WatcherFault::NoRescanFlag, w, r);

    // when
    let caught = catch_watcher("watcher-overflowRescan", make);

    // then
    // then: the full revision is published, but without the rescan the model asks for
    assert_caught_at(&caught, "watcher-overflowRescan", 9, "settle");
    assert_eq!(
        caught.expected.published,
        Some(publish(true, true, &["/index.html"]))
    );
    let faulty = WatcherObs {
        published: Some(publish(true, false, &["/index.html"])),
        ..caught.expected.clone()
    };
    assert_eq!(caught.actual, faulty);
}

#[test]
fn should_catch_hot_revision_after_recovery_at_settle_when_recovery_is_unmarked() {
    // given
    let make = |w, r| FaultyCoordinator::new(WatcherFault::UnmarkedRecovery, w, r);

    // when
    let caught = catch_watcher("watcher-recoveryAfterWatchLimit", make);

    // then
    // then: the revision after the recovery is hot where the model reloads fully
    assert_caught_at(&caught, "watcher-recoveryAfterWatchLimit", 14, "settle");
    let mut faulty = caught.expected.clone();
    let published = faulty.published.as_mut().expect("the model publishes");
    assert!(published.full && !published.rescan);
    published.full = false;
    assert_eq!(caught.actual, faulty);
}

/// A published revision as the watcher driver observes it.
fn publish(full: bool, rescan: bool, paths: &[&str]) -> Publish {
    Publish {
        full,
        rescan,
        paths: paths.iter().map(|p| (*p).to_owned()).collect(),
    }
}
