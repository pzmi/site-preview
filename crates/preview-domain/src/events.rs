//! Events the domain emits for the application layer to publish or log.

use crate::site_state::SiteState;

/// Why the watcher is degraded. Fixed categories only, so no path or error text can reach a log
/// or an HTTP response through an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DegradeReason {
    /// Initialization or a watch registration failed (plan §8.1 step 6).
    RegistrationFailed,
    /// The OS watcher reported a runtime failure (plan §8.4).
    RuntimeFailure,
    /// The revision counter reached `u64::MAX`, so no further change can be published.
    /// Permanent: the watcher health never recovers from it.
    RevisionExhausted,
}

/// A fact the domain reports; it carries no I/O handle and no wall-clock time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    /// A debounced change produced a new state for subscribers.
    RevisionPublished(SiteState),
    /// Live reload is unavailable; HTTP serving continues.
    WatcherDegraded {
        /// The failure category.
        reason: DegradeReason,
    },
    /// The watcher is working again.
    WatcherRecovered,
}
