//! What a run reports and why it can fail, and [`run`], the binary's entry point over
//! [`Server`] (plan.md section 9, ADR 0028, ADR 0049, ADR 0051).

use core::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use preview_adapter_http::WarningSink;
use preview_domain::InstanceId;

use crate::bootstrap::{RootRefusal, WatchStart, start_watcher};
use crate::config::{BindRefusal, ServeConfig};
use crate::connections::Stopped;
use crate::launcher::SystemOpener;
use crate::lifecycle::{Server, Signals};

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
    pub(crate) fn of(start: &'a WatchStart) -> Self {
        match &start.unavailable {
            Some(reason) => Self::Unavailable { reason },
            None if start.is_complete() => Self::Watching { dirs: start.dirs },
            None => Self::Partial { dirs: start.dirs },
        }
    }
}

/// Why the watcher is degraded, in the categories the machine interface names (README
/// "Watcher events"); never a path or operating-system text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedReason {
    /// Initialization or a watch registration failed.
    RegistrationFailed,
    /// The operating system's watcher reported a failure while running.
    RuntimeFailure,
    /// The revision counter is exhausted: live reload cannot resume until a restart.
    RevisionExhausted,
}

impl DegradedReason {
    /// The stable `reason` string of the `--json` watcher event. Pinned by
    /// `should_name_each_degraded_reason_with_a_stable_code_when_rendered`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RegistrationFailed => "registration-failed",
            Self::RuntimeFailure => "runtime-failure",
            Self::RevisionExhausted => "revision-exhausted",
        }
    }
}

/// A change in whether live reload works, after the server is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherState {
    /// The watcher works again after a degradation.
    Watching,
    /// Live reload is unavailable or unreliable; pages are still served.
    Degraded(DegradedReason),
}

/// Receives the watcher's state changes from the pipeline task, so it must be callable from
/// any thread. Called once per change, never for a repeated failure (the health machine
/// absorbs those, `should_emit_degraded_once_when_failures_repeat`).
pub trait WatcherEvents: Send + Sync {
    /// The watcher's state changed to `state`.
    fn changed(&self, state: WatcherState);
}

/// Drops every watcher change: the sink of a reporter that has no use for them.
struct NoWatcherEvents;

impl WatcherEvents for NoWatcherEvents {
    fn changed(&self, _state: WatcherState) {}
}

/// Receives the run's reports; the CLI renders them as human output or as JSON lines.
pub trait StartupReporter {
    /// Called once, after the listener is bound and before the first request is served.
    fn ready(&mut self, ready: &Ready<'_>);

    /// Called once, when a stop signal arrived and the shutdown is still waiting for open
    /// connections after a short grace: it waits up to `bound` for them, and a second signal
    /// stops it at once. A shutdown that finishes within the grace (a browser tab with its
    /// event stream and idle keep-alive connections) never calls it.
    fn draining(&mut self, bound: Duration);

    /// Where the watcher pipeline reports later changes of its state; asked once, before the
    /// pipeline starts. A change that happens before [`Self::ready`] is returned may reach the
    /// sink first: the sink is responsible for ordering it after the ready report. Defaults to
    /// dropping them (the human output reports them as warnings only).
    fn watcher_events(&self) -> Arc<dyn WatcherEvents> {
        Arc::new(NoWatcherEvents)
    }
}

/// Why a `serve` run could not start. Every variant is a run-time failure (exit 1), except
/// [`StartupError::RemoteBind`], which the CLI reports as a usage error (exit 2). Nothing
/// acquired before the failure stays allocated (ADR 0051).
#[derive(Debug)]
pub enum StartupError {
    /// A non-loopback address without `--allow-remote`. The CLI checks
    /// [`ServeConfig::bind_scope`] first and reports a usage error; this is the backstop.
    RemoteBind(BindRefusal),
    /// The root does not exist, is not a directory or cannot be read
    /// (`should_exit_1_when_root_missing`).
    InvalidRoot(RootRefusal),
    /// The root holds `__site_preview` in some letter case, which would shadow the internal
    /// endpoints (`should_exit_1_with_message_when_reserved_path_exists`).
    ReservedPath(RootRefusal),
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

impl From<RootRefusal> for StartupError {
    fn from(refusal: RootRefusal) -> Self {
        if refusal.is_reserved_path() {
            Self::ReservedPath(refusal)
        } else {
            Self::InvalidRoot(refusal)
        }
    }
}

impl fmt::Display for StartupError {
    /// Each message says what to do next. Pinned by `should_exit_1_when_explicit_port_in_use`
    /// and `should_suggest_next_step_when_bind_fails`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteBind(refusal) => refusal.fmt(f),
            Self::InvalidRoot(refusal) | Self::ReservedPath(refusal) => refusal.fmt(f),
            Self::Runtime(source) => write!(
                f,
                "cannot start the async runtime: {source}; free some memory or threads and retry"
            ),
            Self::Signal(source) => write!(f, "cannot install the signal handlers: {source}"),
            Self::Bind { addr, source } if addr.port() == 0 => write!(
                f,
                "cannot listen on {addr}: {source}; check that --host names an address of this machine"
            ),
            Self::Bind { addr, source } => write!(
                f,
                "cannot listen on {addr}: {source}; pick another --port, or omit it to use any free port"
            ),
        }
    }
}

