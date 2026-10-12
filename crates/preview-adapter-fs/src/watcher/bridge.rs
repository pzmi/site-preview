//! The bridge from the synchronous `notify` callback to the asynchronous [`ChangeSource`]
//! (plan.md section 8.3, last paragraph; section 8.4; ADR 0038, ADR 0048).
//!
//! The callback runs on the native watcher's own thread and must never block it. It translates
//! each notification (dropping access-only and metadata-only changes, which would otherwise
//! fill the queue every time a page is served) and offers the result to a bounded
//! `tokio::sync::mpsc` channel with `try_send`. When the channel is full, or the operating
//! system reports that its own queue overflowed (the `Rescan` flag), it sets an overflow flag
//! and wakes the consumer; a runtime error is queued as a failure, and if that too finds the
//! channel full a failure flag is set beside the overflow flag.
//!
//! [`NotifyChangeSource::next`] reports in this order: `Closed` once closed; `Overflow` while
//! the overflow flag is set (clearing it); `Failed` while the failure flag is set; then the
//! queued messages, consecutive changes merged into one batch and a queued failure delivered
//! on its own. A message received while the overflow flag is set may postdate the loss, so it
//! is held back until the `Overflow` has been delivered: a loss is reported before any change
//! observed after it. All state lives in the source, the flags and the channel, never in the
//! future, so dropping a pending `next` loses nothing.

use core::fmt;
use core::future::poll_fn;
use core::num::NonZeroUsize;
use core::pin::pin;
use core::task::Poll;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use preview_application::{ChangeBatch, ChangeSource, SourceFailure};
use preview_domain::{ChangeEvent, ChangeKind, Classification, RelPath, classify};
use tokio::sync::Notify;
use tokio::sync::mpsc::{self, error::TryRecvError, error::TrySendError};

use super::display_rel;
use super::reconcile::{ReconcileHint, ReconcileReport};
use super::registration::{NotifyWatcher, RegistrationError, WatchFault};
use super::translate::{relative, translate};
use crate::capability::RootDir;

/// How many distinct reconcile hints the source keeps between two reconciles; past it, the
/// next reconcile covers every directory instead (one walk beats thousands of small ones).
const MAX_HINTS: usize = 256;

/// The channel capacity [`NotifyChangeSource::start`] uses, in queued notifications.
pub const DEFAULT_CAPACITY: NonZeroUsize = NonZeroUsize::new(1024).expect("non-zero");

/// A runtime failure of the native watcher (plan.md section 8.4), with the place it names
/// relative to the site root when it names one inside it. Never an absolute path or OS text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeError {
    /// The path the failure names, relative to the site root, if any.
    pub path: Option<RelPath>,
    /// Why.
    pub fault: WatchFault,
}

impl fmt::Display for RuntimeError {
    /// Pinned by `should_deliver_failed_when_notify_reports_error`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(
                f,
                "file watching failed at `{}`: {}",
                display_rel(path),
                self.fault
            ),
            None => write!(f, "file watching failed: {}", self.fault),
        }
    }
}

impl core::error::Error for RuntimeError {}

/// What [`NotifyChangeSource::start`] set up (plan.md section 8.1 step 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartReport {
    /// The number of directories watched.
    pub watching: usize,
    /// The directories that could not be walked or watched.
    pub errors: Vec<RegistrationError>,
    /// Set when the native watcher could not be created at all; nothing is watched then.
    pub unavailable: Option<WatchFault>,
}

impl StartReport {
    /// Whether every eligible directory is watched.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.errors.is_empty() && self.unavailable.is_none()
    }
}

enum Message {
    Events(Vec<ChangeEvent>),
    Failed(SourceFailure, Option<RuntimeError>),
}

#[derive(Default)]
struct Flags {
    overflow: AtomicBool,
    failure: AtomicBool,
    wake: Notify,
}

/// The `notify` event handler: the producer half, owned by the native watcher.
pub(crate) struct Bridge {
    sender: mpsc::Sender<Message>,
    flags: Arc<Flags>,
    root: PathBuf,
}

