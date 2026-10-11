//! The shared state every handler receives.

use std::sync::Arc;

use preview_application::{ReloadState, SiteContent};

/// What the handlers share: the site's content port and the reload state.
///
/// Clones are cheap and share the same content and channel. The Host policy joins this struct
/// in HTTP-05; it is not a field yet because nothing reads it before then.
#[derive(Clone)]
pub struct AppState {
    /// Where files are opened from. Its calls block, so handlers run them on a blocking
    /// thread.
    pub content: Arc<dyn SiteContent>,
    /// The latest site state, read when a response needs the embedded snapshot.
    pub reload: ReloadState,
}
