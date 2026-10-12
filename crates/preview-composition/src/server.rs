//! Running the server: runtime, listener, router and shutdown (plan.md section 9, ADR 0028).

use core::fmt;
use core::future::poll_fn;
use core::task::Poll;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Shutdown, StreamLimit, build_router,
};
use preview_application::ReloadState;
use preview_domain::{BoundAddr, HostPolicy, InstanceId};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::bootstrap::{RootRefusal, open_root};
use crate::config::{BindRefusal, ServeConfig};
use crate::connections::{StopSignal, Stopped, serve_connections};

/// What the server reports once it accepts connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ready<'a> {
    /// The resolved site root.
    pub root: &'a Path,
    /// The base URL of the bound listener, for example `http://127.0.0.1:49152/`.
    pub url: &'a str,
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
/// a [`Shutdown`] for the event streams, report [`Ready`], then serve within
/// [`crate::ServerLimits`] (ADR 0045). The first signal closes the listener, ends every open
/// event stream (a browser tab never holds the process) and drains the other connections for
/// at most the drain bound; a second signal stops at once ([`Stopped::Forced`]). A non-loopback
/// host without `--allow-remote` is refused before anything opens; an explicit port in use is
/// [`StartupError::Bind`] (`should_exit_1_when_explicit_port_in_use`). Pinned end to end by
/// `should_serve_index_when_started_on_fixture`, `should_exit_1_when_directory_does_not_exist`
/// and the lifecycle tests (`should_exit_0_within_drain_bound_when_clients_stall`,
/// `should_exit_130_when_second_sigint_arrives_during_drain`).
///
/// # Errors
///
/// [`StartupError`] as documented on its variants.
pub fn run(config: &ServeConfig, out: &mut dyn StartupReporter) -> Result<Stopped, StartupError> {
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
        let state = AppState {
            content: root.content,
            reload: ReloadState::new(instance_id()),
            // From the address actually bound: with `--port 0` the request said 0, and a
            // policy over port 0 would refuse every Host.
            host: HostPolicy::new(
                BoundAddr::new(local.ip(), local.port()),
                config.allow_remote,
            ),
            shutdown: streams.signal(),
            keep_alive: DEFAULT_KEEP_ALIVE,
            streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
        };
        let url = format!("http://{}/", reachable(local));
        out.ready(&Ready {
            root: &root.display_path,
            url: &url,
        });
        let limits = config.limits;
        Ok(
            serve_connections(listener, build_router(state), limits, &mut stops, |open| {
                // A drain waits for open responses, and an event stream never ends on its
                // own: end them now, or one open tab holds the process for the whole bound.
                streams.trigger();
                if open > 0 {
                    out.draining(limits.drain_timeout);
                }
            })
            .await,
        )
    });
    // Neither a dropped connection nor a blocking read still running may delay the exit: the
    // drain bound is the promise.
    runtime.shutdown_background();
    stopped
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
}