impl Bridge {
    /// Handles one notification without blocking. Pinned by
    /// `should_never_block_callback_when_full`.
    pub(crate) fn handle(&self, result: notify::Result<notify::Event>) {
        match result {
            Ok(event) if event.need_rescan() => self.flag(&self.flags.overflow),
            Ok(event) => {
                let changes: Vec<ChangeEvent> = translate(event, &self.root)
                    .into_iter()
                    .filter(|change| {
                        !matches!(
                            change.kind,
                            ChangeKind::AccessOnly | ChangeKind::MetadataOnly
                        )
                    })
                    .collect();
                if !changes.is_empty() {
                    self.offer(Message::Events(changes));
                }
            }
            Err(error) => {
                let detail = RuntimeError {
                    path: error
                        .paths
                        .iter()
                        .find_map(|path| relative(path, &self.root)),
                    fault: WatchFault::from(&error),
                };
                self.offer(Message::Failed(SourceFailure::RuntimeFailure, Some(detail)));
            }
        }
    }

    fn offer(&self, message: Message) {
        match self.sender.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(Message::Events(_))) => self.flag(&self.flags.overflow),
            Err(TrySendError::Full(Message::Failed(..))) => {
                self.flags.failure.store(true, Ordering::Release);
                self.flag(&self.flags.overflow);
            }
            // The source is gone; nobody is left to tell.
            Err(TrySendError::Closed(_)) => {}
        }
    }

    fn flag(&self, flag: &AtomicBool) {
        flag.store(true, Ordering::Release);
        self.flags.wake.notify_one();
    }
}

impl notify::EventHandler for Bridge {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        self.handle(event);
    }
}

/// The production [`ChangeSource`]: the native watcher of the eligible directories behind a
/// bounded channel. Dropping it, or [`close`](Self::close), removes every watch.
pub struct NotifyChangeSource {
    receiver: mpsc::Receiver<Message>,
    flags: Arc<Flags>,
    held: Option<Message>,
    watcher: Option<NotifyWatcher>,
    closed: bool,
    drain_limit: usize,
    last_failure: Option<RuntimeError>,
    /// What the changes delivered since the last reconcile ask to reconcile.
    hints: BTreeSet<ReconcileHint>,
}

