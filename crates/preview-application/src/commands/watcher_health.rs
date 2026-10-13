//! The watcher's health: the `watcher_failed` and `watcher_ready` commands (plan.md section
//! 8.4).
//!
//! A failure comes in through the change source as
//! [`ChangeBatch::Failed`](crate::ports::ChangeBatch::Failed) and reaches
//! [`ChangeCoordinator::watcher_failed`] through [`ChangeCoordinator::receive`]. Readiness, and so recovery, is reported by the
//! caller after the watcher adapter registered its directories, at startup or after a
//! reconcile or a bounded retry: the source has no "recovered" batch (ADR 0040).

use preview_domain::{
    DegradeReason, DomainEvent, Tick, WatcherHealth, WatcherHealthRefusal, WatcherInput,
};

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

    /// Reports that start-up registered `dirs` directories, every eligible one: the
    /// coordinator moves from `Starting` to `Watching` and emits nothing
    /// (`should_watch_silently_when_ready_at_startup`).
    ///
    /// # Errors
    ///
    /// [`WatcherHealthRefusal`] in any health but `Starting`, which is unchanged: this is the
    /// start-up report only. Readiness after a reconcile goes through
    /// [`watcher_reconciled`](Self::watcher_reconciled), which also makes the revision after a
    /// recovery a full reload; accepting a recovery here would claim live reload again without
    /// catching up the changes it missed (`should_refuse_startup_ready_when_not_starting`,
    /// FORMAL-10). Also for `dirs == 0`: the root is always watched, so no registered
    /// directory means no watch at all, and live reload must not be claimed
    /// (`should_refuse_ready_when_nothing_was_registered`).
    pub fn watcher_ready(&mut self, dirs: usize) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        let input = WatcherInput::Ready { dirs };
        if self.health != WatcherHealth::Starting {
            return Err(WatcherHealthRefusal {
                state: self.health,
                input,
            });
        }
        self.ready(input)
    }

    /// Feeds `input` (a `Ready`) to the health machine and returns its event.
    fn ready(&mut self, input: WatcherInput) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        let transition = self.health.apply(input)?;
        Ok(transition.event.into_iter().collect())
    }

    /// Reports, at `now`, that a reconcile left `dirs` directories registered and none
    /// outstanding: readiness, and when that is a recovery
    /// ([`DomainEvent::WatcherRecovered`], once) the next revision is a full reload, due a
    /// debounce window after `now` (ADR 0054).
    ///
    /// While a directory could not be watched, a change made inside it was never notified;
    /// the reload that follows the recovery is full because nothing else would publish that
    /// change, and live reload is claimed again from this report on
    /// (`should_publish_full_revision_when_watcher_recovers`). No rescan is requested: the
    /// reconcile that reported readiness has just registered every directory. Readiness that
    /// is no recovery (from `Starting`, or while already watching) marks nothing, so start-up
    /// causes no reload (`should_mark_nothing_when_reconciled_without_recovery`).
    ///
    /// # Errors
    ///
    /// [`WatcherHealthRefusal`] once the revisions are exhausted or the root is removed: live
    /// reload can never resume, so the caller carries on; the health is unchanged
    /// (`should_degrade_once_and_refuse_when_revisions_are_exhausted`). Also for
    /// `dirs == 0`, as [`watcher_ready`](Self::watcher_ready). A refusal marks nothing.
    pub fn watcher_reconciled(
        &mut self,
        dirs: usize,
        now: Tick,
    ) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        let events = self.ready(WatcherInput::Ready { dirs })?;
        if events.contains(&DomainEvent::WatcherRecovered) {
            self.debouncer.mark_recovered(now);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use preview_domain::{
        ChangeKind, Deadline, DebounceWindow, FullReason, InstanceId, WatcherHealth,
    };

    use super::*;
    use crate::commands::test_support::{event, ms};
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
        let first = c.watcher_reconciled(2, ms(0)).expect("accepted");
        let second = c.watcher_reconciled(3, ms(10)).expect("accepted");
        // then
        assert_eq!(first, [DomainEvent::WatcherRecovered]);
        assert!(second.is_empty());
        assert_eq!(c.health(), WatcherHealth::Watching { dirs: 3 });
    }

    #[test]
    fn should_refuse_startup_ready_when_not_starting() {
        // given: one coordinator degraded, one already watching
        let mut degraded = coordinator();
        degraded.watcher_failed(SourceFailure::RegistrationFailed);
        let mut watching = coordinator();
        watching.watcher_ready(2).expect("accepted at start-up");
        // when
        let recovery = degraded.watcher_ready(3);
        let again = watching.watcher_ready(3);
        // then: neither moves; a recovery cannot skip the full reload `watcher_reconciled` marks
        assert_eq!(
            recovery,
            Err(WatcherHealthRefusal {
                state: WatcherHealth::Degraded {
                    reason: DegradeReason::RegistrationFailed
                },
                input: WatcherInput::Ready { dirs: 3 }
            })
        );
        assert_eq!(
            degraded.health(),
            WatcherHealth::Degraded {
                reason: DegradeReason::RegistrationFailed
            }
        );
        assert_eq!(degraded.deadline(), None);
        assert!(again.is_err());
        assert_eq!(watching.health(), WatcherHealth::Watching { dirs: 2 });
    }

    #[test]
    fn should_refuse_recovery_when_root_is_removed() {
        // given
        let mut c = coordinator();
        c.watcher_ready(2).expect("accepted");
        // when
        let removed = c.watcher_failed(SourceFailure::RootRemoved);
        let recovery = c.watcher_reconciled(2, ms(0));
        // then: degraded for good, and no full reload is marked by the refused readiness
        assert_eq!(
            removed,
            [DomainEvent::WatcherDegraded {
                reason: DegradeReason::RootRemoved
            }]
        );
        assert!(recovery.is_err());
        assert_eq!(c.deadline(), None);
        assert!(!c.health().is_live());
    }

    #[test]
    fn should_publish_full_revision_when_watcher_recovers() {
        // given: degraded, with nothing pending
        let mut c = coordinator();
        c.watcher_ready(2).expect("accepted");
        c.watcher_failed(SourceFailure::RegistrationFailed);
        // when
        let events = c.watcher_reconciled(3, ms(100)).expect("accepted");
        let early = c.settle(ms(249));
        let outcome = c.settle(ms(250));
        // then: one full reload a window later, with no rescan
        assert_eq!(events, [DomainEvent::WatcherRecovered]);
        assert_eq!(early.next_deadline.map(Deadline::at), Some(ms(250)));
        let settled = outcome.settled.expect("the recovery publishes");
        assert!(!settled.rescan);
        assert_eq!(settled.changes.reason(), Some(FullReason::Recovered));
        assert_eq!(settled.state.revision().get(), 1);
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn should_make_pending_hot_revision_full_when_watcher_recovers() {
        // given: a stylesheet edit pending while degraded
        let mut c = coordinator();
        c.watcher_failed(SourceFailure::RuntimeFailure);
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        // when
        c.watcher_reconciled(3, ms(40)).expect("accepted");
        let outcome = c.settle(ms(190));
        // then: still one revision, now a full one carrying the edit
        let settled = outcome.settled.expect("settled");
        assert!(settled.changes.is_full());
        assert_eq!(settled.changes.changes().len(), 1);
        assert_eq!(c.reload_state().current().revision().get(), 1);
    }

    #[test]
    fn should_mark_nothing_when_reconciled_without_recovery() {
        // given
        let mut c = coordinator();
        // when: readiness from `Starting`, then again while watching
        let startup = c.watcher_reconciled(3, ms(0)).expect("accepted");
        let again = c.watcher_reconciled(4, ms(10)).expect("accepted");
        // then: no reload is due
        assert!(startup.is_empty());
        assert!(again.is_empty());
        assert_eq!(c.deadline(), None);
        assert_eq!(c.settle(ms(1_000)), crate::SettleOutcome::default());
    }
}
