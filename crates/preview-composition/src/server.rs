//! Running the server: runtime, listener, router and shutdown (plan.md section 9, ADR 0028).

use core::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Shutdown, StreamLimit, build_router,
};
use preview_application::ReloadState;
use preview_domain::{BoundAddr, HostPolicy, InstanceId};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::bootstrap::{RootRefusal, open_root};
use crate::config::{BindRefusal, ServeConfig};

/// What the server reports once it accepts connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ready<'a> {
    /// The resolved site root.
    pub root: &'a Path,
    /// The base URL of the bound listener, for example `http://127.0.0.1:49152/`.
    pub url: &'a str,
}

/// Receives the start-up report; the CLI renders it as human output (or, later, JSON).
pub trait StartupReporter {
    /// Called once, after the listener is bound and before the first request is served.
    fn ready(&mut self, ready: &Ready<'_>);
}

/// Why a `serve` run could not start or stopped abnormally. Every variant is a run-time
/// failure (exit 1), never a usage error.
#[derive(Debug)]
pub enum StartupError {
    /// A non-loopback address without `--allow-remote`. The CLI checks
    /// [`ServeConfig::bind_scope`] first and reports a usage error; this is the backstop.
    RemoteBind(BindRefusal),
    /// The root does not exist, is not a readable directory, or holds `__site_preview`.
    Root(RootRefusal),
    /// The Tokio runtime could not be built.
    Runtime(io::ErrorKind),
    /// The Ctrl-C handler could not be installed.
    Signal(io::ErrorKind),
    /// The listener could not be bound.
    Bind {
        /// The address that was asked for.
        addr: SocketAddr,
        /// Why binding failed.
        kind: io::ErrorKind,
    },
    /// Accepting connections failed after start-up.
    Serve(io::ErrorKind),
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteBind(refusal) => refusal.fmt(f),
            Self::Root(refusal) => refusal.fmt(f),
            Self::Runtime(kind) => write!(f, "cannot start the async runtime: {kind}"),
            Self::Signal(kind) => write!(f, "cannot install the Ctrl-C handler: {kind}"),
            Self::Bind { addr, kind } => write!(f, "cannot listen on {addr}: {kind}"),
            Self::Serve(kind) => write!(f, "the server stopped: {kind}"),
        }
    }
}

impl core::error::Error for StartupError {}

/// Serves `config.root` until Ctrl-C.
///
/// In order (plan.md section 9): open the root, build a multi-thread runtime, bind the
/// listener, install the Ctrl-C handler, build the router over a fresh [`ReloadState`], a
/// [`HostPolicy`] for the address actually bound and `--allow-remote`, and a [`Shutdown`]
/// for the event streams, report [`Ready`], then serve. Ctrl-C stops accepting and ends every
/// open event stream, so the graceful shutdown does not wait on a browser tab (the stream side
/// is pinned by `should_end_stream_when_shutdown_token_cancelled` in the HTTP adapter; the
/// process side by the lifecycle tests). A non-loopback host without `--allow-remote` is
/// refused before anything opens; an explicit port in use is [`StartupError::Bind`]
/// (`should_exit_1_when_explicit_port_in_use`). A clean Ctrl-C returns `Ok(())`. Pinned end to end by
/// `should_serve_index_when_started_on_fixture` and `should_exit_1_when_directory_does_not_exist`.
///
/// # Errors
///
/// [`StartupError`] as documented on its variants.
pub fn run(config: &ServeConfig, out: &mut dyn StartupReporter) -> Result<(), StartupError> {
    config.bind_scope().map_err(StartupError::RemoteBind)?;
    let root = open_root(&config.root).map_err(StartupError::Root)?;
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| StartupError::Runtime(e.kind()))?;
    runtime.block_on(async {
        let addr = SocketAddr::new(config.host, config.port);
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| StartupError::Bind {
                addr,
                kind: e.kind(),
            })?;
        let local = listener.local_addr().map_err(|e| StartupError::Bind {
            addr,
            kind: e.kind(),
        })?;
        let interrupt =
            signal(SignalKind::interrupt()).map_err(|e| StartupError::Signal(e.kind()))?;
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
        axum::serve(listener, build_router(state))
            .with_graceful_shutdown(async move {
                interrupted(interrupt).await;
                // A graceful shutdown waits for open responses, and an event stream never
                // ends on its own: end them now, or one open tab holds the process.
                streams.trigger();
            })
            .await
            .map_err(|e| StartupError::Serve(e.kind()))
    })
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

/// Resolves on the first SIGINT. A minimal handler: SIGTERM and the shutdown order of plan.md
/// section 9 come with the lifecycle task.
async fn interrupted(mut interrupt: Signal) {
    interrupt.recv().await;
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