impl fmt::Debug for NotifyChangeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotifyChangeSource")
            .field("watcher", &self.watcher)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl NotifyChangeSource {
    /// Starts watching every eligible directory of the site with [`DEFAULT_CAPACITY`].
    ///
    /// `display_root` is the resolved root `root` was opened from
    /// ([`crate::SiteRoot::display_path`]). Never fails: when a directory could not be
    /// watched, or the watcher could not be created, the report says which and why, and the
    /// first batch is `Failed(RegistrationFailed)` so the coordinator degrades while HTTP keeps
    /// serving. Without a watcher, `Closed` follows that batch.
    #[must_use]
    pub fn start(root: &RootDir, display_root: &Path) -> (Self, StartReport) {
        Self::start_with_capacity(root, display_root, DEFAULT_CAPACITY)
    }

    /// [`start`](Self::start) with a channel of `capacity` notifications.
    #[must_use]
    pub fn start_with_capacity(
        root: &RootDir,
        display_root: &Path,
        capacity: NonZeroUsize,
    ) -> (Self, StartReport) {
        let (bridge, mut source) = Self::channel(display_root, capacity);
        let report = match NotifyWatcher::start(root, display_root, bridge) {
            Ok((watcher, errors)) => {
                let report = StartReport {
                    watching: watcher.watched().len(),
                    errors,
                    unavailable: None,
                };
                source.watcher = Some(watcher);
                source.request_rescan_if_needed();
                report
            }
            Err(fault) => StartReport {
                watching: 0,
                errors: Vec::new(),
                unavailable: Some(fault),
            },
        };
        if !report.is_complete() {
            source.held = Some(Message::Failed(SourceFailure::RegistrationFailed, None));
        }
        (source, report)
    }

    /// A source and the bridge that feeds it, with no watcher yet.
    pub(crate) fn channel(display_root: &Path, capacity: NonZeroUsize) -> (Bridge, Self) {
        let (sender, receiver) = mpsc::channel(capacity.get());
        let flags = Arc::new(Flags::default());
        let bridge = Bridge {
            sender,
            flags: Arc::clone(&flags),
            root: display_root.to_path_buf(),
        };
        let source = Self {
            receiver,
            flags,
            held: None,
            watcher: None,
            closed: false,
            drain_limit: capacity.get(),
            last_failure: None,
            hints: BTreeSet::new(),
        };
        (bridge, source)
    }

    /// Stops watching: every watch is removed and every later `next` returns `Closed`.
    pub fn close(&mut self) {
        self.watcher = None;
        self.closed = true;
        self.held = None;
        self.receiver.close();
    }

    /// The detail of the most recent runtime failure delivered as a `Failed` batch, for the
    /// caller's warning. `None` for a registration failure (see [`StartReport`]) and for a
    /// failure that found the channel full.
    #[must_use]
    pub fn last_failure(&self) -> Option<&RuntimeError> {
        self.last_failure.as_ref()
    }

    /// The batch available now, if any. Never waits.
    fn ready(&mut self) -> Option<ChangeBatch<ChangeEvent>> {
        if self.closed {
            return Some(ChangeBatch::Closed);
        }
        if self.flags.overflow.swap(false, Ordering::AcqRel) {
            return Some(ChangeBatch::Overflow);
        }
        if self.flags.failure.swap(false, Ordering::AcqRel) {
            self.last_failure = None;
            return Some(ChangeBatch::Failed(SourceFailure::RuntimeFailure));
        }
        let mut events = Vec::new();
        for _ in 0..self.drain_limit {
            let message = match self.held.take() {
                Some(message) => message,
                None => match self.receiver.try_recv() {
                    Ok(message) => message,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) if events.is_empty() => {
                        self.closed = true;
                        return Some(ChangeBatch::Closed);
                    }
                    Err(TryRecvError::Disconnected) => break,
                },
            };
            if self.flags.overflow.load(Ordering::Acquire) {
                self.held = Some(message);
                break;
            }
            match message {
                Message::Events(changes) => events.extend(changes),
                Message::Failed(failure, detail) if events.is_empty() => {
                    self.last_failure = detail;
                    return Some(ChangeBatch::Failed(failure));
                }
                failed @ Message::Failed(..) => {
                    self.held = Some(failed);
                    break;
                }
            }
        }
        if !events.is_empty() {
            self.note_hints(&events);
            return Some(ChangeBatch::Events(events));
        }
        self.flags
            .overflow
            .swap(false, Ordering::AcqRel)
            .then_some(ChangeBatch::Overflow)
    }

    /// Records what `events` ask to reconcile (see [`hint_for`]), bounded by [`MAX_HINTS`].
    fn note_hints(&mut self, events: &[ChangeEvent]) {
        if self.hints.contains(&ReconcileHint::Full) {
            return;
        }
        self.hints.extend(events.iter().filter_map(hint_for));
        if self.hints.len() > MAX_HINTS || self.hints.contains(&ReconcileHint::Full) {
            self.hints = BTreeSet::from([ReconcileHint::Full]);
        }
    }

    /// Brings the watches in line with the directories the changes delivered since the last
    /// reconcile touched, and retries a bounded number of directories not watched yet
    /// (plan.md section 8.1, "New directories"; ADR 0050). `root` is the capability the
    /// source was started with.
    ///
    /// Pinned by `should_watch_new_nested_directory_when_created_then_file_edited`,
    /// `should_rewatch_directory_when_deleted_and_recreated` and
    /// `should_drop_stale_watch_when_directory_removed`.
    pub fn reconcile_changed(&mut self, root: &RootDir) -> ReconcileReport {
        let hints = core::mem::take(&mut self.hints);
        self.reconcile_with(root, &hints)
    }

    /// Walks every eligible directory and brings the watches in line with it, after a loss or
    /// an event too coarse to attribute. Pinned by
    /// `should_rewatch_replaced_directory_when_full_reconcile_follows_a_loss`.
    pub fn reconcile_full(&mut self, root: &RootDir) -> ReconcileReport {
        self.hints.clear();
        self.reconcile_with(root, &BTreeSet::from([ReconcileHint::Full]))
    }

    fn reconcile_with(
        &mut self,
        root: &RootDir,
        hints: &BTreeSet<ReconcileHint>,
    ) -> ReconcileReport {
        let report = match &mut self.watcher {
            Some(watcher) => watcher.reconcile(root, hints),
            None => ReconcileReport::default(),
        };
        self.request_rescan_if_needed();
        report
    }

    /// When a registration stopped at its relisting cap, reports a loss: the next batch is
    /// `Overflow`, whose full reconcile finds the directories left unwatched (and whose full
    /// reload covers changes in them that were never notified). Pinned by
    /// `should_hand_the_rest_to_a_full_reconcile_when_relisting_passes_the_cap`.
    fn request_rescan_if_needed(&mut self) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        if core::mem::take(&mut watcher.rescan_needed) {
            self.flags.overflow.store(true, Ordering::Release);
            self.flags.wake.notify_one();
        }
    }

    /// The native watcher, for a test that sets its hooks after start-up.
    #[cfg(test)]
    pub(crate) fn watcher_mut(&mut self) -> Option<&mut NotifyWatcher> {
        self.watcher.as_mut()
    }

    /// The directories currently watched, root-relative; empty once closed.
    #[cfg(test)]
    pub(crate) fn watched(&self) -> Vec<RelPath> {
        self.watcher
            .as_ref()
            .map(|watcher| watcher.watched().iter().cloned().collect())
            .unwrap_or_default()
    }

    async fn next_batch(&mut self) -> ChangeBatch<ChangeEvent> {
        loop {
            if let Some(batch) = self.ready() {
                return batch;
            }
            // A wake stored before this point is kept as a permit, so none is lost.
            let mut woken = pin!(self.flags.wake.notified());
            let receiver = &mut self.receiver;
            let received = poll_fn(|cx| {
                if woken.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
                receiver.poll_recv(cx).map(Some)
            })
            .await;
            if let Some(Some(message)) = received {
                self.held = Some(message);
            }
        }
    }
}

