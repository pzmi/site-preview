//! The server handle: start-up and shutdown driven by the domain [`Lifecycle`] machine
//! (plan.md section 9, foundation.md section 6.1 item 1, ADR 0051).
//!
//! [`Server::start`] checks the bind scope, opens the root and builds the runtime (the
//! preconditions: a refusal there leaves the lifecycle `Stopped`, since nothing was started),
//! then feeds the machine `Start` and performs each [`LifecycleEffect`] it returns, in order:
//! `BindListener` installs the stop-signal forwarder (when asked) and binds; with the listener
//! held, the reload state, the event-stream switch and the router are built; `StartWatcher`
//! starts the watcher and its pipeline task; `Degraded` warns that live reload is off;
//! `AnnounceReady` spawns the serve task and reports [`Ready`]; with `--open`, the browser is
//! started right after that, once the server accepts (CLI-07,
//! `should_invoke_opener_with_url_when_open_flag_set`). The outcome of each step is the
//! machine's next input (`Bound` or `BindFailed`, `WatcherReady` or `WatcherFailed`).
//!
//! A failed step releases what was acquired so far by dropping it: every resource is an owned
//! value (the root descriptor, the runtime with its signal forwarder, the listener) held by the
//! start-up frame, so an early return closes them all, and nothing outlives a refused start
//! (`should_start_no_watcher_and_never_report_ready_when_bind_fails`,
//! `should_refuse_with_its_variant_when_root_missing_or_reserved`). A watcher that cannot start, or panics
//! while starting, is `WatcherFailed`: the server serves, degraded, never exits
//! (`should_serve_degraded_when_watcher_unavailable`).
//!
//! [`Server::stop`] feeds `Stop` and performs the shutdown effects in the machine's order
//! (listener, event streams, watcher, tasks); a second call gets no effects and returns the
//! first call's outcome (`should_be_noop_when_stop_called_twice`). Dropping a running server
//! stops it.
//!
//! The serve task itself ends the event streams right after it closes the listener, in the
//! same step, and only then starts the drain's notice grace: a stream is never still open
//! because the stopping thread was scheduled late, so a tab alone never draws the notice
//! (`should_not_announce_drain_when_stopping_thread_runs_late_with_only_a_tab_open`). The
//! `CloseStreams` effect then finds them ended (the switch is idempotent); the order is still
//! listener, streams, watcher, tasks. A second stop signal while the tasks are joined stops at
//! once, as one during the drain does
//! (`should_stop_at_once_when_second_stop_arrives_while_joining`).

use core::future::{Future, poll_fn};
use core::pin::{Pin, pin};
use core::task::Poll;
use std::io;
use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::time::Duration;

use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Injection, Shutdown, StreamLimit,
    WarningSink, build_router,
};
use preview_application::{ChangeCoordinator, ReloadState};
use preview_domain::{
    BoundAddr, HostPolicy, Lifecycle, LifecycleEffect, LifecycleInput, LifecycleState,
};
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};

use crate::bootstrap::{OpenedRoot, WatchStart, open_root};
use crate::config::{ServeConfig, ServerLimits};
use crate::connections::{StopSignal, Stopped, serve_connections};
use crate::launcher::{SystemOpener, UrlOpener, launch};
use crate::server::{
    Ready, StartupError, StartupReporter, WatchStatus, instance_id, reachable,
    report_watch_problems, unavailable_warning,
};
use crate::shutdown::stop_signals;
use crate::watch_pipeline::{TokioClock, WatchControl, WatchPipeline};

/// Whether the server forwards SIGINT and SIGTERM to its own stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signals {
    /// Install the handlers before the listener is bound (the binary).
    Forward,
    /// Leave the process's signal handling alone (a server embedded in a test).
    Ignore,
}

/// How long a drain may run before the shutdown says it is waiting for connections: longer
/// than ending event streams and closing idle connections ever takes, far shorter than the
/// drain bound. Measured by the drain itself from the moment the serve task has closed the
/// listener and ended the event streams, never from when the caller's thread gets round to
/// the rest of the shutdown.
const DRAIN_NOTICE_GRACE: Duration = Duration::from_millis(500);

/// Why nothing could be watched when the watcher panicked while starting.
const WATCHER_PANICKED: &str = "the file watcher failed while starting";

/// What the serve task hands back: how the drain ended, and the stop channel, so a second
/// signal while the other tasks are joined still stops at once.
type ServeOutcome = (Stopped, mpsc::UnboundedReceiver<StopSignal>);

/// A running server, its lifecycle and the tasks it owns.
pub struct Server {
    lifecycle: Lifecycle,
    /// `None` once shut down.
    runtime: Option<Runtime>,
    reporter: Box<dyn StartupReporter>,
    limits: ServerLimits,
    local: SocketAddr,
    url: String,
    /// Asks the serve task to stop; the signal forwarder holds a clone.
    stops: mpsc::UnboundedSender<StopSignal>,
    /// Answers once the listener is closed (with the number of connections then open); `None`
    /// once answered.
    listener_closed: Option<oneshot::Receiver<usize>>,
    /// When the drain bound ends, from the moment the listener closed.
    drain_ends: Option<Instant>,
    /// Answers once the drain has run [`DRAIN_NOTICE_GRACE`] with a connection still open; its
    /// sender is dropped unanswered when the drain ends sooner (or the serve task fails).
    waiting: Option<oneshot::Receiver<()>>,
    /// The event-stream switch; the serve task holds the other handle and fires it as it
    /// closes the listener.
    streams: Arc<Shutdown>,
    stop_watching: Option<oneshot::Sender<()>>,
    serve: Option<JoinHandle<ServeOutcome>>,
    pipeline: Option<JoinHandle<()>>,
    /// The task forwarding SIGINT and SIGTERM (the binary only): aborted once the tasks are
    /// joined, so nothing the server spawned outlives its stop.
    forwarder: Option<JoinHandle<()>>,
    outcome: Option<Stopped>,
}

