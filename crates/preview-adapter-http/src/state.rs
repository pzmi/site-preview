//! The shared state every handler receives.

use std::sync::Arc;

use preview_application::{ReloadState, SiteContent};
use preview_domain::HostPolicy;

/// What the handlers share: the site's content port, the reload state and the Host policy.
///
/// Clones are cheap and share the same content and channel.
#[derive(Clone)]
pub struct AppState {
    /// Where files are opened from. Its calls block, so handlers run them on a blocking
    /// thread.
    pub content: Arc<dyn SiteContent>,
    /// The latest site state, read when a response needs the embedded snapshot.
    pub reload: ReloadState,
    /// Which Host authorities this listener answers for. Composition builds it from the
    /// address the listener actually bound, never from a requested port 0.
    pub host: HostPolicy,
}