/// The reconcile a delivered change asks for: none when the domain says the directory
/// structure cannot have changed (a file edit, an ignored path); the subtree when the OS did
/// not attribute the change; the entry itself when it was created, removed or renamed; every
/// directory when the change names the root.
fn hint_for(event: &ChangeEvent) -> Option<ReconcileHint> {
    let classification = classify(event);
    if !classification.requests_reconcile() {
        return None;
    }
    Some(match classification {
        _ if event.path.is_root() => ReconcileHint::Full,
        Classification::Rescan => ReconcileHint::Subtree(event.path.clone()),
        _ => ReconcileHint::Entry(event.path.clone()),
    })
}

impl ChangeSource for NotifyChangeSource {
    type Event = ChangeEvent;

    /// See the port's contract; this module's documentation gives the order. Cancellation
    /// safe: pinned by `should_lose_nothing_when_a_pending_next_is_dropped`.
    fn next(&mut self) -> impl Future<Output = ChangeBatch<ChangeEvent>> + Send {
        self.next_batch()
    }
}

#[cfg(test)]
mod tests {
    use core::task::{Context, Waker};
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;
    use std::thread;
    use std::time::Duration;

    use notify::event::{AccessKind, CreateKind, DataChange, Flag, MetadataKind, ModifyKind};
    use notify::{Event, EventKind};
    use preview_domain::Component;
    use preview_testkit::poll::wait_until;

    use super::*;

    const ROOT: &str = "/srv/site";

    fn rel(text: &str) -> RelPath {
        RelPath::new(
            text.split('/')
                .filter(|part| !part.is_empty())
                .map(|part| Component::new(part).expect("valid component"))
                .collect(),
        )
    }

    fn source(capacity: usize) -> (Bridge, NotifyChangeSource) {
        NotifyChangeSource::channel(
            Path::new(ROOT),
            NonZeroUsize::new(capacity).expect("non-zero capacity"),
        )
    }

    fn written(name: &str) -> notify::Result<Event> {
        Ok(
            Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
                .add_path(Path::new(ROOT).join(name)),
        )
    }

    fn modified(name: &str) -> ChangeEvent {
        ChangeEvent {
            kind: ChangeKind::Modify,
            path: rel(name),
        }
    }