impl core::error::Error for StartupError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Runtime(source) | Self::Signal(source) | Self::Bind { source, .. } => {
                Some(source)
            }
            Self::RemoteBind(_) | Self::InvalidRoot(_) | Self::ReservedPath(_) => None,
        }
    }
}

/// Serves `config.root` until SIGINT or SIGTERM: [`Server`] started with the signal handlers
/// installed, then stopped by the first signal (plan.md section 9, ADR 0051; the signals and
/// the shutdown order are in `shutdown.rs`).
///
/// The first signal closes the listener, ends every open event stream (a browser tab never
/// holds the process), stops the watcher pipeline (joined within the drain bound) and drains
/// the other connections for at most the drain bound; a second signal stops at once
/// ([`Stopped::Forced`]). Pinned end to end by `should_serve_index_when_started_on_fixture`,
/// `should_exit_1_when_root_missing` and the lifecycle tests
/// (`should_exit_0_when_sigterm_received`,
/// `should_exit_0_when_sigint_received_with_open_sse_connection`,
/// `should_close_listener_when_shut_down`,
/// `should_exit_0_within_drain_bound_when_clients_stall`,
/// `should_exit_130_when_second_sigint_arrives_during_drain`). Warnings about served pages
/// (ADR 0020, ADR 0046) and about the watcher (ADR 0049) go to `warnings`. A watcher that
/// cannot watch anything is degraded mode, never a start-up failure.
///
/// # Errors
///
/// [`StartupError`] as documented on its variants.
pub fn run(
    config: &ServeConfig,
    reporter: Box<dyn StartupReporter>,
    warnings: Arc<dyn WarningSink>,
) -> Result<Stopped, StartupError> {
    let mut server = Server::start_with(
        config,
        reporter,
        warnings,
        start_watcher,
        Signals::Forward,
        &SystemOpener,
    )?;
    Ok(server.serve_until_stopped())
}

/// Warns about each directory that could not be watched (`watch-registration-failed`;
/// README "Warnings"). That nothing can be watched is the lifecycle's `Degraded` effect.
pub(crate) fn report_watch_problems(watch: &WatchStart, warnings: &dyn WarningSink) {
    for problem in &watch.problems {
        warnings.warn("watch-registration-failed", problem);
    }
}

/// The `watch-unavailable` message: why, and that pages are still served. Pinned by
/// `should_serve_degraded_when_watcher_unavailable`.
pub(crate) fn unavailable_warning(reason: &str) -> String {
    format!(
        "file watching is unavailable: {reason}; live reload is disabled, pages are still served"
    )
}

/// The address a browser on this machine uses for the listener at `local`. A wildcard is
/// never a destination (the Host policy refuses `0.0.0.0` and `::`, ADR 0037), so a wildcard
/// listener is reported through the loopback address of its family, which it also serves.
/// Pinned by `should_report_loopback_url_when_listener_is_wildcard`.
pub(crate) fn reachable(local: SocketAddr) -> SocketAddr {
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
pub(crate) fn instance_id() -> InstanceId {
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

    #[test]
    fn should_suggest_next_step_when_bind_fails() {
        // given
        let in_use = |port| StartupError::Bind {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            source: io::Error::from(io::ErrorKind::AddrInUse),
        };

        // when
        let messages = [in_use(4173), in_use(0)].map(|error| error.to_string());

        // then
        assert!(
            messages[0].starts_with("cannot listen on 127.0.0.1:4173: "),
            "{messages:?}"
        );
        assert!(messages[0].ends_with("pick another --port, or omit it to use any free port"));
        assert!(messages[1].ends_with("check that --host names an address of this machine"));
        let source = core::error::Error::source(&in_use(4173)).map(ToString::to_string);
        assert!(source.is_some(), "the io::Error stays the source");
    }
}
