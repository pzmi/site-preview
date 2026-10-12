//! A [`ChangeSource`] fed by a test through a handle.

use core::num::NonZeroUsize;
use core::task::{Poll, Waker};
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use preview_application::{ChangeBatch, ChangeSource, SourceFailure};
use preview_domain::eligibility::{self, ComponentKind, Eligibility};
use preview_domain::{ChangeEvent, ChangeKind, Component, RelPath};

use crate::contracts::{ChangeSourceFactory, WatchTree};

struct Shared<E> {
    queue: VecDeque<ChangeBatch<E>>,
    closed: bool,
    waker: Option<Waker>,
}

fn lock<E>(shared: &Mutex<Shared<E>>) -> MutexGuard<'_, Shared<E>> {
    shared
        .lock()
        .expect("a scripted-source lock is never poisoned")
}

/// The [`ChangeSource`] half: hand it to the code under test.
///
/// Batches are taken from the script only when the future from `next` is polled, so dropping
/// an unpolled or pending `next` loses nothing (the cancellation rule). Batches come out in
/// the order they were scripted, `Closed` comes after the last of them, and `Closed` then
/// repeats for ever.
pub struct ScriptedChangeSource<E = ChangeEvent> {
    shared: Arc<Mutex<Shared<E>>>,
}

/// The producer half: feeds the source. Dropping it closes the source, as a vanished producer
/// must (the `Closed` rule).
pub struct ScriptedHandle<E = ChangeEvent> {
    shared: Arc<Mutex<Shared<E>>>,
}

impl<E> ScriptedChangeSource<E> {
    /// A source with an empty script, and the handle that feeds it.
    #[must_use]
    pub fn new() -> (Self, ScriptedHandle<E>) {
        let shared = Arc::new(Mutex::new(Shared {
            queue: VecDeque::new(),
            closed: false,
            waker: None,
        }));
        (
            Self {
                shared: Arc::clone(&shared),
            },
            ScriptedHandle { shared },
        )
    }
}

impl<E> ScriptedHandle<E> {
    fn enqueue(&self, batch: ChangeBatch<E>) {
        let mut state = lock(&self.shared);
        if state.closed {
            return;
        }
        state.queue.push_back(batch);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    /// Queues one batch of changes. An empty `events` queues nothing: a batch is never empty.
    /// Ignored once the source is closed.
    pub fn push(&self, events: Vec<E>) {
        if !events.is_empty() {
            self.enqueue(ChangeBatch::Events(events));
        }
    }

    /// Queues a loss report; it is delivered before anything pushed after it.
    pub fn overflow(&self) {
        self.enqueue(ChangeBatch::Overflow);
    }

    /// Queues a failure report. The source stays open.
    pub fn fail(&self, failure: SourceFailure) {
        self.enqueue(ChangeBatch::Failed(failure));
    }

    /// Stops the source: what is already queued is still delivered, then `Closed` for ever.
    pub fn close(&self) {
        let mut state = lock(&self.shared);
        state.closed = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}

impl<E> Drop for ScriptedHandle<E> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<E: Send> ChangeSource for ScriptedChangeSource<E> {
    type Event = E;

    fn next(&mut self) -> impl Future<Output = ChangeBatch<E>> + Send {
        let shared = &self.shared;
        core::future::poll_fn(move |cx| {
            let mut state = lock(shared);
            if let Some(batch) = state.queue.pop_front() {
                return Poll::Ready(batch);
            }
            if state.closed {
                return Poll::Ready(ChangeBatch::Closed);
            }
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        })
    }
}

/// [`ScriptedChangeSource`] as a [`ChangeSourceFactory`]: scripts the batches the native
/// watcher would deliver for what the contract harness does to the site.
///
/// It models the watcher, not just a queue: a write inside a directory the eligibility
/// predicate excludes is never delivered (the watcher never registers that directory), a
/// file's first write is a `Create` and later ones `Modify`, an unwatchable directory queues
/// `Failed(RegistrationFailed)` at start, and a burst larger than the capacity keeps the first
/// `capacity` changes and reports the rest as one `Overflow`. Stopping drops the handle.
pub struct ScriptedSite {
    handle: Option<ScriptedHandle<ChangeEvent>>,
    written: BTreeSet<String>,
    capacity: usize,
}

impl ScriptedSite {
    /// The change the native watcher would report for a write of `path`, if any.
    fn change(&mut self, path: &str) -> Option<ChangeEvent> {
        let components: Vec<Component> = path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| Component::new(part).expect("a harness path is valid"))
            .collect();
        let (_, parents) = components.split_last()?;
        if eligibility::components(parents, ComponentKind::Dir) != Eligibility::Eligible {
            return None;
        }
        let kind = if self.written.insert(path.to_owned()) {
            ChangeKind::Create
        } else {
            ChangeKind::Modify
        };
        Some(ChangeEvent {
            kind,
            path: RelPath::new(components),
        })
    }

    fn push(&self, change: ChangeEvent) {
        if let Some(handle) = &self.handle {
            handle.push(vec![change]);
        }
    }
}

impl ChangeSourceFactory for ScriptedSite {
    type Source = ScriptedChangeSource<ChangeEvent>;
    type Site = Self;

