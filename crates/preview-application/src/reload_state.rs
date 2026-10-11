//! The latest-state channel between the change pipeline and the pages (plan.md sections 7.1
//! and 7.2; ADR 0013).
//!
//! [`ReloadState`] holds one value, the newest [`Published`] revision, in a
//! `tokio::sync::watch`. A subscriber always sees the newest value and never a queue, so a
//! slow or vanished browser costs no memory (pinned by
//! `should_deliver_latest_only_when_reader_is_slow`). A reader that skipped revisions can
//! detect the gap from the revision numbers it saw; the change set of a skipped revision is
//! gone, which is why the SSE layer must treat a gap as a full reload (plan.md section 7.2).
//!
//! The channel is a port contract between the commands that publish and the adapters that
//! subscribe: neither side names `tokio` types, only the types here.

use std::sync::Arc;

use preview_domain::{ChangeSet, FullReason, InstanceId, RevisionExhausted, SiteState};
use tokio::sync::watch;

/// One published revision: the state and the changes that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The instance and revision this value announces.
    pub state: SiteState,
    /// What changed since the previous revision. Only meaningful for a reader that saw the
    /// previous revision; across a gap the reader must reload fully.
    pub changes: ChangeSet,
}

/// The channel is closed: every [`ReloadState`] handle was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReloadClosed;

impl std::fmt::Display for ReloadClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the reload state was dropped")
    }
}

impl std::error::Error for ReloadClosed {}

/// The latest site state and the way to publish and watch it. Clones share one channel.
///
/// Reading the state for an HTML response goes through [`ReloadState::current`], which
/// returns the [`SiteState`] alone (D29).
#[derive(Debug, Clone)]
pub struct ReloadState {
    tx: Arc<watch::Sender<Published>>,
}

impl ReloadState {
    /// A channel at revision 0 for a new server instance.
    ///
    /// The initial value has an empty change list and is marked full
    /// ([`FullReason::Rescan`]): nothing is known about the page a first subscriber holds.
    /// Pinned by `should_start_full_at_revision_zero_when_created`.
    #[must_use]
    pub fn new(instance: InstanceId) -> Self {
        Self::from_state(SiteState::initial(instance))
    }

    fn from_state(state: SiteState) -> Self {
        let mut changes = ChangeSet::new();
        changes.mark_full(FullReason::Rescan);
        let (tx, _rx) = watch::channel(Published { state, changes });
        Self { tx: Arc::new(tx) }
    }

    /// The current state. A request handler takes this before it opens content, so the
    /// snapshot never runs ahead of the bytes (plan.md section 7.3).
    #[must_use]
    pub fn current(&self) -> SiteState {
        self.tx.borrow().state.clone()
    }

    /// Subscribes, then reads the value that was current at that moment.
    ///
    /// Subscribing comes first and the read marks that value as seen, so a publish racing
    /// with this call is delivered exactly once: either as [`StateSubscription::initial`] or
    /// by the next [`StateSubscription::changed`], never lost and never twice (plan.md
    /// section 7.2; `should_observe_publish_after_subscribe_when_published_between_subscribe_and_read`).
    #[must_use]
    pub fn subscribe(&self) -> StateSubscription {
        let mut rx = self.tx.subscribe();
        let initial = rx.borrow_and_update().clone();
        StateSubscription { rx, initial }
    }

    /// Publishes the next revision with the changes that produced it.
    ///
    /// The read-increment-write is one atomic step on the channel, so concurrent publishers
    /// never produce a repeated or smaller revision. Pinned by
    /// `should_never_decrease_revision_when_publishing`. On exhaustion nothing is published
    /// and the state is unchanged (`should_refuse_and_keep_state_when_revision_exhausted`).
    ///
    /// # Errors
    ///
    /// [`RevisionExhausted`] when the revision counter cannot advance.
    pub fn publish_next(&self, changes: ChangeSet) -> Result<SiteState, RevisionExhausted> {
        let mut outcome = Err(RevisionExhausted);
        let mut changes = Some(changes);
        self.tx
            .send_if_modified(|published| match published.state.advanced() {
                Ok(next) => {
                    published.state = next.clone();
                    published.changes = changes.take().unwrap_or_default();
                    outcome = Ok(next);
                    true
                }
                Err(e) => {
                    outcome = Err(e);
                    false
                }
            });
        outcome
    }
}

/// One subscriber's view: the value at subscribe time and the newer ones, coalesced.
#[derive(Debug)]
pub struct StateSubscription {
    rx: watch::Receiver<Published>,
    initial: Published,
}

impl StateSubscription {
    /// The value observed when this subscription was created.
    #[must_use]
    pub fn initial(&self) -> &Published {
        &self.initial
    }

