//! What a `serve` run is asked to do.

use core::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Duration;

use preview_domain::{BoundAddr, DebounceWindow};

/// The settings of one `serve` run, already parsed by the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeConfig {
    /// The directory to serve, as the user gave it; it is resolved once, at start-up.
    pub root: PathBuf,
    /// The address to listen on.
    pub host: IpAddr,
    /// The port to listen on; 0 lets the operating system choose a free one.
    pub port: u16,
    /// Whether a listener outside loopback is allowed (`--allow-remote`, plan.md section 3).
    pub allow_remote: bool,
    /// The quiet interval of the global debounce (`--debounce`, D22).
    pub debounce: DebounceWindow,
    /// Whether start-up and watcher events go to stdout as JSONL (`--json`).
    pub json: bool,
    /// Whether a browser is opened once the server is ready (`--open`).
    pub open: bool,
    /// Connection timeouts, the connection cap and the shutdown drain bound. Not on the
    /// command line: the CLI always uses [`ServerLimits::DEFAULT`]; tests shrink them.
    pub limits: ServerLimits,
}

/// How long a connection may hold the server, and how many may at once (ADR 0045).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerLimits {
    /// How long a connection may take to send a complete request head, counted from the
    /// moment the server waits for one. It covers a half-sent request and an idle keep-alive
    /// connection alike: hyper starts the timer whenever it is ready for the next head.
    pub header_read_timeout: Duration,
    /// How many connections are served at once. A connection accepted beyond it is closed
    /// at once, so a flood never costs more than this many descriptors.
    pub max_connections: usize,
    /// How long a shutdown waits for open responses to finish before it drops them.
    pub drain_timeout: Duration,
}

impl ServerLimits {
    /// The limits of every CLI run (ADR 0045): a 10 s head and idle timeout, 128 connections
    /// (the 64 event streams of plan.md section 7.4 plus room for page loads, and well under
    /// the 256 descriptors macOS allows a process by default) and a 3 s shutdown drain.
    pub const DEFAULT: Self = Self {
        header_read_timeout: Duration::from_secs(10),
        max_connections: 128,
        drain_timeout: Duration::from_secs(3),
    };
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How far a listener on [`ServeConfig::host`] can be reached, once it is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindScope {
    /// Only this machine can connect.
    Loopback,
    /// Other machines can connect, and nothing authenticates them: the CLI warns.
    Remote,
}

/// A bind outside loopback without `--allow-remote` (plan.md section 3, ADR 0021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindRefusal {
    host: IpAddr,
}

impl fmt::Display for BindRefusal {
    /// Pinned by `should_refuse_non_loopback_host_when_allow_remote_absent`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refusing to bind {} without --allow-remote", self.host)
    }
}

impl core::error::Error for BindRefusal {}

impl ServeConfig {
    /// Serves `root` on `127.0.0.1` with an OS-assigned port (plan.md section 3): loopback by
    /// default, and never a fixed port that a second instance would collide with.
    ///
    /// Pinned by `should_bind_loopback_on_any_port_when_built_with_defaults`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            allow_remote: false,
            debounce: DebounceWindow::DEFAULT,
            json: false,
            open: false,
            limits: ServerLimits::DEFAULT,
        }
    }

    /// Whether [`Self::host`] may be bound: loopback always, anything else (the wildcards
    /// `0.0.0.0` and `::` included) only with `allow_remote`. Loopback is judged as the Host
    /// policy judges it, so an IPv4-mapped loopback address counts.
    ///
    /// Pinned by `should_refuse_non_loopback_host_when_allow_remote_absent`,
    /// `should_scope_bind_as_loopback_when_host_is_any_loopback_address` and
    /// `should_scope_bind_as_remote_when_allow_remote_given`.
    ///
    /// # Errors
    ///
    /// [`BindRefusal`] for a non-loopback host without `allow_remote`.
    pub const fn bind_scope(&self) -> Result<BindScope, BindRefusal> {
        if BoundAddr::new(self.host, self.port).is_loopback() {
            Ok(BindScope::Loopback)
        } else if self.allow_remote {
            Ok(BindScope::Remote)
        } else {
            Err(BindRefusal { host: self.host })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use super::*;

    #[test]
    fn should_bind_loopback_on_any_port_when_built_with_defaults() {
        // given
        let root = PathBuf::from("site");

        // when
        let config = ServeConfig::new(root.clone());

        // then
        assert_eq!(config.root, root);
        assert!(config.host.is_loopback());
        assert_eq!(config.port, 0);
        assert!(!config.allow_remote);
        assert_eq!(config.debounce, DebounceWindow::DEFAULT);
        assert!(!config.json);
        assert!(!config.open);
        assert_eq!(config.limits, ServerLimits::DEFAULT);
        assert_eq!(config.bind_scope(), Ok(BindScope::Loopback));
    }

    #[test]
    fn should_refuse_non_loopback_host_when_allow_remote_absent() {
        for host in [
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
        ] {
            // given
            let mut config = ServeConfig::new(PathBuf::from("site"));
            config.host = host;

            // when
            let scope = config.bind_scope();

            // then
            let refusal = scope.expect_err("a remote bind is refused");
            assert_eq!(
                refusal.to_string(),
                format!("refusing to bind {host} without --allow-remote")
            );
        }
    }

    #[test]
    fn should_scope_bind_as_loopback_when_host_is_any_loopback_address() {
        for host in [
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()),
        ] {
            // given
            let mut config = ServeConfig::new(PathBuf::from("site"));
            config.host = host;

            // when
            let scope = config.bind_scope();

            // then
            assert_eq!(scope, Ok(BindScope::Loopback), "{host}");
        }
    }

    #[test]
    fn should_scope_bind_as_remote_when_allow_remote_given() {
        // given
        let mut config = ServeConfig::new(PathBuf::from("site"));
        config.host = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        config.allow_remote = true;

        // when
        let scope = config.bind_scope();

        // then
        assert_eq!(scope, Ok(BindScope::Remote));
    }
}