    const DEADLINE: Duration = Duration::ZERO;

    fn start(tree: &WatchTree, capacity: NonZeroUsize) -> (Self::Source, Self) {
        let (source, handle) = ScriptedChangeSource::new();
        if !tree.unwatchable_dirs().is_empty() {
            handle.fail(SourceFailure::RegistrationFailed);
        }
        let site = Self {
            handle: Some(handle),
            written: tree.files().iter().cloned().collect(),
            capacity: capacity.get(),
        };
        (source, site)
    }

    fn write(site: &mut Self, path: &str) {
        if let Some(change) = site.change(path) {
            site.push(change);
        }
    }

    fn burst(site: &mut Self, paths: &[String]) {
        let mut kept = 0;
        let mut lost = false;
        for path in paths {
            let Some(change) = site.change(path) else {
                continue;
            };
            if kept < site.capacity {
                site.push(change);
                kept += 1;
            } else {
                lost = true;
            }
        }
        if lost && let Some(handle) = &site.handle {
            handle.overflow();
        }
    }

    fn stop(site: &mut Self, _source: &mut Self::Source) {
        site.handle = None;
    }
}

#[cfg(test)]
mod tests {
    use core::pin::pin;
    use core::task::Context;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Wake;

    use super::*;

    type Source = ScriptedChangeSource<&'static str>;

