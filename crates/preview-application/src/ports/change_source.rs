//! The [`ChangeSource`] port: the watcher's stream of change notifications.

use core::future::Future;

use preview_domain::DegradeReason;

/// Why a [`ChangeSource`] cannot watch some or all of the site (plan.md section 8.4). A
/// category only, never a path or OS error text.
///
/// Deliberately narrower than [`DegradeReason`]: revision exhaustion is decided by the
/// application when a publish fails, never reported by a source, so a source cannot make the
/// coordinator permanently inert (ADR 0038, ADR 0040 amendments).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceFailure {
    /// Initialization or a watch registration failed (plan.md section 8.1 step 6).
    RegistrationFailed,
    /// The OS watcher reported a runtime failure (plan.md section 8.4).
    RuntimeFailure,
}

impl From<SourceFailure> for DegradeReason {
    /// The health category a source failure degrades the watcher with. Pinned by
    /// `should_degrade_with_matching_reason_when_source_fails`.
    fn from(failure: SourceFailure) -> Self {
        match failure {
            SourceFailure::RegistrationFailed => Self::RegistrationFailed,
            SourceFailure::RuntimeFailure => Self::RuntimeFailure,
        }
    }
}

/// One delivery from a [`ChangeSource`].
///
/// `E` is the change event the source translates the operating system's notifications into:
/// a root-relative path and a change kind, never an OS watcher type. The domain's change
/// event (plan.md section 8.2) is that type; the parameter keeps this port independent of it
/// until it exists (ADR 0038).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeBatch<E> {
    /// One or more changes, in the order the source observed them. Never empty.
    Events(Vec<E>),
    /// Changes were lost since the previous batch; anything may have changed. The consumer
    /// treats it as a request for a full rescan and at least one reload (plan.md section 8.3).
    Overflow,
    /// The source cannot watch some or all of the site (plan.md section 8.4). Not terminal:
    /// further batches may follow.
    Failed(SourceFailure),
    /// The source has stopped for good.
    Closed,
}

/// The watcher's stream of change notifications, consumed by one task.
///
/// # Contract
///
/// Every implementation, real or fake, honours all of the following; the `ChangeSource`
/// contract harness checks them against each one.
///
/// - **Single consumer.** `next` takes `&mut self`: one owner, one pending call at a time.
/// - **Order.** Batches, and the events inside each batch, come in the order the source
///   observed them. Events are root-relative; a change outside the root is dropped by the
///   source, never delivered.
/// - **No silent loss.** When the source drops changes (a full buffer, an OS queue
///   overflow), it reports [`ChangeBatch::Overflow`] before any change observed after the
///   loss. Several losses may be reported as one `Overflow`, but a loss is never reported as
///   nothing.
/// - **Failure is reported, not fatal.** A watch that cannot be established or breaks at
///   runtime is [`ChangeBatch::Failed`] with a fixed category; the path and OS error text
///   stay in the adapter's log, never in the batch. Any retry is bounded; the source never
///   spins re-reporting the same failure.
/// - **Closed is final.** [`ChangeBatch::Closed`] is returned once the source has stopped
///   (its producer is gone or it was shut down), and from then on every call returns
///   `Closed` at once.
/// - **Cancellation safe.** Dropping the future returned by `next` before it completes
///   loses nothing: no batch and no overflow signal. The next call returns what the dropped
///   one would have. A consumer may therefore race `next` against a debounce deadline and a
///   shutdown signal and drop the loser.
/// - **Non-blocking.** The future never blocks its thread while waiting; it returns
///   `Pending` and arranges a wake-up. It is `Send`, so the consumer can run on any worker
///   thread.
///
/// The trait is used through generics rather than `dyn`: the returned future is an
/// `impl Future`, which keeps the source allocation-free and is the shape an async adapter
/// handle needs, and the one production source is known at compile time (ADR 0038).
pub trait ChangeSource: Send {
    /// The change event this source delivers.
    type Event: Send;

    /// Waits for the next batch; see the trait's contract for order, overflow, failure,
    /// closing and cancellation.
    fn next(&mut self) -> impl Future<Output = ChangeBatch<Self::Event>> + Send;
}

#[cfg(test)]
mod tests {
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};
    use std::collections::VecDeque;

    use super::*;

    /// A source over a fixed script that is `Closed` once the script runs out. The batch is
    /// taken when the future is polled, not when it is created, so dropping an unpolled
    /// future loses nothing.
    struct Script(VecDeque<ChangeBatch<&'static str>>);

    impl ChangeSource for Script {
        type Event = &'static str;

        async fn next(&mut self) -> ChangeBatch<&'static str> {
            self.0.pop_front().unwrap_or(ChangeBatch::Closed)
        }
    }

    /// Polls a ready future once, without an async runtime.
    fn poll_ready<S: ChangeSource>(source: &mut S) -> ChangeBatch<S::Event> {
        let future = pin!(source.next());
        let mut context = Context::from_waker(Waker::noop());
        match future.poll(&mut context) {
            Poll::Ready(batch) => batch,
            Poll::Pending => panic!("a scripted source is always ready"),
        }
    }

    fn assert_send<T: Send>(_: &T) {}

    #[test]
    fn should_degrade_with_matching_reason_when_source_fails() {
        // given
        let failures = [
            SourceFailure::RegistrationFailed,
            SourceFailure::RuntimeFailure,
        ];
        // when
        let reasons = failures.map(DegradeReason::from);
        // then
        assert_eq!(
            reasons,
            [
                DegradeReason::RegistrationFailed,
                DegradeReason::RuntimeFailure
            ]
        );
    }

    #[test]
    fn should_deliver_in_order_when_an_unpolled_next_was_dropped() {
        // given
        let mut source = Script(VecDeque::from([
            ChangeBatch::Events(vec!["index.html", "style.css"]),
            ChangeBatch::Overflow,
            ChangeBatch::Failed(SourceFailure::RuntimeFailure),
        ]));
        let cancelled = source.next();
        assert_send(&cancelled);
        drop(cancelled);
        // when
        let batches: Vec<_> = (0..5).map(|_| poll_ready(&mut source)).collect();
        // then
        assert_eq!(
            batches,
            [
                ChangeBatch::Events(vec!["index.html", "style.css"]),
                ChangeBatch::Overflow,
                ChangeBatch::Failed(SourceFailure::RuntimeFailure),
                ChangeBatch::Closed,
                ChangeBatch::Closed,
            ]
        );
    }
}