impl Server {
    /// Starts serving `config.root`, without touching the process's signal handling: the
    /// caller stops it with [`Server::stop`] (or by dropping it).
    ///
    /// # Errors
    ///
    /// [`StartupError`] as documented on its variants; nothing stays allocated.
    pub fn start(
        config: &ServeConfig,
        reporter: Box<dyn StartupReporter>,
        warnings: Arc<dyn WarningSink>,
    ) -> Result<Self, StartupError> {
        Self::start_with(
            config,
            reporter,
            warnings,
            crate::bootstrap::start_watcher,
            Signals::Ignore,
            &SystemOpener,
        )
    }

    /// [`Server::start`] with the watcher factory and the signal choice injected.
    pub(crate) fn start_with<W: WatchControl>(
        config: &ServeConfig,
        reporter: Box<dyn StartupReporter>,
        warnings: Arc<dyn WarningSink>,
        watcher: impl FnOnce(&OpenedRoot) -> (W, WatchStart),
        signals: Signals,
        opener: &dyn UrlOpener,
    ) -> Result<Self, StartupError> {
        config.bind_scope().map_err(StartupError::RemoteBind)?;
        let root = open_root(&config.root).map_err(StartupError::from)?;
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(StartupError::Runtime)?;
        let (stops, stop_requests) = mpsc::unbounded_channel();
        let mut startup = Startup {
            config,
            runtime,
            root,
            warnings: Arc::clone(&warnings),
            reporter,
            signals,
            watcher: Some(watcher),
            stops,
            stop_requests: Some(stop_requests),
            listener: None,
            error: None,
            wiring: None,
            watch: None,
            pipeline: None,
            serve: None,
            forwarder: None,
            listener_closed: None,
            waiting: None,
            local_url: None,
        };
        let mut lifecycle = Lifecycle::new();
        let mut input = Some(LifecycleInput::Start);
        while let Some(current) = input.take() {
            let transition = lifecycle
                .apply(current)
                .expect("start-up feeds only the inputs its effects lead to (DOM-09)");
            for &effect in transition.effects {
                if let Some(next) = startup.perform(effect)? {
                    input = Some(next);
                }
            }
        }
        let server = startup.into_server(lifecycle)?;
        if config.open {
            launch(opener, server.url(), warnings.as_ref());
        }
        Ok(server)
    }

    /// The lifecycle state, for conformance and tests.
    #[must_use]
    pub const fn state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// The address the listener is bound to (the operating system's choice for port 0).
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// The base URL reported as ready.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Stops the server: closes the listener, ends the event streams, stops the watcher, then
    /// drains the connections and joins the owned tasks within the drain bound (plan.md
    /// section 9). Idempotent: a later call does nothing and returns the same outcome.
    pub fn stop(&mut self) -> Stopped {
        let transition = self
            .lifecycle
            .apply(LifecycleInput::Stop)
            .expect("Stop is accepted in every state (DOM-09)");
        for &effect in transition.effects {
            self.perform(effect);
        }
        if self.lifecycle.state() == (LifecycleState::Stopping { awaiting: None }) {
            self.lifecycle
                .apply(LifecycleInput::Stopped)
                .expect("a drained stop is accepted (DOM-09)");
        }
        self.outcome.unwrap_or(Stopped::Drained)
    }

    fn perform(&mut self, effect: LifecycleEffect) {
        match effect {
            LifecycleEffect::CloseListener => self.close_listener(),
            // Normally already fired by the serve task as it closed the listener; idempotent,
            // and still needed when the serve task ended without closing it (a panic).
            LifecycleEffect::CloseStreams => self.streams.trigger(),
            LifecycleEffect::StopWatcher => {
                // The pipeline may already have ended (a panic); nothing is left to stop.
                if let Some(stop) = self.stop_watching.take() {
                    let _ = stop.send(());
                }
            }
            LifecycleEffect::JoinTasks => self.join_tasks(),
            // Start-up effects: a running server never receives them.
            LifecycleEffect::BindListener
            | LifecycleEffect::StartWatcher
            | LifecycleEffect::AnnounceReady
            | LifecycleEffect::Degraded
            | LifecycleEffect::AbortStartup => {}
        }
    }

    /// Asks the serve task to close the listener, unless a stop signal already did, and waits
    /// until it has: from then on a connection is refused.
    fn close_listener(&mut self) {
        if self.listener_closed.is_some() {
            // A caller's stop asks as SIGTERM does; the serve task treats it as the first stop.
            let _ = self.stops.send(StopSignal::Terminate);
            self.await_listener_closed();
        }
    }

    /// Waits until the serve task has closed the listener (at the first stop signal or a
    /// caller's stop) and starts the drain bound from then.
    pub(crate) fn await_listener_closed(&mut self) {
        let (Some(closed), Some(runtime)) = (self.listener_closed.take(), self.runtime.as_ref())
        else {
            return;
        };
        // An error means the serve task ended without closing it (a panic): it is gone too. The
        // count is not used: it includes event streams and idle keep-alive connections, which
        // the shutdown ends at once, so only the drain's grace knows whether it waits for
        // anything.
        let _open = runtime.block_on(closed);
        self.drain_ends = Some(Instant::now() + self.limits.drain_timeout);
    }

