//! What a `serve` run is asked to do.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

/// The settings of one `serve` run, already parsed by the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeConfig {
    /// The directory to serve, as the user gave it; it is resolved once, at start-up.
    pub root: PathBuf,
    /// The address to listen on.
    pub host: IpAddr,
    /// The port to listen on; 0 lets the operating system choose a free one.
    pub port: u16,
}

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
        }
    }
}

#[cfg(test)]
mod tests {
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
    }
}
