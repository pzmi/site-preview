//! The [`ChangeCoordinator`]: the state the change commands share.
//!
//! The commands themselves live beside this file, one per concern: recording changes
//! ([`record_change`](super::record_change)), settling the debounce
//! ([`settle`](super::settle)) and the watcher's health
//! ([`watcher_health`](super::watcher_health)).

use preview_domain::{Deadline, DebounceWindow, Debouncer, DegradeReason, WatcherHealth};

use crate::reload_state::ReloadState;

/// Turns the watcher's change batches into published revisions and health events.
///
/// It owns the global trailing-edge [`Debouncer`] (plan.md section 8.3), the
/// [`WatcherHealth`] (plan.md section 8.4) and a handle to the [`ReloadState`] it publishes
/// to. It reads no clock and performs no I/O: every command takes `now` from the caller,
/// which owns the timer and the watcher (ADR 0028, ADR 0040), and returns its
/// [`DomainEvent`](crate::events::DomainEvent)s as values (foundation.md section 2.4).
///
/// One task owns it; it is not shared between threads.
#[derive(Debug)]
pub struct ChangeCoordinator {
    pub(super) debouncer: Debouncer,
    pub(super) health: WatcherHealth,
    pub(super) reload: ReloadState,
}

impl ChangeCoordinator {
    /// A coordinator with nothing pending, a watcher still starting, publishing to `reload`.
    #[must_use]
    pub fn new(window: DebounceWindow, reload: ReloadState) -> Self {
        Self {
            debouncer: Debouncer::new(window),
            health: WatcherHealth::Starting,
            reload,
        }
    }

    /// The watcher's health. Live reload may be claimed only when
    /// [`WatcherHealth::is_live`] holds (plan.md section 8.4).
    #[must_use]
    pub const fn health(&self) -> WatcherHealth {
        self.health
    }

    /// When the pending changes settle, if any are pending: the instant the caller should
    /// call [`settle`](Self::settle) at.
    #[must_use]
    pub fn deadline(&self) -> Option<Deadline> {
        self.debouncer.deadline()
    }

    /// The handle revisions are published through.
    #[must_use]
    pub const fn reload_state(&self) -> &ReloadState {
        &self.reload
    }

    /// Whether no revision can ever be published again (ADR 0040): the revision counter ran
    /// out, so recording, overflow and settling are refused from then on.
    pub(super) const fn is_exhausted(&self) -> bool {
        matches!(
            self.health,
            WatcherHealth::Degraded {
                reason: DegradeReason::RevisionExhausted
            }
        )
    }
}