    /// Waits for the drain (the serve task), then for the pipeline within what is left of the
    /// drain bound, and shuts the runtime down without waiting for anything else: the drain
    /// bound is the promise (ADR 0045). A second stop signal while the pipeline is joined stops
    /// at once ([`Stopped::Forced`]), as one during the drain does.
    fn join_tasks(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        // A serve task that panicked is re-raised, so a crash never exits 0, unless this
        // stop runs while the thread already unwinds (from `Drop`), where it would abort.
        let (mut stopped, mut stops) = match self.serve.take() {
            None => (Stopped::Drained, None),
            Some(mut serve) => {
                // The notice is for a drain that is really waiting: a browser tab's event
                // stream and idle connections end within the grace, and a notice about
                // waiting there would be false (`should_not_announce_drain_when_only_a_tab_is_open`).
                // The drain decides on its own clock, so a thread scheduled late (a loaded
                // machine) still tells of a drain that waited
                // (`should_announce_drain_when_joined_after_a_stalled_drain_ended`).
                if let Some(waiting) = self.waiting.take()
                    && runtime.block_on(waiting).is_ok()
                {
                    self.reporter.draining(self.limits.drain_timeout);
                }
                match runtime.block_on(&mut serve) {
                    Ok((stopped, stops)) => (stopped, Some(stops)),
                    Err(failed) if failed.is_panic() && !std::thread::panicking() => {
                        resume_unwind(failed.into_panic())
                    }
                    Err(_) => (Stopped::DrainTimedOut, None),
                }
            }
        };
        // Cancelled at the listener's close, the pipeline has normally ended long before the
        // drain does; after a second signal it is not waited for at all.
        let ends = match (stopped, self.drain_ends) {
            (Stopped::Forced(_), _) | (_, None) => Instant::now(),
            (_, Some(ends)) => ends,
        };
        if let Some(mut pipeline) = self.pipeline.take() {
            let second = runtime.block_on(join_pipeline(&mut pipeline, stops.as_mut(), ends));
            // Ended, past the bound, or cut short: an abort of an ended task does nothing.
            pipeline.abort();
            if let (Some(signal), false) = (second, matches!(stopped, Stopped::Forced(_))) {
                stopped = Stopped::Forced(signal);
            }
        }
        if let Some(forwarder) = self.forwarder.take() {
            forwarder.abort();
        }
        runtime.shutdown_background();
        self.outcome = Some(stopped);
    }
}

/// Waits for `pipeline` until `ends`; returns the stop signal that arrived meanwhile, if one
/// did first (README "Stopping": pressing Ctrl-C again stops at once).
async fn join_pipeline(
    pipeline: &mut JoinHandle<()>,
    mut stops: Option<&mut mpsc::UnboundedReceiver<StopSignal>>,
    ends: Instant,
) -> Option<StopSignal> {
    let mut deadline = pin!(sleep_until(ends));
    poll_fn(|cx| {
        // A closed channel cannot deliver a second stop; it is not one.
        if let Some(stops) = stops.as_deref_mut()
            && let Poll::Ready(Some(signal)) = stops.poll_recv(cx)
        {
            return Poll::Ready(Some(signal));
        }
        if Pin::new(&mut *pipeline).poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        deadline.as_mut().poll(cx).map(|()| None)
    })
    .await
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.runtime.is_some() {
            let _ = self.stop();
        }
    }
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server")
            .field("state", &self.lifecycle.state())
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

/// What the router and the pipeline share, built once the listener is held.
struct Wiring {
    reload: ReloadState,
    streams: Arc<Shutdown>,
    router: axum::Router,
}

/// Everything start-up has acquired so far. Dropped on a refusal, which releases it all.
struct Startup<'c, F> {
    config: &'c ServeConfig,
    runtime: Runtime,
    root: OpenedRoot,
    warnings: Arc<dyn WarningSink>,
    reporter: Box<dyn StartupReporter>,
    signals: Signals,
    watcher: Option<F>,
    stops: mpsc::UnboundedSender<StopSignal>,
    stop_requests: Option<mpsc::UnboundedReceiver<StopSignal>>,
    listener: Option<(TcpListener, SocketAddr)>,
    error: Option<StartupError>,
    wiring: Option<Wiring>,
    watch: Option<WatchStart>,
    pipeline: Option<(JoinHandle<()>, oneshot::Sender<()>)>,
    serve: Option<JoinHandle<ServeOutcome>>,
    forwarder: Option<JoinHandle<()>>,
    listener_closed: Option<oneshot::Receiver<usize>>,
    waiting: Option<oneshot::Receiver<()>>,
    /// The bound address and the URL reported as ready.
    local_url: Option<(SocketAddr, String)>,
}

