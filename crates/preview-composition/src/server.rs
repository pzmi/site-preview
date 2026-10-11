//! Running the server: runtime, listener, router and shutdown (plan.md section 9, ADR 0028).

use core::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use preview_adapter_http::{AppState, build_router};
use preview_application::ReloadState;
use preview_domain::{BoundAddr, HostPolicy, InstanceId};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::bootstrap::{RootRefusal, open_root};
use crate::config::ServeConfig;

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
/// listener, install the Ctrl-C handler, build the router over a fresh [`ReloadState`] and a
/// [`HostPolicy`] for the bound address (no `--allow-remote` yet), report [`Ready`], then
/// serve. A clean Ctrl-C returns `Ok(())`. Pinned end to end by
/// `should_serve_index_when_started_on_fixture` and `should_exit_1_when_directory_does_not_exist`.
///
/// # Errors
///
/// [`StartupError`] as documented on its variants.
pub fn run(config: &ServeConfig, out: &mut dyn StartupReporter) -> Result<(), StartupError> {
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
        let state = AppState {
            content: root.content,
            reload: ReloadState::new(instance_id()),
            // From the address actually bound: with `--port 0` the request said 0, and a
            // policy over port 0 would refuse every Host.
            host: HostPolicy::new(BoundAddr::new(local.ip(), local.port()), false),
        };
        let url = format!("http://{local}/");
        out.ready(&Ready {
            root: &root.display_path,
            url: &url,
        });
        axum::serve(listener, build_router(state))
            .with_graceful_shutdown(interrupted(interrupt))
            .await
            .map_err(|e| StartupError::Serve(e.kind()))
    })
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
