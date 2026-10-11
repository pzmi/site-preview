//! Recording changes: the `record`, `overflow` and `receive` commands (plan.md sections 8.2
//! and 8.3).

use preview_domain::{ChangeEvent, Classification, Deadline, DomainEvent, Tick};

use super::change_coordinator::ChangeCoordinator;
use crate::ports::ChangeBatch;

/// What recording a batch decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordOutcome {
    /// When the pending changes settle, if anything is pending after this batch: the caller
    /// arms its timer for it and calls [`ChangeCoordinator::settle`] then. An all-ignored
    /// batch leaves an earlier deadline as it was.
    pub next_deadline: Option<Deadline>,
    /// The directory structure may have changed: the caller reconciles the watch set
    /// (plan.md section 8.3) before the revision settles.
    pub needs_reconcile: bool,
    /// How many events of the batch reached the debouncer; the rest were ignored.
    pub recorded: usize,
    /// Health events the batch caused (only a [`ChangeBatch::Failed`] causes any).
    pub events: Vec<DomainEvent>,
}

/// What [`ChangeCoordinator::receive`] did with one batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    /// The batch was handled; keep consuming.
    Recorded(RecordOutcome),
    /// The source has stopped for good; stop consuming. Anything still pending settles at
    /// its deadline as usual.
    Closed,
}

impl ChangeCoordinator {
    /// Records one batch of change events.
    ///
    /// Each event is classified by the domain's debouncer first
    /// ([`preview_domain::classify`]); an ignored event (an
    /// ineligible path such as `.git/` or `node_modules/`, an access-only or metadata-only
    /// event) never reaches the debouncer, so it neither dirties it nor moves its deadline
    /// (`should_leave_debouncer_untouched_when_every_event_is_ignored`). Every other event is
    /// recorded, so no eligible change in a mixed batch is lost
    /// (`should_record_every_relevant_event_when_batch_is_mixed`).
    ///
    /// `needs_reconcile` is set whenever the domain says the directory structure may have
    /// changed ([`Classification::requests_reconcile`]): a known directory change, a create,
    /// remove or rename of an entry of unknown type whose name is eligible as a directory, and
    /// an event too coarse to attribute ([`Classification::Rescan`]), which may hide one
    /// (`should_request_reconcile_when_directory_changes`,
    /// `should_request_reconcile_when_entry_of_unknown_type_is_renamed`).
    ///
    /// Once no revision can be published any more (ADR 0040), the batch is refused: nothing
    /// is recorded and no deadline is returned
    /// (`should_degrade_once_and_refuse_when_revisions_are_exhausted`).
    pub fn record(&mut self, events: &[ChangeEvent], now: Tick) -> RecordOutcome {
        let mut outcome = RecordOutcome::default();
        if self.is_exhausted() {
            return outcome;
        }
        for event in events {
            let classification = self.debouncer.record(now, event);
            if !matches!(classification, Classification::Ignored(_)) {
                outcome.recorded += 1;
                outcome.needs_reconcile |= classification.requests_reconcile();
            }
        }
        outcome.next_deadline = self.debouncer.deadline();
        outcome
    }

    /// Records that changes were lost (the change source overflowed): the next revision is a
    /// full rescan and a full reload, and the watch set must be reconciled (plan.md section
    /// 8.3). Pinned by `should_publish_with_rescan_when_overflow_signalled`.
    ///
    /// Refused like [`record`](Self::record) once the revisions are exhausted.
    pub fn overflow(&mut self, now: Tick) -> RecordOutcome {
        if self.is_exhausted() {
            return RecordOutcome::default();
        }
        let deadline = self.debouncer.mark_overflow(now);
        RecordOutcome {
            next_deadline: Some(deadline),
            needs_reconcile: true,
            recorded: 0,
            events: Vec::new(),
        }
    }

