//! Running the server: runtime, listener, router, watcher pipeline and shutdown (plan.md
//! section 9, ADR 0028, ADR 0049).

use core::fmt;
use core::future::poll_fn;
use core::task::Poll;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Injection, Shutdown, StreamLimit,
    WarningSink, build_router,
};
use preview_application::{ChangeCoordinator, ReloadState};
use preview_domain::{BoundAddr, HostPolicy, InstanceId};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout_at};

use crate::bootstrap::{RootRefusal, WatchStart, open_root, start_watcher};
use crate::config::{BindRefusal, ServeConfig};
use crate::connections::{StopSignal, Stopped, serve_connections};
use crate::watch_pipeline::{TokioClock, WatchPipeline};

/// What the server reports once it accepts connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ready<'a> {
    /// The resolved site root.
    pub root: &'a Path,
    /// The base URL of the bound listener, for example `http://127.0.0.1:49152/`.
    pub url: &'a str,
    /// What the file watcher covers, so live reload is never claimed when it is off.
    pub watching: WatchStatus<'a>,
}

/// What the file watcher covers at start-up (plan.md section 3, "Human startup output";
/// section 8.1 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus<'a> {
    /// Every eligible directory is watched: live reload works.
    Watching {
        /// The number of watched directories; at least one, the root.
        dirs: usize,
    },
    /// Some eligible directories could not be watched (each was warned about): live reload
    /// misses changes in them.
    Partial {
        /// The number of watched directories; at least one.
        dirs: usize,
    },
    /// Nothing is watched: live reload is off, pages are still served.
    Unavailable {
        /// Why, in words for a person; never an absolute path.
        reason: &'a str,
    },
}

impl<'a> WatchStatus<'a> {
    /// Pinned by `should_report_watch_status_when_registration_is_complete_partial_or_absent`.
    fn of(start: &'a WatchStart) -> Self {
        match &start.unavailable {
            Some(reason) => Self::Unavailable { reason },
            None if start.is_complete() => Self::Watching { dirs: start.dirs },
            None => Self::Partial { dirs: start.dirs },
        }
    }
}

/// Receives the run's reports; the CLI renders them as human output (or, later, JSON).
pub trait StartupReporter {
    /// Called once, after the listener is bound and before the first request is served.
    fn ready(&mut self, ready: &Ready<'_>);

    /// Called once, when a stop signal arrived while connections were open: the server waits
    /// up to `bound` for them, and a second signal stops it at once.
    fn draining(&mut self, bound: Duration);
}

/// Why a `serve` run could not start. Every variant is a run-time failure (exit 1), never a
/// usage error.
#[derive(Debug)]
pub enum StartupError {
    /// A non-loopback address without `--allow-remote`. The CLI checks
    /// [`ServeConfig::bind_scope`] first and reports a usage error; this is the backstop.
    RemoteBind(BindRefusal),
    /// The root does not exist, is not a readable directory, or holds `__site_preview`.
    Root(RootRefusal),
    /// The Tokio runtime could not be built.
    Runtime(io::Error),
    /// The SIGINT or SIGTERM handler could not be installed.
    Signal(io::Error),
    /// The listener could not be bound.
    Bind {
        /// The address that was asked for.
        addr: SocketAddr,
        /// Why binding failed, with the operating system's own words
        /// (`should_exit_1_when_explicit_port_in_use`).
        source: io::Error,
    },
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteBind(refusal) => refusal.fmt(f),
            Self::Root(refusal) => refusal.fmt(f),
            Self::Runtime(source) => write!(f, "cannot start the async runtime: {source}"),
            Self::Signal(source) => write!(f, "cannot install the signal handlers: {source}"),
            Self::Bind { addr, source } => write!(f, "cannot listen on {addr}: {source}"),
        }
    }
}

impl core::error::Error for StartupError {}