impl<W: WatchControl, F: FnOnce(&OpenedRoot) -> (W, WatchStart)> Startup<'_, F> {
    /// Performs one start-up effect; returns the machine's next input when the effect is a
    /// step with an outcome.
    fn perform(&mut self, effect: LifecycleEffect) -> Result<Option<LifecycleInput>, StartupError> {
        match effect {
            LifecycleEffect::BindListener => Ok(Some(self.bind())),
            LifecycleEffect::StartWatcher => Ok(Some(self.start_watcher())),
            LifecycleEffect::Degraded => {
                if let Some(reason) = self.watch.as_ref().and_then(|w| w.unavailable.as_ref()) {
                    self.warnings
                        .warn("watch-unavailable", &unavailable_warning(reason));
                }
                Ok(None)
            }
            LifecycleEffect::AnnounceReady => {
                self.announce();
                Ok(None)
            }
            LifecycleEffect::AbortStartup => Err(self
                .error
                .take()
                .unwrap_or_else(|| StartupError::Runtime(io::Error::other("start-up aborted")))),
            // Shutdown effects: start-up never receives them.
            LifecycleEffect::CloseListener
            | LifecycleEffect::CloseStreams
            | LifecycleEffect::StopWatcher
            | LifecycleEffect::JoinTasks => Ok(None),
        }
    }

    /// Installs the signal forwarder (before anything can report readiness, so a signal sent
    /// as soon as the URL is known is never the default action), binds, then builds the
    /// reload state, the event-stream switch and the router (plan.md section 9 order).
    fn bind(&mut self) -> LifecycleInput {
        let addr = SocketAddr::new(self.config.host, self.config.port);
        let stops = self.stops.clone();
        let signals = self.signals;
        let bound = self.runtime.block_on(async move {
            let forwarder = if signals == Signals::Forward {
                Some(stop_signals(stops).map_err(StartupError::Signal)?)
            } else {
                None
            };
            let listener = TcpListener::bind(addr)
                .await
                .map_err(|source| StartupError::Bind { addr, source })?;
            let local = listener
                .local_addr()
                .map_err(|source| StartupError::Bind { addr, source })?;
            Ok((listener, local, forwarder))
        });
        match bound {
            Ok((listener, local, forwarder)) => {
                self.forwarder = forwarder;
                self.wiring = Some(self.wire(local));
                self.listener = Some((listener, local));
                LifecycleInput::Bound
            }
            Err(error) => {
                self.error = Some(error);
                LifecycleInput::BindFailed
            }
        }
    }

    fn wire(&self, local: SocketAddr) -> Wiring {
        let reload = ReloadState::new(instance_id());
        let streams = Arc::new(Shutdown::new());
        let state = AppState {
            content: Arc::clone(&self.root.content),
            reload: reload.clone(),
            // From the address actually bound: with `--port 0` the request said 0, and a
            // policy over port 0 would refuse every Host.
            host: HostPolicy::new(
                BoundAddr::new(local.ip(), local.port()),
                self.config.allow_remote,
            ),
            shutdown: streams.signal(),
            keep_alive: DEFAULT_KEEP_ALIVE,
            streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
            injection: Injection::new(Arc::clone(&self.warnings)),
        };
        Wiring {
            reload,
            streams,
            router: build_router(state),
        }
    }

    /// Starts the watcher and, when it watches anything, its pipeline task. A watcher that
    /// cannot watch anything, or panics while starting, is closed and reported as failed:
    /// degraded mode, never a start-up failure.
    fn start_watcher(&mut self) -> LifecycleInput {
        let Some(factory) = self.watcher.take() else {
            return LifecycleInput::WatcherFailed;
        };
        // The walk and the registrations block: they run here, on the caller's thread with the
        // runtime entered, never on an async worker (plan.md section 9, ADR 0051).
        let started = {
            let _context = self.runtime.enter();
            catch_unwind(AssertUnwindSafe(|| factory(&self.root)))
        };
        let (mut source, watch) = match started {
            Ok(started) => started,
            // The payload is not shown: it may name an absolute path.
            Err(_panic) => {
                self.watch = Some(WatchStart {
                    dirs: 0,
                    problems: Vec::new(),
                    unavailable: Some(WATCHER_PANICKED.to_owned()),
                });
                return LifecycleInput::WatcherFailed;
            }
        };
        report_watch_problems(&watch, self.warnings.as_ref());
        let failed = watch.unavailable.is_some();
        if failed {
            source.close();
        } else if let Some(wiring) = &self.wiring {
            let mut coordinator =
                ChangeCoordinator::new(self.config.debounce, wiring.reload.clone());
            if watch.is_complete() && watch.dirs > 0 {
                // A fresh coordinator accepts any non-zero count; nothing to report.
                let _ = coordinator.watcher_ready(watch.dirs);
            }
            let (stop, stopped) = oneshot::channel();
            let pipeline = WatchPipeline::new(
                source,
                coordinator,
                TokioClock::start(),
                Arc::clone(&self.warnings),
                self.reporter.watcher_events(),
            );
            let task = {
                let _context = self.runtime.enter();
                tokio::spawn(pipeline.run(stopped))
            };
            self.pipeline = Some((task, stop));
        }
        self.watch = Some(watch);
        if failed {
            LifecycleInput::WatcherFailed
        } else {
            LifecycleInput::WatcherReady
        }
    }

    /// Spawns the serve task, then reports [`Ready`]: the first client reading the URL finds
    /// the server accepting.
    fn announce(&mut self) {
        let (Some((listener, local)), Some(wiring), Some(mut stop_requests)) = (
            self.listener.take(),
            self.wiring.as_ref(),
            self.stop_requests.take(),
        ) else {
            return;
        };
        let (closed, listener_closed) = oneshot::channel();
        let (still_open, waiting) = oneshot::channel();
        let router = wiring.router.clone();
        let streams = Arc::clone(&wiring.streams);
        let limits = self.config.limits;
        self.serve = Some(self.runtime.spawn(async move {
            let stopped = serve_connections(
                listener,
                router,
                limits,
                &mut stop_requests,
                move |open| {
                    // The streams end here, with the listener closed, before the grace starts:
                    // never later, on whichever thread gets round to `CloseStreams`.
                    streams.trigger();
                    let _ = closed.send(open);
                },
                DRAIN_NOTICE_GRACE,
                move || {
                    let _ = still_open.send(());
                },
            )
            .await;
            (stopped, stop_requests)
        }));
        self.listener_closed = Some(listener_closed);
        self.waiting = Some(waiting);
        let url = format!("http://{}/", reachable(local));
        let unknown = WatchStart {
            dirs: 0,
            problems: Vec::new(),
            unavailable: Some(WATCHER_PANICKED.to_owned()),
        };
        let watch = self.watch.as_ref().unwrap_or(&unknown);
        self.reporter.ready(&Ready {
            root: &self.root.display_path,
            url: &url,
            watching: WatchStatus::of(watch),
        });
        self.local_url = Some((local, url));
    }

    /// The running server, once the machine has reached `Serving`.
    fn into_server(self, lifecycle: Lifecycle) -> Result<Server, StartupError> {
        let (Some((local, url)), Some(wiring), Some(serve), Some(listener_closed)) = (
            self.local_url,
            self.wiring,
            self.serve,
            self.listener_closed,
        ) else {
            return Err(StartupError::Runtime(io::Error::other(
                "start-up ended before the server was ready",
            )));
        };
        let (pipeline, stop_watching) = self.pipeline.unzip();
        Ok(Server {
            lifecycle,
            runtime: Some(self.runtime),
            reporter: self.reporter,
            limits: self.config.limits,
            local,
            url,
            stops: self.stops,
            listener_closed: Some(listener_closed),
            drain_ends: None,
            waiting: self.waiting,
            streams: wiring.streams,
            stop_watching,
            serve: Some(serve),
            pipeline,
            forwarder: self.forwarder,
            outcome: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use core::future::{Future, pending, ready};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{TcpListener as StdListener, TcpStream};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use preview_application::{ChangeBatch, ChangeSource, SourceFailure};
    use preview_domain::ChangeEvent;
    use preview_testkit::poll::wait_until;
    use tempfile::TempDir;

    use super::*;
    use crate::server::{DegradedReason, WatcherEvents, WatcherState};
    use crate::watch_pipeline::{ReconcileScope, Reconciled};

    /// How long a test waits for a reply that should come at once.
    const DEADLINE: Duration = Duration::from_secs(20);

    fn site() -> TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            dir.path().join("index.html"),
            "<!doctype html><title>Lifecycle</title><h1>Hello</h1>",
        )
        .expect("an index page");
        dir
    }

    #[derive(Default)]
    struct Warnings(Mutex<Vec<(String, String)>>);

    impl Warnings {
        fn recorded(&self) -> Vec<(String, String)> {
            self.0.lock().expect("unpoisoned").clone()
        }
    }

    impl WarningSink for Warnings {
        fn warn(&self, code: &str, message: &str) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push((code.to_owned(), message.to_owned()));
        }
    }

    /// The watcher state changes the pipeline announced.
    #[derive(Clone, Default)]
    struct States(Arc<Mutex<Vec<WatcherState>>>);

    impl WatcherEvents for States {
        fn changed(&self, state: WatcherState) {
            self.0.lock().expect("unpoisoned").push(state);
        }
    }

    /// What the server reported: each `ready` as its `Watching` status, and each drain notice.
    #[derive(Clone, Default)]
    struct Reports(Arc<Mutex<Vec<String>>>, States);

    impl Reports {
        fn recorded(&self) -> Vec<String> {
            self.0.lock().expect("unpoisoned").clone()
        }

        fn states(&self) -> Vec<WatcherState> {
            (self.1).0.lock().expect("unpoisoned").clone()
        }
    }

    impl StartupReporter for Reports {
        fn ready(&mut self, ready: &Ready<'_>) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push(format!("ready {:?}", ready.watching));
        }

        fn draining(&mut self, bound: Duration) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push(format!("draining {bound:?}"));
        }

        fn watcher_events(&self) -> Arc<dyn WatcherEvents> {
            Arc::new(self.1.clone())
        }
    }

    /// A change source standing in for the native watcher: it never delivers a change, and
    /// records whether it was closed.
    struct FakeSource {
        live: bool,
        closed: Arc<AtomicBool>,
    }

    impl ChangeSource for FakeSource {
        type Event = ChangeEvent;

        fn next(&mut self) -> impl Future<Output = ChangeBatch<ChangeEvent>> + Send {
            let live = self.live;
            async move {
                if live {
                    pending::<()>().await;
                }
                ready(ChangeBatch::Closed).await
            }
        }
    }

    impl WatchControl for FakeSource {
        fn reconcile(&mut self, _scope: ReconcileScope) -> Reconciled {
            Reconciled::default()
        }

        fn failure_detail(&self) -> Option<String> {
            None
        }

        fn close(&mut self) {
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    /// A watcher whose changes the test scripts.
    struct Scripted(preview_testkit::ScriptedChangeSource);

    impl ChangeSource for Scripted {
        type Event = ChangeEvent;

        fn next(&mut self) -> impl Future<Output = ChangeBatch<ChangeEvent>> + Send {
            self.0.next()
        }
    }

    impl WatchControl for Scripted {
        fn reconcile(&mut self, _scope: ReconcileScope) -> Reconciled {
            Reconciled::default()
        }

        fn failure_detail(&self) -> Option<String> {
            None
        }

        fn close(&mut self) {}
    }

    /// A factory whose watcher watches `dirs` directories, or reports `unavailable`.
    fn watcher(
        unavailable: Option<&str>,
        closed: &Arc<AtomicBool>,
    ) -> impl FnOnce(&OpenedRoot) -> (FakeSource, WatchStart) + use<> {
        let closed = Arc::clone(closed);
        let unavailable = unavailable.map(str::to_owned);
        move |_root| {
            let start = WatchStart {
                dirs: usize::from(unavailable.is_none()),
                problems: Vec::new(),
                unavailable: unavailable.clone(),
            };
            let live = unavailable.is_none();
            (FakeSource { live, closed }, start)
        }
    }

    /// A boxed watcher factory, so cases with different factories share one array.
    type Factory = Box<dyn FnOnce(&OpenedRoot) -> (FakeSource, WatchStart)>;

    fn panicking(_root: &OpenedRoot) -> (FakeSource, WatchStart) {
        panic!("an injected panic while the watcher starts");
    }

    fn start<W: WatchControl>(
        root: &TempDir,
        factory: impl FnOnce(&OpenedRoot) -> (W, WatchStart),
    ) -> (Result<Server, StartupError>, Reports, Arc<Warnings>) {
        let reports = Reports::default();
        let warnings = Arc::new(Warnings::default());
        let config = ServeConfig::new(root.path().to_path_buf());
        let server = Server::start_with(
            &config,
            Box::new(reports.clone()),
            Arc::clone(&warnings) as Arc<dyn WarningSink>,
            factory,
            Signals::Ignore,
            &SystemOpener,
        );
        (server, reports, warnings)
    }

    /// Records every URL it is asked to open; fails when `fail` is set.
    #[derive(Default)]
    struct Recording {
        urls: Mutex<Vec<String>>,
        fail: bool,
    }

    impl UrlOpener for Recording {
        fn open(&self, url: &str) -> io::Result<()> {
            self.urls.lock().expect("unpoisoned").push(url.to_owned());
            if self.fail {
                Err(io::Error::other("an injected launcher failure"))
            } else {
                Ok(())
            }
        }
    }

    /// Starts a server with the quiet fake watcher and `opener`, `--open` per `open`.
    fn start_opening(root: &TempDir, open: bool, opener: &Recording) -> (Server, Arc<Warnings>) {
        let warnings = Arc::new(Warnings::default());
        let mut config = ServeConfig::new(root.path().to_path_buf());
        config.open = open;
        let closed = Arc::new(AtomicBool::new(false));
        let server = Server::start_with(
            &config,
            Box::new(Reports::default()),
            Arc::clone(&warnings) as Arc<dyn WarningSink>,
            watcher(None, &closed),
            Signals::Ignore,
            opener,
        )
        .expect("the server starts");
        (server, warnings)
    }

    #[test]
    fn should_invoke_opener_with_url_when_open_flag_set() {
        // given
        let root = site();
        let opener = Recording::default();

        // when
        let (server, warnings) = start_opening(&root, true, &opener);

        // then
        let expected = format!("http://127.0.0.1:{}/", server.local_addr().port());
        assert_eq!(*opener.urls.lock().expect("unpoisoned"), [expected]);
        assert_eq!(
            server.url(),
            format!("http://127.0.0.1:{}/", server.local_addr().port())
        );
        assert!(warnings.recorded().is_empty());
    }

    #[test]
    fn should_not_invoke_opener_when_flag_absent() {
        // given
        let root = site();
        let opener = Recording::default();

        // when
        let (_server, _warnings) = start_opening(&root, false, &opener);

        // then
        assert!(opener.urls.lock().expect("unpoisoned").is_empty());
    }

    #[test]
    fn should_warn_and_keep_serving_when_opener_fails() {
        // given
        let root = site();
        let opener = Recording {
            fail: true,
            ..Recording::default()
        };

        // when
        let (mut server, warnings) = start_opening(&root, true, &opener);

        // then
        let recorded = warnings.recorded();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].0, "open-failed");
        assert!(
            recorded[0].1.contains("an injected launcher failure"),
            "{recorded:?}"
        );
        assert_eq!(get_status(server.local_addr()), 200);
        assert_eq!(server.stop(), Stopped::Drained);
    }

    /// The status code of `GET /` on `addr`.
    fn get_status(addr: SocketAddr) -> u16 {
        let mut stream = TcpStream::connect(addr).expect("the server accepts");
        stream
            .set_read_timeout(Some(DEADLINE))
            .expect("a read timeout");
        let head = format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(head.as_bytes())
            .expect("the request is sent");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("a reply");
        reply
            .get(9..12)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn should_serve_degraded_when_watcher_unavailable() {
        // given
        let root = site();
        let closed = Arc::new(AtomicBool::new(false));

        // when
        let (server, reports, warnings) =
            start(&root, watcher(Some("an injected watcher failure"), &closed));

        // then
        let mut server = server.expect("a failing watcher is not a start-up failure");
        assert_eq!(server.state(), LifecycleState::Serving { watcher: false });
        assert_eq!(get_status(server.local_addr()), 200);
        assert!(closed.load(Ordering::SeqCst), "the failed source is closed");
        assert_eq!(
            reports.recorded(),
            [r#"ready Unavailable { reason: "an injected watcher failure" }"#]
        );
        assert_eq!(
            warnings.recorded(),
            [(
                "watch-unavailable".to_owned(),
                "file watching is unavailable: an injected watcher failure; live reload is \
                 disabled, pages are still served"
                    .to_owned()
            )]
        );
        assert_eq!(server.stop(), Stopped::Drained);
        assert_eq!(server.state(), LifecycleState::Stopped);
    }

    #[test]
    fn should_emit_watcher_degraded_event_when_watcher_fails_at_runtime() {
        // given: a running server whose watcher worked at start-up
        let root = site();
        let (source, handle) = preview_testkit::ScriptedChangeSource::new();
        let (server, reports, _warnings) = start(&root, move |_root| {
            let start = WatchStart {
                dirs: 1,
                problems: Vec::new(),
                unavailable: None,
            };
            (Scripted(source), start)
        });
        let mut server = server.expect("a server");

        // when: the operating system's watcher fails, twice
        handle.fail(SourceFailure::RuntimeFailure);
        handle.fail(SourceFailure::RuntimeFailure);

        // then: one event, however often it repeats
        // Polled: the pipeline task reports on the runtime's own thread.
        wait_until(DEADLINE, "the degraded event", || {
            !reports.states().is_empty()
        })
        .expect("the pipeline reports the failure");
        assert_eq!(
            reports.states(),
            [WatcherState::Degraded(DegradedReason::RuntimeFailure)]
        );
        assert_eq!(server.stop(), Stopped::Drained);
    }

    #[test]
    fn should_release_listener_when_watcher_init_panics_or_fails() {
        let closed = Arc::new(AtomicBool::new(false));
        let cases: [(&str, Factory); 2] = [
            ("panics", Box::new(panicking)),
            (
                "fails",
                Box::new(watcher(Some("an injected watcher failure"), &closed)),
            ),
        ];
        for (case, factory) in cases {
            // given
            let root = site();
            let (server, _reports, _warnings) = start(&root, factory);
            let mut server = server.expect("a watcher failure is degraded mode");
            assert_eq!(
                server.state(),
                LifecycleState::Serving { watcher: false },
                "{case}"
            );
            let addr = server.local_addr();

            // when
            let stopped = server.stop();

            // then: the port is free at once, for a connection and for a new listener
            assert_eq!(stopped, Stopped::Drained, "{case}");
            assert_eq!(server.state(), LifecycleState::Stopped, "{case}");
            let refused = TcpStream::connect(addr).map_err(|error| error.kind());
            assert_eq!(refused.err(), Some(ErrorKind::ConnectionRefused), "{case}");
            StdListener::bind(addr)
                .unwrap_or_else(|error| panic!("{case}: port {addr} still held: {error}"));
        }
    }

    #[test]
    fn should_be_noop_when_stop_called_twice() {
        // given
        let root = site();
        let closed = Arc::new(AtomicBool::new(false));
        let (server, reports, _warnings) = start(&root, watcher(None, &closed));
        let mut server = server.expect("a server");
        assert_eq!(server.state(), LifecycleState::Serving { watcher: true });
        let first = server.stop();

        // when
        let second = server.stop();

        // then
        assert_eq!((first, second), (Stopped::Drained, Stopped::Drained));
        assert_eq!(server.state(), LifecycleState::Stopped);
        assert!(
            closed.load(Ordering::SeqCst),
            "the pipeline closed the watcher"
        );
        assert_eq!(reports.recorded(), ["ready Watching { dirs: 1 }"]);
        drop(server);
    }

    /// An open `GET /__site_preview/events` on `addr`, read up to the end of the response
    /// head: the stream is live and holds its connection.
    fn open_event_stream(addr: SocketAddr) -> TcpStream {
        let mut stream = TcpStream::connect(addr).expect("the server accepts");
        stream
            .set_read_timeout(Some(DEADLINE))
            .expect("a read timeout");
        let head = format!("GET /__site_preview/events HTTP/1.1\r\nHost: {addr}\r\n\r\n");
        stream
            .write_all(head.as_bytes())
            .expect("the request is sent");
        let mut seen = Vec::new();
        let mut byte = [0_u8; 1];
        while !seen.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).expect("a response head");
            seen.push(byte[0]);
        }
        assert!(seen.starts_with(b"HTTP/1.1 200"), "{seen:?}");
        stream
    }

    #[test]
    fn should_not_announce_drain_when_only_a_tab_is_open() {
        // given: a browser tab's event stream, which never ends on its own
        let root = site();
        let closed = Arc::new(AtomicBool::new(false));
        let (server, reports, _warnings) = start(&root, watcher(None, &closed));
        let mut server = server.expect("a server");
        let mut events = open_event_stream(server.local_addr());

        // when
        let stopped = server.stop();

        // then: the stream was ended, not dropped when the drain bound passed
        assert_eq!(stopped, Stopped::Drained);
        let mut rest = Vec::new();
        events
            .read_to_end(&mut rest)
            .expect("the stream ends cleanly");
        assert!(
            rest.ends_with(b"0\r\n\r\n"),
            "the chunked body is terminated: {}",
            String::from_utf8_lossy(&rest)
        );
        assert!(closed.load(Ordering::SeqCst), "the watcher is closed");
        assert_eq!(
            reports.recorded(),
            ["ready Watching { dirs: 1 }"],
            "the shutdown waited for nothing, so it says nothing"
        );
    }

    /// A server with the quiet fake watcher and a drain bound of twice the notice grace, so a
    /// drain that waits for its bound has passed the grace when it ends.
    fn start_bounded(root: &TempDir) -> (Server, Reports) {
        let reports = Reports::default();
        let mut config = ServeConfig::new(root.path().to_path_buf());
        config.limits.drain_timeout = DRAIN_NOTICE_GRACE * 2;
        let closed = Arc::new(AtomicBool::new(false));
        let server = Server::start_with(
            &config,
            Box::new(reports.clone()),
            Arc::new(Warnings::default()),
            watcher(None, &closed),
            Signals::Ignore,
            &SystemOpener,
        )
        .expect("a server");
        (server, reports)
    }

    /// Sends the first stop as a signal does and waits until the serve task has ended: stands
    /// in for the stopping thread being scheduled late on a loaded machine.
    fn stop_serve_task_alone(server: &mut Server) {
        server
            .stops
            .send(StopSignal::Terminate)
            .expect("the serve task runs");
        server.await_listener_closed();
        // Polled: the drain runs on the runtime's threads; the deadline only bounds a hang.
        wait_until(DEADLINE, "the serve task to end", || {
            server.serve.as_ref().is_some_and(JoinHandle::is_finished)
        })
        .expect("the serve task ends");
    }

    #[test]
    fn should_not_announce_drain_when_stopping_thread_runs_late_with_only_a_tab_open() {
        // given: a browser tab's event stream, which never ends on its own
        let root = site();
        let (mut server, reports) = start_bounded(&root);
        let mut events = open_event_stream(server.local_addr());

        // when: the serve task stops and drains before the stopping thread performs the rest
        // of the shutdown (`CloseStreams` included)
        stop_serve_task_alone(&mut server);
        let stopped = server.stop();

        // then: the serve task ended the stream itself, so the drain waited for nothing
        assert_eq!(stopped, Stopped::Drained);
        let mut rest = Vec::new();
        events
            .read_to_end(&mut rest)
            .expect("the stream ends cleanly");
        assert!(
            rest.ends_with(b"0\r\n\r\n"),
            "the chunked body is terminated: {}",
            String::from_utf8_lossy(&rest)
        );
        assert_eq!(reports.recorded(), ["ready Watching { dirs: 1 }"]);
    }

    #[test]
    fn should_stop_at_once_when_second_stop_arrives_while_joining() {
        // given: a drain that has ended, and a pipeline that would hold the join for the rest
        // of the drain bound (a reconcile that blocks)
        let root = site();
        let (mut server, _reports) = start_bounded(&root);
        let stuck = server
            .runtime
            .as_ref()
            .expect("a running server")
            .spawn(pending::<()>());
        drop(server.pipeline.replace(stuck));
        stop_serve_task_alone(&mut server);

        // when: a second Ctrl-C, once the serve task has ended
        let _sent = server.stops.send(StopSignal::Interrupt);
        let stopped = server.stop();

        // then: the join is cut short, as during the drain
        assert_eq!(stopped, Stopped::Forced(StopSignal::Interrupt));
    }

    #[test]
    fn should_announce_drain_when_joined_after_a_stalled_drain_ended() {
        // given: a client stuck in its request head, which the server has accepted (the reply
        // to a later request shows it: the accept loop takes connections in the kernel's order)
        let root = site();
        let reports = Reports::default();
        let mut config = ServeConfig::new(root.path().to_path_buf());
        config.limits.drain_timeout = DRAIN_NOTICE_GRACE * 2;
        let closed = Arc::new(AtomicBool::new(false));
        let mut server = Server::start_with(
            &config,
            Box::new(reports.clone()),
            Arc::new(Warnings::default()),
            watcher(None, &closed),
            Signals::Ignore,
            &SystemOpener,
        )
        .expect("a server");
        let mut stalled = TcpStream::connect(server.local_addr()).expect("the server accepts");
        stalled
            .write_all(b"GET / HTTP/1.1\r\nHost:")
            .expect("half a head is sent");
        assert_eq!(get_status(server.local_addr()), 200);
        server
            .stops
            .send(StopSignal::Terminate)
            .expect("the serve task runs");
        server.await_listener_closed();
        // Polled: stands in for the stopping thread being scheduled late on a loaded machine,
        // until the drain, which runs on the runtime's threads, has ended on its bound.
        wait_until(DEADLINE, "the drain to end", || {
            server.serve.as_ref().is_some_and(JoinHandle::is_finished)
        })
        .expect("the drain ends on its bound");

        // when
        let stopped = server.stop();

        // then: the drain waited past the grace, so it is told, however late
        assert_eq!(stopped, Stopped::DrainTimedOut);
        assert_eq!(
            reports.recorded(),
            ["ready Watching { dirs: 1 }", "draining 1s"]
        );
    }

    #[test]
    fn should_start_no_watcher_and_never_report_ready_when_bind_fails() {
        // given: the port is held by another listener
        let root = site();
        let occupied = StdListener::bind("127.0.0.1:0").expect("a test listener");
        let port = occupied.local_addr().expect("a bound port").port();
        let mut config = ServeConfig::new(root.path().to_path_buf());
        config.port = port;
        let reports = Reports::default();
        let started = Arc::new(AtomicBool::new(false));
        let factory = {
            let started = Arc::clone(&started);
            move |root: &OpenedRoot| {
                started.store(true, Ordering::SeqCst);
                watcher(None, &Arc::new(AtomicBool::new(false)))(root)
            }
        };

        // when
        let result = Server::start_with(
            &config,
            Box::new(reports.clone()),
            Arc::new(Warnings::default()),
            factory,
            Signals::Ignore,
            &SystemOpener,
        );

        // then
        let error = result.expect_err("the port is taken");
        assert!(
            matches!(&error, StartupError::Bind { addr, source }
                if addr.port() == port && source.kind() == ErrorKind::AddrInUse),
            "{error:?}"
        );
        assert!(
            !started.load(Ordering::SeqCst),
            "no watcher after a bind failure"
        );
        assert!(reports.recorded().is_empty(), "never ready");
    }

    #[test]
    fn should_refuse_with_its_variant_when_root_missing_or_reserved() {
        // given
        let missing = tempfile::tempdir().expect("a temporary directory");
        let missing_root = missing.path().join("missing");
        let reserved = site();
        std::fs::create_dir(reserved.path().join("__Site_Preview")).expect("a reserved name");
        let factory = || watcher(None, &Arc::new(AtomicBool::new(false)));
        let refuse = |root: std::path::PathBuf| {
            Server::start_with(
                &ServeConfig::new(root),
                Box::new(Reports::default()),
                Arc::new(Warnings::default()),
                factory(),
                Signals::Ignore,
                &SystemOpener,
            )
        };

        // when
        let refusals = [refuse(missing_root), refuse(reserved.path().to_path_buf())];

        // then
        let [missing, reserved] = refusals.map(|r| r.expect_err("refused"));
        assert!(
            matches!(missing, StartupError::InvalidRoot(_)),
            "{missing:?}"
        );
        assert!(
            matches!(reserved, StartupError::ReservedPath(_)),
            "{reserved:?}"
        );
        assert!(
            reserved.to_string().contains("rename or remove it"),
            "{reserved}"
        );
    }
}