    fn poll_once<S: ChangeSource>(source: &mut S) -> Poll<ChangeBatch<S::Event>> {
        let future = pin!(source.next());
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    fn ready<S: ChangeSource>(source: &mut S) -> ChangeBatch<S::Event> {
        match poll_once(source) {
            Poll::Ready(batch) => batch,
            Poll::Pending => panic!("expected a batch to be ready"),
        }
    }

    struct Flag(AtomicBool);

    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn should_deliver_in_scripted_order_when_batches_are_queued() {
        // given
        let (mut source, handle) = Source::new();
        handle.push(vec!["a.html", "b.css"]);
        handle.overflow();
        handle.fail(SourceFailure::RuntimeFailure);
        handle.push(vec!["c.js"]);
        // when
        let batches: Vec<_> = (0..4).map(|_| ready(&mut source)).collect();
        // then
        assert_eq!(
            batches,
            [
                ChangeBatch::Events(vec!["a.html", "b.css"]),
                ChangeBatch::Overflow,
                ChangeBatch::Failed(SourceFailure::RuntimeFailure),
                ChangeBatch::Events(vec!["c.js"]),
            ]
        );
    }

    #[test]
    fn should_report_overflow_before_later_events_when_overflow_precedes_push() {
        // given
        let (mut source, handle) = Source::new();
        handle.overflow();
        handle.push(vec!["late.html"]);
        // when
        let first = ready(&mut source);
        let second = ready(&mut source);
        // then
        assert_eq!(first, ChangeBatch::Overflow);
        assert_eq!(second, ChangeBatch::Events(vec!["late.html"]));
    }

    #[test]
    fn should_lose_nothing_when_a_next_future_is_dropped_unpolled() {
        // given
        let (mut source, handle) = Source::new();
        handle.push(vec!["a.html"]);
        handle.overflow();
        // when
        drop(source.next());
        let first = ready(&mut source);
        let second = ready(&mut source);
        // then
        assert_eq!(first, ChangeBatch::Events(vec!["a.html"]));
        assert_eq!(second, ChangeBatch::Overflow);
    }

    #[test]
    fn should_lose_nothing_when_a_pending_next_future_is_dropped() {
        // given
        let (mut source, handle) = Source::new();
        {
            let pending = pin!(source.next());
            assert!(
                pending
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        // when
        handle.push(vec!["a.html"]);
        // then
        assert_eq!(ready(&mut source), ChangeBatch::Events(vec!["a.html"]));
    }

    #[test]
    fn should_wake_the_waiting_task_when_a_batch_is_pushed() {
        // given
        let (mut source, handle) = Source::new();
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));
        let mut future = pin!(source.next());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        // when
        handle.push(vec!["a.html"]);
        // then
        assert!(flag.0.load(Ordering::SeqCst));
        assert_eq!(
            future.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(ChangeBatch::Events(vec!["a.html"]))
        );
    }

    #[test]
    fn should_deliver_queued_batches_then_stay_closed_when_closed() {
        // given
        let (mut source, handle) = Source::new();
        handle.push(vec!["a.html"]);
        handle.close();
        handle.push(vec!["ignored.html"]);
        // when
        let batches: Vec<_> = (0..4).map(|_| ready(&mut source)).collect();
        // then
        assert_eq!(
            batches,
            [
                ChangeBatch::Events(vec!["a.html"]),
                ChangeBatch::Closed,
                ChangeBatch::Closed,
                ChangeBatch::Closed,
            ]
        );
    }

    #[test]
    fn should_close_when_the_handle_is_dropped() {
        // given
        let (mut source, handle) = Source::new();
        handle.overflow();
        // when
        drop(handle);
        // then
        assert_eq!(ready(&mut source), ChangeBatch::Overflow);
        assert_eq!(ready(&mut source), ChangeBatch::Closed);
    }

    #[test]
    fn should_wake_with_closed_when_closed_while_waiting() {
        // given
        let (mut source, handle) = Source::new();
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));
        let mut future = pin!(source.next());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        // when
        handle.close();
        // then
        assert!(flag.0.load(Ordering::SeqCst));
        assert_eq!(
            future.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(ChangeBatch::Closed)
        );
    }

    #[test]
    fn should_queue_nothing_when_pushed_events_are_empty() {
        // given
        let (mut source, handle) = Source::new();
        // when
        handle.push(Vec::new());
        // then
        assert!(poll_once(&mut source).is_pending());
    }

    #[test]
    fn should_keep_delivering_when_a_failure_was_reported() {
        // given
        let (mut source, handle) = Source::new();
        handle.fail(SourceFailure::RegistrationFailed);
        handle.push(vec!["a.html"]);
        // when
        let first = ready(&mut source);
        let second = ready(&mut source);
        // then
        assert_eq!(
            first,
            ChangeBatch::Failed(SourceFailure::RegistrationFailed)
        );
        assert_eq!(second, ChangeBatch::Events(vec!["a.html"]));
    }

    #[test]
    fn should_carry_domain_change_events_when_default_event_type_is_used() {
        // given
        let (mut source, handle) = ScriptedChangeSource::<ChangeEvent>::new();
        let event = ChangeEvent {
            kind: preview_domain::ChangeKind::Modify,
            path: preview_domain::RelPath::new(vec![
                preview_domain::Component::new("index.html").expect("valid component"),
            ]),
        };
        // when
        handle.push(vec![event.clone()]);
        // then
        assert_eq!(ready(&mut source), ChangeBatch::Events(vec![event]));
    }
}