/// Serves `config.root` until SIGINT or SIGTERM.
///
/// In order (plan.md section 9): open the root, build a multi-thread runtime, bind the
/// listener, install the SIGINT and SIGTERM handlers, build the router over a fresh
/// [`ReloadState`], a [`HostPolicy`] for the address actually bound and `--allow-remote`, and
/// a [`Shutdown`] for the event streams, start the file watcher and its pipeline task (warning
/// about every directory it cannot watch), report [`Ready`], then serve within
/// [`crate::ServerLimits`] (ADR 0045). The first signal closes the listener, ends every open
/// event stream (a browser tab never holds the process), stops the watcher pipeline (joined
/// within the drain bound) and drains the other connections for
/// at most the drain bound; a second signal stops at once ([`Stopped::Forced`]). A non-loopback
/// host without `--allow-remote` is refused before anything opens; an explicit port in use is
/// [`StartupError::Bind`] (`should_exit_1_when_explicit_port_in_use`). Pinned end to end by
/// `should_serve_index_when_started_on_fixture`, `should_exit_1_when_directory_does_not_exist`
/// and the lifecycle tests (`should_exit_0_within_drain_bound_when_clients_stall`,
/// `should_exit_130_when_second_sigint_arrives_during_drain`). Warnings about served pages
/// (ADR 0020, ADR 0046) and about the watcher (ADR 0049) go to `warnings`. A watcher that
/// cannot watch anything is degraded mode, never a start-up failure.
///
/// # Errors
///
/// [`StartupError`] as documented on its variants.
pub fn run(
    config: &ServeConfig,
    out: &mut dyn StartupReporter,
    warnings: Arc<dyn WarningSink>,
) -> Result<Stopped, StartupError> {
    config.bind_scope().map_err(StartupError::RemoteBind)?;
    let root = open_root(&config.root).map_err(StartupError::Root)?;
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(StartupError::Runtime)?;
    let stopped = runtime.block_on(async {
        let addr = SocketAddr::new(config.host, config.port);
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|source| StartupError::Bind { addr, source })?;
        let local = listener
            .local_addr()
            .map_err(|source| StartupError::Bind { addr, source })?;
        let mut stops = stop_signals().map_err(StartupError::Signal)?;
        let streams = Shutdown::new();
        let reload = ReloadState::new(instance_id());
        let (source, watch) = start_watcher(&root);
        report_watch_problems(&watch, warnings.as_ref());
        let mut coordinator = ChangeCoordinator::new(config.debounce, reload.clone());
        if watch.is_complete() && watch.dirs > 0 {
            // A fresh coordinator accepts any non-zero count; nothing to report.
            let _ = coordinator.watcher_ready(watch.dirs);
        }
        let (stop_watching, watching_stopped) = oneshot::channel();
        let pipeline = WatchPipeline::new(
            source,
            coordinator,
            TokioClock::start(),
            Arc::clone(&warnings),
        );
        let mut pipeline = tokio::spawn(pipeline.run(watching_stopped));
        let state = AppState {
            content: Arc::clone(&root.content),
            reload,
            // From the address actually bound: with `--port 0` the request said 0, and a
            // policy over port 0 would refuse every Host.
            host: HostPolicy::new(
                BoundAddr::new(local.ip(), local.port()),
                config.allow_remote,
            ),
            shutdown: streams.signal(),
            keep_alive: DEFAULT_KEEP_ALIVE,
            streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
            injection: Injection::new(warnings),
        };
        let url = format!("http://{}/", reachable(local));
        out.ready(&Ready {
            root: &root.display_path,
            url: &url,
            watching: WatchStatus::of(&watch),
        });
        let limits = config.limits;
        let mut drain_ends = Instant::now() + limits.drain_timeout;
        let stopped =
            serve_connections(listener, build_router(state), limits, &mut stops, |open| {
                drain_ends = Instant::now() + limits.drain_timeout;
                // A drain waits for open responses, and an event stream never ends on its
                // own: end them now, or one open tab holds the process for the whole bound.
                streams.trigger();
                // The pipeline may already have ended (a panic); nothing is left to stop.
                let _ = stop_watching.send(());
                if open > 0 {
                    out.draining(limits.drain_timeout);
                }
            })
            .await;
        // Cancelled at the first stop, the task has normally ended long before the drain
        // does. The join shares the drain's bound, so it never extends the promised exit, and
        // after a second signal it is not waited for at all.
        if matches!(stopped, Stopped::Forced(_)) {
            drain_ends = Instant::now();
        }
        if timeout_at(drain_ends, &mut pipeline).await.is_err() {
            pipeline.abort();
        }
        Ok(stopped)
    });
    // Neither a dropped connection nor a blocking read still running may delay the exit: the
    // drain bound is the promise.
    runtime.shutdown_background();
    stopped
}