    /// Waits for a value newer than the last one this subscription returned and returns the
    /// newest. Several publishes in between collapse into this one value.
    ///
    /// Cancel safe: dropping the future loses nothing.
    ///
    /// # Errors
    ///
    /// [`ReloadClosed`] once every [`ReloadState`] handle is dropped.
    pub async fn changed(&mut self) -> Result<Published, ReloadClosed> {
        self.rx.changed().await.map_err(|_| ReloadClosed)?;
        Ok(self.rx.borrow_and_update().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use preview_domain::{Revision, SiteState};
    use proptest::prelude::*;
    use std::future::Future;

    fn instance() -> InstanceId {
        InstanceId::parse("test-1").expect("valid id")
    }

    /// Whether `changed()` would complete right now, polled once without a timer or sleep.
    fn is_ready(subscription: &mut StateSubscription) -> bool {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut future = std::pin::pin!(subscription.changed());
        future.as_mut().poll(&mut cx).is_ready()
    }

    fn full(reason: FullReason) -> ChangeSet {
        let mut set = ChangeSet::new();
        set.mark_full(reason);
        set
    }

    #[test]
    fn should_start_full_at_revision_zero_when_created() {
        // given
        let reload = ReloadState::new(instance());
        // when
        let subscription = reload.subscribe();
        // then
        let initial = subscription.initial();
        assert_eq!(initial.state.revision(), Revision::ZERO);
        assert!(initial.changes.changes().is_empty());
        assert!(initial.changes.is_full());
        assert_eq!(reload.current(), initial.state);
    }

    #[tokio::test]
    async fn should_deliver_latest_only_when_reader_is_slow() {
        // given
        let reload = ReloadState::new(instance());
        let mut subscription = reload.subscribe();
        // when
        for _ in 0..3 {
            reload
                .publish_next(ChangeSet::new())
                .expect("not exhausted");
        }
        let seen = subscription.changed().await.expect("open");
        // then
        assert_eq!(seen.state.revision().get(), 3);
        let ready = is_ready(&mut subscription);
        assert!(!ready, "no queued older revision may remain");
    }

    #[tokio::test]
    async fn should_observe_publish_after_subscribe_when_published_between_subscribe_and_read() {
        // given
        let reload = ReloadState::new(instance());
        // when
        let mut subscription = reload.subscribe();
        let published = reload
            .publish_next(full(FullReason::Removed))
            .expect("not exhausted");
        // then
        assert_eq!(subscription.initial().state.revision(), Revision::ZERO);
        let seen = subscription.changed().await.expect("open");
        assert_eq!(seen.state, published);
        assert_eq!(seen.changes.reason(), Some(FullReason::Removed));
        assert!(!is_ready(&mut subscription), "delivered once");
    }

    #[tokio::test]
    async fn should_deliver_once_when_published_before_subscribe() {
        // given
        let reload = ReloadState::new(instance());
        reload
            .publish_next(ChangeSet::new())
            .expect("not exhausted");
        // when
        let mut subscription = reload.subscribe();
        // then
        assert_eq!(subscription.initial().state.revision().get(), 1);
        assert!(!is_ready(&mut subscription));
    }

    #[test]
    fn should_refuse_and_keep_state_when_revision_exhausted() {
        // given
        let last = SiteState::new(instance(), Revision::from(u64::MAX));
        let reload = ReloadState::from_state(last.clone());
        let subscription = reload.subscribe();
        // when
        let result = reload.publish_next(ChangeSet::new());
        // then
        assert_eq!(result, Err(RevisionExhausted));
        assert_eq!(reload.current(), last);
        assert_eq!(subscription.initial().state, last);
    }

    #[tokio::test]
    async fn should_report_closed_when_every_handle_is_dropped() {
        // given
        let reload = ReloadState::new(instance());
        let mut subscription = reload.subscribe();
        // when
        drop(reload);
        // then
        assert_eq!(subscription.changed().await.map(|_| ()), Err(ReloadClosed));
    }

    #[test]
    fn should_share_one_channel_when_cloned() {
        // given
        let a = ReloadState::new(instance());
        let b = a.clone();
        // when
        let published = a.publish_next(ChangeSet::new()).expect("not exhausted");
        // then
        assert_eq!(b.current(), published);
    }

    proptest! {
        #[test]
        fn should_never_decrease_revision_when_publishing(
            start in 0u64..1_000,
            steps in proptest::collection::vec(0u8..3, 0..40),
        ) {
            // given
            let reload = ReloadState::from_state(SiteState::new(instance(), Revision::from(start)));
            let other = reload.clone();
            let mut previous = reload.current().revision();
            // when
            for step in steps {
                let handle = if step == 1 { &other } else { &reload };
                let published = handle.publish_next(ChangeSet::new()).expect("far from max");
                // then
                prop_assert!(published.revision() > previous);
                prop_assert_eq!(published.revision().get(), previous.get() + 1);
                prop_assert_eq!(reload.current(), published.clone());
                previous = published.revision();
            }
        }
    }
}
