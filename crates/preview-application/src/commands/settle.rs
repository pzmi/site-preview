//! Settling the debounce: the `settle` command publishes the next revision (plan.md section
//! 8.3; D29).

use preview_domain::{ChangeSet, Deadline, DegradeReason, DomainEvent, Poll, SiteState, Tick};

use super::change_coordinator::ChangeCoordinator;

/// A revision that was published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// The state that was published, with its new revision.
    pub state: SiteState,
    /// Changes were lost or could not be attributed: the caller rescans every eligible
    /// directory and reconciles its watches (plan.md section 8.3).
    pub rescan: bool,
    /// The changes the revision carries; full whenever `rescan` is.
    pub changes: ChangeSet,
}

/// What [`ChangeCoordinator::settle`] decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettleOutcome {
    /// The revision published now, if the deadline had passed.
    pub settled: Option<Settled>,
    /// The deadline still pending when `settle` was called too early; the caller sleeps
    /// until then. `None` once nothing is pending.
    pub next_deadline: Option<Deadline>,
    /// [`DomainEvent::RevisionPublished`] for a published revision, or the one
    /// [`DomainEvent::WatcherDegraded`] that reports the revisions running out.
    pub events: Vec<DomainEvent>,
}

impl ChangeCoordinator {
    /// Publishes the pending changes once their deadline has passed.
    ///
    /// Before the deadline it publishes nothing and returns the deadline; with nothing
    /// pending it returns an empty outcome. At the deadline it takes the accumulated
    /// [`ChangeSet`] from the debouncer and publishes it with the next revision through
    /// [`ReloadState::publish_next`](crate::ReloadState::publish_next), so the state and its
    /// change set reach subscribers together (D29). Pinned by
    /// `should_publish_one_revision_when_burst_settles`.
    ///
    /// When the revision counter is exhausted nothing is published. The watcher degrades
    /// with [`DegradeReason::RevisionExhausted`], reported once, and the coordinator refuses
    /// every later record and settle without retrying the publish (ADR 0040;
    /// `should_degrade_once_and_refuse_when_revisions_are_exhausted`).
    pub fn settle(&mut self, now: Tick) -> SettleOutcome {
        if self.is_exhausted() {
            return SettleOutcome::default();
        }
        let fired = match self.debouncer.poll(now) {
            Poll::Idle => return SettleOutcome::default(),
            Poll::Wait(deadline) => {
                return SettleOutcome {
                    next_deadline: Some(deadline),
                    ..SettleOutcome::default()
                };
            }
            Poll::Fire(fired) => fired,
        };
        match self.reload.publish_next(fired.changes.clone()) {
            Ok(state) => SettleOutcome {
                events: vec![DomainEvent::RevisionPublished(state.clone())],
                settled: Some(Settled {
                    state,
                    rescan: fired.rescan,
                    changes: fired.changes,
                }),
                next_deadline: None,
            },
            Err(_exhausted) => SettleOutcome {
                events: self.watcher_failed(DegradeReason::RevisionExhausted),
                ..SettleOutcome::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use preview_domain::{
        ChangeKind, DebounceWindow, InstanceId, Revision, WatcherHealth, WatcherInput,
    };

    use super::*;
    use crate::commands::test_support::{event, ms};
    use crate::reload_state::ReloadState;

    fn instance() -> InstanceId {
        InstanceId::parse("test-1").expect("valid id")
    }

    fn coordinator_at(revision: u64) -> ChangeCoordinator {
        let window = DebounceWindow::from_millis(150).expect("in range");
        let reload = ReloadState::from_state(SiteState::new(instance(), Revision::from(revision)));
        ChangeCoordinator::new(window, reload)
    }

    #[test]
    fn should_return_deadline_and_publish_nothing_when_called_early() {
        // given
        let mut c = coordinator_at(0);
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        // when
        let early = c.settle(ms(149));
        // then
        assert_eq!(early.settled, None);
        assert!(early.events.is_empty());
        assert_eq!(early.next_deadline.map(Deadline::at), Some(ms(150)));
        assert_eq!(c.reload_state().current().revision(), Revision::ZERO);
    }

    #[test]
    fn should_return_empty_outcome_when_nothing_is_pending() {
        // given
        let mut c = coordinator_at(0);
        // when
        let outcome = c.settle(ms(1_000));
        // then
        assert_eq!(outcome, SettleOutcome::default());
    }

    #[test]
    fn should_publish_state_and_changes_together_when_settled() {
        // given
        let mut c = coordinator_at(6);
        let mut subscription = c.reload_state().subscribe();
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        // when
        let outcome = c.settle(ms(150));
        // then
        let settled = outcome.settled.expect("published");
        assert_eq!(settled.state.revision().get(), 7);
        assert!(!settled.rescan);
        assert_eq!(
            outcome.events,
            [DomainEvent::RevisionPublished(settled.state.clone())]
        );
        let seen = futures_now(subscription.changed()).expect("open");
        assert_eq!(seen.state, settled.state);
        assert_eq!(seen.changes, settled.changes);
    }

    #[test]
    fn should_degrade_once_and_refuse_when_revisions_are_exhausted() {
        // given
        let mut c = coordinator_at(u64::MAX);
        c.watcher_ready(3).expect("accepted");
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        // when
        let first = c.settle(ms(150));
        let later_record = c.record(&[event(ChangeKind::Modify, &["b.css"])], ms(200));
        let later_overflow = c.overflow(ms(210));
        let later_settle = c.settle(ms(1_000));
        let later_failure = c.watcher_failed(DegradeReason::RuntimeFailure);
        // then
        assert_eq!(first.settled, None);
        assert_eq!(
            first.events,
            [DomainEvent::WatcherDegraded {
                reason: DegradeReason::RevisionExhausted
            }]
        );
        assert_eq!(
            c.health(),
            WatcherHealth::Degraded {
                reason: DegradeReason::RevisionExhausted
            }
        );
        assert_eq!(later_record.next_deadline, None);
        assert_eq!(later_record.recorded, 0);
        assert_eq!(later_overflow.next_deadline, None);
        assert_eq!(later_settle, SettleOutcome::default());
        assert!(later_failure.is_empty());
        assert_eq!(c.deadline(), None, "nothing is left pending for a retry");
        assert_eq!(c.reload_state().current().revision().get(), u64::MAX);
        assert!(c.watcher_ready(3).is_err());
        let again = WatcherInput::Failed {
            reason: DegradeReason::RevisionExhausted,
        };
        assert_eq!(c.health().step(again).map(|t| t.event), Ok(None));
    }

    #[test]
    fn should_escalate_to_exhaustion_when_already_degraded() {
        // given
        let mut c = coordinator_at(u64::MAX);
        c.watcher_failed(DegradeReason::RuntimeFailure);
        c.overflow(ms(0));
        // when
        let outcome = c.settle(ms(150));
        // then
        assert_eq!(
            outcome.events,
            [DomainEvent::WatcherDegraded {
                reason: DegradeReason::RevisionExhausted
            }]
        );
    }

    /// Polls a future once with a no-op waker; the value must already be available.
    fn futures_now<F: core::future::Future>(future: F) -> F::Output {
        let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
        match core::pin::pin!(future).poll(&mut cx) {
            core::task::Poll::Ready(value) => value,
            core::task::Poll::Pending => panic!("expected a value to be ready"),
        }
    }
}