/// Warns about each directory that could not be watched, or that nothing can be watched
/// (`watch-registration-failed`, `watch-unavailable`; README "Warnings").
fn report_watch_problems(watch: &WatchStart, warnings: &dyn WarningSink) {
    for problem in &watch.problems {
        warnings.warn("watch-registration-failed", problem);
    }
    let Some(reason) = &watch.unavailable else {
        return;
    };
    warnings.warn(
        "watch-unavailable",
        &format!("file watching is unavailable: {reason}; live reload is disabled, pages are still served"),
    );
}

/// Installs the SIGINT and SIGTERM handlers and forwards every delivery, in order, to the
/// returned channel. Installed before [`Ready`] is reported, so a signal sent as soon as the
/// URL is known is never the default action (which would end the process uncleanly).
fn stop_signals() -> io::Result<mpsc::UnboundedReceiver<StopSignal>> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let next = poll_fn(|cx| {
                if let Poll::Ready(delivered) = interrupt.poll_recv(cx) {
                    return Poll::Ready(delivered.map(|()| StopSignal::Interrupt));
                }
                terminate
                    .poll_recv(cx)
                    .map(|delivered| delivered.map(|()| StopSignal::Terminate))
            })
            .await;
            // `None`: the signal driver is gone with the runtime; nothing more can arrive.
            let Some(stop) = next else { return };
            if tx.send(stop).is_err() {
                return;
            }
        }
    });
    Ok(rx)
}

/// The address a browser on this machine uses for the listener at `local`. A wildcard is
/// never a destination (the Host policy refuses `0.0.0.0` and `::`, ADR 0037), so a wildcard
/// listener is reported through the loopback address of its family, which it also serves.
/// Pinned by `should_report_loopback_url_when_listener_is_wildcard`.
fn reachable(local: SocketAddr) -> SocketAddr {
    let ip = match local.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, local.port())
}

/// This process start's identity: pid plus the wall-clock nanoseconds at start-up. The clock
/// is read here, outside the domain; a clock before the epoch gives 0, which the pid still
/// distinguishes.
fn instance_id() -> InstanceId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    InstanceId::from_parts(std::process::id(), nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_report_loopback_url_when_listener_is_wildcard() {
        let cases = [
            ("0.0.0.0:4173", "127.0.0.1:4173"),
            ("[::]:4173", "[::1]:4173"),
            ("192.168.1.20:4173", "192.168.1.20:4173"),
            ("127.0.0.1:4173", "127.0.0.1:4173"),
        ];
        for (bound, expected) in cases {
            // given
            let local: SocketAddr = bound.parse().expect("an address");

            // when
            let shown = reachable(local);

            // then
            assert_eq!(shown.to_string(), expected, "{bound}");
        }
    }

    #[test]
    fn should_report_watch_status_when_registration_is_complete_partial_or_absent() {
        // given
        let start = |dirs, problems: &[&str], unavailable: Option<&str>| WatchStart {
            dirs,
            problems: problems.iter().map(|p| (*p).to_owned()).collect(),
            unavailable: unavailable.map(str::to_owned),
        };
        let complete = start(3, &[], None);
        let partial = start(
            2,
            &["cannot watch directory `blog`: permission denied"],
            None,
        );
        let absent = start(0, &[], Some("the native file watcher is unavailable"));

        // when
        let statuses = [&complete, &partial, &absent].map(WatchStatus::of);

        // then
        assert_eq!(
            statuses,
            [
                WatchStatus::Watching { dirs: 3 },
                WatchStatus::Partial { dirs: 2 },
                WatchStatus::Unavailable {
                    reason: "the native file watcher is unavailable"
                },
            ]
        );
    }

    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<(String, String)>>);

    impl WarningSink for Recorder {
        fn warn(&self, code: &str, message: &str) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push((code.to_owned(), message.to_owned()));
        }
    }

    /// The stand-in for an unavailable native watcher in the black-box suite
    /// (`crates/preview-cli/tests/watcher.rs`), which cannot lower inotify limits.
    #[test]
    fn should_warn_and_keep_serving_when_watcher_is_unavailable() {
        // given
        let warnings = Recorder::default();
        let start = WatchStart {
            dirs: 0,
            problems: Vec::new(),
            unavailable: Some("the native file watcher is unavailable".to_owned()),
        };

        // when
        report_watch_problems(&start, &warnings);

        // then
        let recorded = warnings.0.lock().expect("unpoisoned");
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0].0, "watch-unavailable");
        assert!(
            recorded[0]
                .1
                .ends_with("live reload is disabled, pages are still served"),
            "{recorded:?}"
        );
    }
}
