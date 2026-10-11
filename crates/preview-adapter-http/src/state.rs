//! The shared state every handler receives.

use std::sync::Arc;
use std::time::Duration;

use preview_application::{ReloadState, SiteContent};
use preview_domain::HostPolicy;

use crate::shutdown::ShutdownSignal;

/// What the handlers share: the site's content port, the reload state, the Host policy and
/// what the event streams need.
///
/// Clones are cheap and share the same content and channel.
#[derive(Clone)]
pub struct AppState {
    /// Where files are opened from. Its calls block, so handlers run them on a blocking
    /// thread.
    pub content: Arc<dyn SiteContent>,
    /// The latest site state, read when a response needs the embedded snapshot and
    /// subscribed to by every event stream.
    pub reload: ReloadState,
    /// Which Host authorities this listener answers for. Composition builds it from the
    /// address the listener actually bound, never from a requested port 0.
    pub host: HostPolicy,
    /// Ends every open event stream when it fires, so a graceful shutdown is not held open
    /// by a browser tab (plan.md section 9).
    pub shutdown: ShutdownSignal,
    /// Silence after which an event stream sends a keepalive comment; composition passes
    /// [`crate::DEFAULT_KEEP_ALIVE`].
    pub keep_alive: Duration,
}