    /// Handles one batch from the [`ChangeSource`](crate::ports::ChangeSource): events are
    /// [`record`](Self::record)ed, an overflow is [`overflow`](Self::overflow), a failure is
    /// [`watcher_failed`](Self::watcher_failed), and `Closed` ends consumption. A failure
    /// keeps the pending deadline, unless the revisions are exhausted, when it returns none
    /// (`should_return_no_deadline_when_revisions_are_exhausted`).
    ///
    /// The one place a batch is mapped to a command, so the watcher pipeline in composition
    /// makes no decision of its own. Pinned by the tests in `tests/change_commands.rs`.
    pub fn receive(&mut self, batch: ChangeBatch<ChangeEvent>, now: Tick) -> Received {
        match batch {
            ChangeBatch::Events(events) => Received::Recorded(self.record(&events, now)),
            ChangeBatch::Overflow => Received::Recorded(self.overflow(now)),
            ChangeBatch::Failed(failure) => {
                let events = self.watcher_failed(failure);
                Received::Recorded(RecordOutcome {
                    next_deadline: self.deadline(),
                    needs_reconcile: false,
                    recorded: 0,
                    events,
                })
            }
            ChangeBatch::Closed => Received::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use preview_domain::{ChangeKind, DebounceWindow, DegradeReason, InstanceId, WatcherHealth};

    use super::*;
    use crate::commands::test_support::{event, ms};
    use crate::ports::SourceFailure;
    use crate::reload_state::ReloadState;

    fn coordinator() -> ChangeCoordinator {
        let window = DebounceWindow::from_millis(150).expect("in range");
        ChangeCoordinator::new(
            window,
            ReloadState::new(InstanceId::parse("test-1").expect("valid id")),
        )
    }

    #[test]
    fn should_leave_debouncer_untouched_when_every_event_is_ignored() {
        // given
        let mut c = coordinator();
        let pending = c.record(&[event(ChangeKind::Modify, &["index.html"])], ms(0));
        let before = c.debouncer.clone();
        // when
        let outcome = c.record(
            &[
                event(ChangeKind::Modify, &[".git", "index"]),
                event(ChangeKind::Create, &["node_modules", "x", "y.js"]),
                event(ChangeKind::AccessOnly, &["index.html"]),
                event(ChangeKind::MetadataOnly, &["style.css"]),
                event(ChangeKind::Create, &["index.html~"]),
            ],
            ms(100),
        );
        // then
        assert_eq!(outcome.recorded, 0);
        assert!(!outcome.needs_reconcile);
        assert_eq!(outcome.next_deadline, pending.next_deadline);
        assert_eq!(
            c.debouncer, before,
            "an ignored event touched the debouncer"
        );
    }

    #[test]
    fn should_not_dirty_when_only_ignored_events_arrive_on_idle_coordinator() {
        // given
        let mut c = coordinator();
        // when
        let outcome = c.record(&[event(ChangeKind::Modify, &[".git", "HEAD"])], ms(5));
        // then
        assert_eq!(outcome, RecordOutcome::default());
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn should_record_every_relevant_event_when_batch_is_mixed() {
        // given
        let mut c = coordinator();
        // when
        let outcome = c.record(
            &[
                event(ChangeKind::Modify, &[".git", "index"]),
                event(ChangeKind::Modify, &["a.css"]),
                event(ChangeKind::AccessOnly, &["b.css"]),
                event(ChangeKind::Modify, &["c.js"]),
            ],
            ms(10),
        );
        // then
        assert_eq!(outcome.recorded, 2);
        assert_eq!(outcome.next_deadline.map(Deadline::at), Some(ms(160)));
        let fired = c.settle(ms(160)).settled.expect("settled");
        let paths: Vec<_> = fired
            .changes
            .changes()
            .iter()
            .map(|p| p.url_path().to_owned())
            .collect();
        assert_eq!(paths, ["/a.css", "/c.js"]);
    }

    #[test]
    fn should_request_reconcile_when_directory_changes() {
        // given
        let mut c = coordinator();
        // when
        let file = c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        let dir = c.record(&[event(ChangeKind::DirectoryChanged, &["blog"])], ms(1));
        let coarse = c.record(&[event(ChangeKind::Ambiguous, &["blog", "x"])], ms(2));
        let ignored_dir = c.record(
            &[event(ChangeKind::DirectoryChanged, &["node_modules"])],
            ms(3),
        );
        // then
        assert!(!file.needs_reconcile);
        assert!(dir.needs_reconcile);
        assert!(coarse.needs_reconcile);
        assert!(!ignored_dir.needs_reconcile);
    }

    #[test]
    fn should_request_reconcile_when_entry_of_unknown_type_is_renamed() {
        // given
        let mut c = coordinator();
        // when
        let moved_in = c.record(&[event(ChangeKind::Rename, &["docs"])], ms(0));
        let removed = c.record(&[event(ChangeKind::Remove, &["blog", "2024"])], ms(1));
        let created = c.record(&[event(ChangeKind::Create, &["assets"])], ms(2));
        let modified = c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(3));
        let excluded = c.record(&[event(ChangeKind::Rename, &["vendor"])], ms(4));
        // then
        assert!(moved_in.needs_reconcile);
        assert!(removed.needs_reconcile);
        assert!(created.needs_reconcile);
        assert!(!modified.needs_reconcile);
        assert!(!excluded.needs_reconcile);
        assert_eq!(excluded.recorded, 1);
    }

    #[test]
    fn should_publish_full_reload_without_path_when_excluded_name_is_removed() {
        // given
        let mut c = coordinator();
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        c.record(&[event(ChangeKind::Remove, &["vendor"])], ms(1));
        // when
        let settled = c.settle(ms(151)).settled.expect("settled");
        // then
        assert!(settled.changes.is_full());
        let paths: Vec<_> = settled
            .changes
            .changes()
            .iter()
            .map(|p| p.url_path().to_owned())
            .collect();
        assert_eq!(paths, ["/a.css"]);
    }

    #[test]
    fn should_mark_full_rescan_and_reconcile_when_overflow_recorded() {
        // given
        let mut c = coordinator();
        // when
        let outcome = c.overflow(ms(40));
        // then
        assert!(outcome.needs_reconcile);
        assert_eq!(outcome.next_deadline.map(Deadline::at), Some(ms(190)));
        let settled = c.settle(ms(190)).settled.expect("settled");
        assert!(settled.rescan);
        assert!(settled.changes.is_full());
    }

    #[test]
    fn should_keep_pending_deadline_when_failure_is_received() {
        // given
        let mut c = coordinator();
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        // when
        let received = c.receive(ChangeBatch::Failed(SourceFailure::RuntimeFailure), ms(50));
        // then
        let Received::Recorded(outcome) = received else {
            panic!("a failure is not the end of the source")
        };
        assert_eq!(outcome.next_deadline.map(Deadline::at), Some(ms(150)));
        assert_eq!(
            outcome.events,
            [DomainEvent::WatcherDegraded {
                reason: DegradeReason::RuntimeFailure
            }]
        );
    }

    #[test]
    fn should_return_no_deadline_when_revisions_are_exhausted() {
        // given: changes pending when the coordinator becomes inert
        let mut c = coordinator();
        c.record(&[event(ChangeKind::Modify, &["a.css"])], ms(0));
        c.health = WatcherHealth::Degraded {
            reason: DegradeReason::RevisionExhausted,
        };
        // when
        let received = c.receive(ChangeBatch::Failed(SourceFailure::RuntimeFailure), ms(500));
        // then
        let Received::Recorded(outcome) = received else {
            panic!("a failure is not the end of the source")
        };
        assert_eq!(outcome.next_deadline, None);
        assert!(outcome.events.is_empty());
        assert_eq!(c.deadline(), None, "an inert coordinator arms no timer");
        assert_eq!(c.settle(ms(500)), crate::SettleOutcome::default());
    }

    #[test]
    fn should_stop_when_closed_is_received() {
        // given
        let mut c = coordinator();
        // when
        let received = c.receive(ChangeBatch::Closed, ms(0));
        // then
        assert_eq!(received, Received::Closed);
    }
}