    fn poll_once(source: &mut NotifyChangeSource, waker: &Waker) -> Poll<ChangeBatch<ChangeEvent>> {
        let future = pin!(source.next());
        future.poll(&mut Context::from_waker(waker))
    }

    fn ready(source: &mut NotifyChangeSource) -> ChangeBatch<ChangeEvent> {
        match poll_once(source, Waker::noop()) {
            Poll::Ready(batch) => batch,
            Poll::Pending => panic!("expected a batch to be ready"),
        }
    }

    struct Wakes(AtomicUsize);

    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn should_signal_overflow_when_channel_full() {
        // given
        let (bridge, mut source) = source(1);

        // when
        for n in 0..100 {
            bridge.handle(written(&format!("page-{n}.html")));
        }

        // then
        assert_eq!(ready(&mut source), ChangeBatch::Overflow);
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Events(vec![modified("page-0.html")])
        );
        assert!(poll_once(&mut source, Waker::noop()).is_pending());
    }

    #[test]
    fn should_never_block_callback_when_full() {
        // given
        let (bridge, mut source) = source(1);
        bridge.handle(written("fills.html"));
        let returned = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&returned);

        // when
        let feeder = thread::spawn(move || {
            for n in 0..1000 {
                bridge.handle(written(&format!("page-{n}.html")));
            }
            done.store(true, Ordering::SeqCst);
        });

        // then
        // Bounded wait on another thread's progress: a blocking callback never returns, and
        // the deadline only limits how long that failing test hangs.
        wait_until(Duration::from_secs(10), "the callback returned", || {
            returned.load(Ordering::SeqCst)
        })
        .expect("the callback never blocks on a full channel");
        feeder.join().expect("feeder thread");
        assert_eq!(ready(&mut source), ChangeBatch::Overflow);
    }

    #[test]
    fn should_deliver_failed_when_notify_reports_error() {
        // given
        let (bridge, mut source) = source(8);
        let error = notify::Error::new(notify::ErrorKind::MaxFilesWatch)
            .add_path(Path::new(ROOT).join("docs"));

        // when
        bridge.handle(Err(error));
        bridge.handle(written("after.html"));

        // then
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Failed(SourceFailure::RuntimeFailure)
        );
        let detail = source.last_failure().expect("failure detail kept");
        assert_eq!(detail.path, Some(rel("docs")));
        assert_eq!(detail.fault, WatchFault::WatchLimitReached);
        assert!(
            detail
                .to_string()
                .starts_with("file watching failed at `docs`: ")
        );
        assert!(!detail.to_string().contains(ROOT));
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Events(vec![modified("after.html")])
        );
    }

    #[test]
    fn should_report_failure_after_overflow_when_failure_finds_channel_full() {
        // given
        let (bridge, mut source) = source(1);
        bridge.handle(written("fills.html"));

        // when
        bridge.handle(Err(notify::Error::generic("lost")));

        // then
        assert_eq!(ready(&mut source), ChangeBatch::Overflow);
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Failed(SourceFailure::RuntimeFailure)
        );
        assert_eq!(source.last_failure(), None);
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Events(vec![modified("fills.html")])
        );
    }

    #[test]
    fn should_merge_queued_changes_when_several_notifications_wait() {
        // given
        let (bridge, mut source) = source(8);
        bridge.handle(written("a.html"));
        bridge.handle(written("b.css"));
        bridge.handle(Err(notify::Error::generic("broken")));
        bridge.handle(written("c.js"));

        // when
        let batches: Vec<_> = (0..3).map(|_| ready(&mut source)).collect();

        // then
        assert_eq!(
            batches,
            [
                ChangeBatch::Events(vec![modified("a.html"), modified("b.css")]),
                ChangeBatch::Failed(SourceFailure::RuntimeFailure),
                ChangeBatch::Events(vec![modified("c.js")]),
            ]
        );
    }

    #[test]
    fn should_report_overflow_before_later_events_when_loss_happens_mid_stream() {
        // given
        let (bridge, mut source) = source(2);
        bridge.handle(written("one.html"));
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Events(vec![modified("one.html")])
        );
        bridge.handle(written("two.html"));
        bridge.handle(written("three.html"));

        // when
        bridge.handle(written("lost.html"));
        bridge.handle(written("later.html"));

        // then
        let mut rest = Vec::new();
        while let Poll::Ready(batch) = poll_once(&mut source, Waker::noop()) {
            rest.push(batch);
        }
        bridge.handle(written("after-drain.html"));
        rest.push(ready(&mut source));
        assert_eq!(
            rest,
            [
                ChangeBatch::Overflow,
                ChangeBatch::Events(vec![modified("two.html"), modified("three.html")]),
                ChangeBatch::Events(vec![modified("after-drain.html")]),
            ]
        );
    }

    #[test]
    fn should_hold_back_message_received_after_loss_when_draining() {
        // given
        let (bridge, mut source) = source(4);
        bridge.handle(written("before.html"));
        bridge.flag(&bridge.flags.overflow);
        bridge.handle(written("after.html"));

        // when
        let batches: Vec<_> = (0..2).map(|_| ready(&mut source)).collect();

        // then
        assert_eq!(
            batches,
            [
                ChangeBatch::Overflow,
                ChangeBatch::Events(vec![modified("before.html"), modified("after.html")]),
            ]
        );
    }

    #[test]
    fn should_signal_overflow_when_os_reports_rescan() {
        // given
        let (bridge, mut source) = source(8);

        // when
        bridge.handle(Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan)));

        // then
        assert_eq!(ready(&mut source), ChangeBatch::Overflow);
        assert!(poll_once(&mut source, Waker::noop()).is_pending());
    }

    #[test]
    fn should_queue_nothing_when_change_is_access_or_metadata_only() {
        // given
        let (bridge, mut source) = source(1);

        // when
        for _ in 0..10 {
            bridge.handle(Ok(Event::new(EventKind::Access(AccessKind::Any))
                .add_path(Path::new(ROOT).join("index.html"))));
            bridge.handle(Ok(Event::new(EventKind::Modify(ModifyKind::Metadata(
                MetadataKind::Any,
            )))
            .add_path(Path::new(ROOT).join("index.html"))));
        }
        bridge.handle(Ok(Event::new(EventKind::Create(CreateKind::File))
            .add_path(PathBuf::from("/elsewhere/x.html"))));

        // then
        assert!(poll_once(&mut source, Waker::noop()).is_pending());
    }

    #[test]
    fn should_wake_waiting_consumer_when_change_arrives() {
        // given
        let (bridge, mut source) = source(8);
        let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut future = pin!(source.next());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );

        // when
        bridge.handle(written("a.html"));

        // then
        assert!(wakes.0.load(Ordering::SeqCst) > 0);
        assert_eq!(
            future.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(ChangeBatch::Events(vec![modified("a.html")]))
        );
    }

    #[test]
    fn should_wake_waiting_consumer_when_overflow_is_flagged() {
        // given
        let (bridge, mut source) = source(8);
        let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut future = pin!(source.next());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );

        // when
        bridge.handle(Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan)));

        // then
        assert!(wakes.0.load(Ordering::SeqCst) > 0);
        assert_eq!(
            future.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(ChangeBatch::Overflow)
        );
    }

    #[test]
    fn should_lose_nothing_when_a_pending_next_is_dropped() {
        // given
        let (bridge, mut source) = source(8);
        assert!(poll_once(&mut source, Waker::noop()).is_pending());
        bridge.handle(written("a.html"));
        bridge.flag(&bridge.flags.overflow);

        // when
        drop(source.next());

        // then
        assert_eq!(ready(&mut source), ChangeBatch::Overflow);
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Events(vec![modified("a.html")])
        );
    }

    #[test]
    fn should_keep_received_message_when_next_is_dropped_after_wake() {
        // given
        let (bridge, mut source) = source(8);
        let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        {
            let mut future = pin!(source.next());
            assert!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            bridge.handle(written("a.html"));
        }

        // when
        let batch = ready(&mut source);

        // then
        assert_eq!(batch, ChangeBatch::Events(vec![modified("a.html")]));
    }

    #[test]
    fn should_return_closed_for_ever_when_bridge_is_gone() {
        // given
        let (bridge, mut source) = source(8);
        bridge.handle(written("a.html"));

        // when
        drop(bridge);

        // then
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Events(vec![modified("a.html")])
        );
        assert_eq!(ready(&mut source), ChangeBatch::Closed);
        assert_eq!(ready(&mut source), ChangeBatch::Closed);
    }

    #[test]
    fn should_return_closed_for_ever_when_closed_with_changes_queued() {
        // given
        let (bridge, mut source) = source(8);
        bridge.handle(written("a.html"));

        // when
        source.close();
        bridge.handle(written("b.html"));

        // then
        assert_eq!(ready(&mut source), ChangeBatch::Closed);
        assert_eq!(ready(&mut source), ChangeBatch::Closed);
    }

    #[test]
    fn should_deliver_registration_failure_then_closed_when_watcher_is_unavailable() {
        // given
        let (bridge, mut source) = source(8);
        source.held = Some(Message::Failed(SourceFailure::RegistrationFailed, None));

        // when
        drop(bridge);

        // then
        assert_eq!(
            ready(&mut source),
            ChangeBatch::Failed(SourceFailure::RegistrationFailed)
        );
        assert_eq!(ready(&mut source), ChangeBatch::Closed);
    }

    #[test]
    fn should_note_reconcile_hints_when_changes_may_alter_directories() {
        // given
        let (bridge, mut source) = source(16);
        let raw =
            |kind: EventKind, name: &str| Ok(Event::new(kind).add_path(Path::new(ROOT).join(name)));

        // when
        bridge.handle(raw(EventKind::Create(CreateKind::Folder), "docs"));
        bridge.handle(raw(EventKind::Create(CreateKind::File), "new.css"));
        bridge.handle(raw(
            EventKind::Remove(notify::event::RemoveKind::Any),
            "old",
        ));
        bridge.handle(raw(EventKind::Any, "blog/x"));
        bridge.handle(written("edited.html"));
        bridge.handle(raw(EventKind::Create(CreateKind::Folder), ".git/objects"));
        bridge.handle(raw(EventKind::Create(CreateKind::Folder), "node_modules"));
        bridge.handle(raw(EventKind::Create(CreateKind::File), ".index.html.swp"));
        let _ = ready(&mut source);

        // then: edits and ignored names ask for nothing
        assert_eq!(
            source.hints,
            BTreeSet::from([
                ReconcileHint::Entry(rel("docs")),
                ReconcileHint::Entry(rel("new.css")),
                ReconcileHint::Entry(rel("old")),
                ReconcileHint::Subtree(rel("blog/x")),
            ])
        );
    }

    #[test]
    fn should_widen_to_full_when_hints_overflow_or_name_the_root() {
        // given
        let (bridge, mut many) = source(MAX_HINTS * 2);
        let (root_bridge, mut rooted) = source(4);

        // when
        for n in 0..=MAX_HINTS {
            bridge.handle(Ok(Event::new(EventKind::Create(CreateKind::Folder))
                .add_path(Path::new(ROOT).join(format!("d{n}")))));
        }
        let _ = ready(&mut many);
        root_bridge.handle(Ok(Event::new(EventKind::Any).add_path(PathBuf::from(ROOT))));
        let _ = ready(&mut rooted);

        // then
        assert_eq!(many.hints, BTreeSet::from([ReconcileHint::Full]));
        assert_eq!(rooted.hints, BTreeSet::from([ReconcileHint::Full]));
    }

    #[test]
    fn should_report_incomplete_when_reconciling_without_a_watcher() {
        // given
        let (_bridge, mut source) = source(4);
        let base = tempfile::tempdir().expect("temp dir");
        let site = crate::open_site_root(base.path()).expect("open root");

        // when
        let changed = source.reconcile_changed(&site.dir);
        let full = source.reconcile_full(&site.dir);

        // then
        assert_eq!(changed, ReconcileReport::default());
        assert!(!full.complete);
    }

    #[test]
    fn should_omit_path_when_runtime_error_names_none() {
        // given
        let error = RuntimeError {
            path: None,
            fault: WatchFault::Unavailable,
        };

        // when
        let text = error.to_string();

        // then
        assert_eq!(
            text,
            "file watching failed: the native file watcher is unavailable"
        );
    }
}
