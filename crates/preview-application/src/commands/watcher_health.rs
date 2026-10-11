//! The watcher's health: the `watcher_failed` and `watcher_ready` commands (plan.md section
//! 8.4).
//!
//! A failure comes in through the change source as
//! [`ChangeBatch::Failed`](crate::ports::ChangeBatch::Failed) and reaches
//! [`ChangeCoordinator::watcher_failed`] through [`ChangeCoordinator::receive`]. Readiness, and so recovery, is reported by the
//! caller after the watcher adapter registered its directories, at startup or after a
//! reconcile or a bounded retry: the source has no "recovered" batch (ADR 0040).

use preview_domain::{DegradeReason, DomainEvent, WatcherHealthRefusal, WatcherInput};

use super::change_coordinator::ChangeCoordinator;
use crate::ports::SourceFailure;

impl ChangeCoordinator {
    /// Reports that the watcher failed: live reload is unavailable, HTTP serving continues.
    ///
    /// Returns [`DomainEvent::WatcherDegraded`] when this failure degrades the watcher, and
    /// nothing when it was already degraded, so a repeating failure cannot produce an event
    /// storm (`should_emit_degraded_once_when_failures_repeat`). The pending changes are
    /// kept: whatever is recorded still settles and publishes.
    ///
    /// Takes a [`SourceFailure`], which cannot name revision exhaustion: that absorbing
    /// degradation comes from [`settle`](Self::settle) alone (ADR 0040, amended).
    pub fn watcher_failed(&mut self, failure: SourceFailure) -> Vec<DomainEvent> {
        self.degrade(failure.into())
    }

    /// Degrades the watcher with `reason`; the one path into the health machine's failure
    /// input, shared by [`watcher_failed`](Self::watcher_failed) and `settle`.
    pub(super) fn degrade(&mut self, reason: DegradeReason) -> Vec<DomainEvent> {
        // A failure is never refused by the health machine; only `Ready` can be.
        self.health
            .apply(WatcherInput::Failed { reason })
            .ok()
            .and_then(|transition| transition.event)
            .into_iter()
            .collect()
    }

    /// Reports that the watcher adapter has registered `dirs` directories.
    ///
    /// The first report after startup emits nothing; after a failure it is a recovery and
    /// emits [`DomainEvent::WatcherRecovered`] once
    /// (`should_emit_recovered_once_when_ready_after_failure`).
    ///
    /// # Errors
    ///
    /// [`WatcherHealthRefusal`] once the revisions are exhausted: live reload can never
    /// resume, so the caller logs the refusal and carries on; the health is unchanged
    /// (`should_degrade_once_and_refuse_when_revisions_are_exhausted`). Also for `dirs == 0`:
    /// the root is always watched, so no registered directory means no watch at all, and live
    /// reload must not be claimed (`should_refuse_ready_when_nothing_was_registered`).
    pub fn watcher_ready(&mut self, dirs: usize) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        let transition = self.health.apply(WatcherInput::Ready { dirs })?;
        Ok(transition.event.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use preview_domain::{DebounceWindow, InstanceId, WatcherHealth};

    use super::*;
    use crate::reload_state::ReloadState;

    fn coordinator() -> ChangeCoordinator {
        ChangeCoordinator::new(
            DebounceWindow::from_millis(150).expect("in range"),
            ReloadState::new(InstanceId::parse("test-1").expect("valid id")),
        )
    }

    #[test]
    fn should_watch_silently_when_ready_at_startup() {
        // given
        let mut c = coordinator();
        // when
        let events = c.watcher_ready(4);
        // then
        assert_eq!(events, Ok(vec![]));
        assert_eq!(c.health(), WatcherHealth::Watching { dirs: 4 });
        assert!(c.health().is_live());
    }

    #[test]
    fn should_refuse_ready_when_nothing_was_registered() {
        // given
        let mut c = coordinator();
        // when
        let refused = c.watcher_ready(0);
        // then
        assert!(refused.is_err());
        assert_eq!(c.health(), WatcherHealth::Starting);
        assert!(!c.health().is_live());
    }

    #[test]
    fn should_emit_degraded_once_when_failures_repeat() {
        // given
        let mut c = coordinator();
        c.watcher_ready(4).expect("accepted");
        // when
        let reported: Vec<_> = [
            SourceFailure::RuntimeFailure,
            SourceFailure::RegistrationFailed,
            SourceFailure::RuntimeFailure,
        ]
        .into_iter()
        .flat_map(|reason| c.watcher_failed(reason))
        .collect();
        // then
        assert_eq!(
            reported,
            [DomainEvent::WatcherDegraded {
                reason: DegradeReason::RuntimeFailure
            }]
        );
        assert!(!c.health().is_live());
    }

    #[test]
    fn should_emit_recovered_once_when_ready_after_failure() {
        // given
        let mut c = coordinator();
        c.watcher_failed(SourceFailure::RegistrationFailed);
        // when
        let first = c.watcher_ready(2).expect("accepted");
        let second = c.watcher_ready(3).expect("accepted");
        // then
        assert_eq!(first, [DomainEvent::WatcherRecovered]);
        assert!(second.is_empty());
        assert_eq!(c.health(), WatcherHealth::Watching { dirs: 3 });
    }
}
