//! The watcher pipeline: one owned task between the change source and the reload state
//! (plan.md sections 8.3, 8.4 and 9; ADR 0040, ADR 0049).
//!
//! [`WatchPipeline::run`] waits on three things at once: cancellation, the debounce deadline
//! (only when the coordinator has one: nothing pending arms no timer) and the source's next
//! batch (only while the source is open: a closed source is never polled again, since it would
//! answer `Closed` at once for ever). Every batch goes to [`ChangeCoordinator::receive`] and
//! every deadline to [`ChangeCoordinator::settle`], so the pipeline makes no decision of its
//! own; it reconciles the watch set when the coordinator asks, turns degradations into
//! warnings and closes the source when it stops.

use core::future::{Future, poll_fn};
use core::pin::{Pin, pin};
use core::task::Poll;
use std::sync::Arc;

use preview_adapter_http::WarningSink;
use preview_application::{
    ChangeBatch, ChangeCoordinator, ChangeSource, Clock, Received, SourceFailure,
};
use preview_domain::{ChangeEvent, DegradeReason, DomainEvent, Tick};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep};

/// How much of the watch set a reconcile covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconcileScope {
    /// The batch just received changed the directory structure (or may have).
    Changed,
    /// Changes were lost or could not be attributed: every eligible directory.
    Full,
}

/// What a reconcile changed in the watcher's health, in words composition can report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reconciled {
    /// `Some(dirs)` when every eligible directory is watched afterwards, which the pipeline
    /// reports as readiness (ADR 0040); `None` otherwise.
    pub ready: Option<usize>,
    /// One line per directory that newly cannot be walked or watched (relative paths only,
    /// at most ten named, the rest counted), each warned as `watch-registration-failed`.
    pub problems: Vec<String>,
}

/// The change source as the pipeline drives it: the port plus the controls only the watcher
/// adapter can carry out. Bootstrap implements it for the filesystem adapter's source, so the
/// pipeline never names the adapter.
pub(crate) trait WatchControl: ChangeSource<Event = ChangeEvent> + 'static {
    /// Re-registers the watch set after a directory change or a loss (plan.md section 8.1,
    /// "New directories"; ADR 0050). The source itself knows which directories the changes
    /// it delivered touched, so `Changed` carries no paths.
    fn reconcile(&mut self, scope: ReconcileScope) -> Reconciled;

    /// The detail of the most recent runtime failure, for its warning: a root-relative path
    /// and a category, never an absolute path or OS text.
    fn failure_detail(&self) -> Option<String>;

    /// Stops watching for good; every later batch is `Closed`.
    fn close(&mut self);
}

/// The production [`Clock`]: milliseconds since the pipeline started, on Tokio's monotonic
/// clock, so a test that pauses Tokio's time controls it too.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TokioClock {
    origin: Instant,
}

impl TokioClock {
    /// A clock reading zero now.
    pub(crate) fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    /// Pinned by `should_read_elapsed_millis_when_tokio_time_advances`.
    fn now(&self) -> Tick {
        let millis = self.origin.elapsed().as_millis();
        Tick::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
    }
}

/// The warning a domain event deserves on stderr, if any: its stable code and message.
///
/// A registration failure is reported where the registration happened (start-up or a
/// reconcile), directory by directory, so its degradation adds nothing here. Pinned by
/// `should_warn_with_stable_code_when_watcher_degrades`.
pub(crate) fn warning_for(
    event: &DomainEvent,
    detail: Option<&str>,
) -> Option<(&'static str, String)> {
    let DomainEvent::WatcherDegraded { reason } = event else {
        return None;
    };
    match reason {
        DegradeReason::RegistrationFailed => None,
        DegradeReason::RuntimeFailure => Some((
            "watch-failed",
            format!(
                "{}; live reload may miss changes until site-preview is restarted, pages are \
                 still served",
                detail.unwrap_or("file watching failed")
            ),
        )),
        DegradeReason::RevisionExhausted => Some((
            "reload-exhausted",
            "the revision counter is exhausted; live reload has stopped until site-preview is \
             restarted, pages are still served"
                .to_owned(),
        )),
    }
}

/// The pipeline's state: the source it drains, the coordinator it feeds and where warnings go.
pub(crate) struct WatchPipeline<S, C> {
    source: S,
    coordinator: ChangeCoordinator,
    clock: C,
    warnings: Arc<dyn WarningSink>,
    /// Whether a full reconcile ran since the last published revision, so a rescan the
    /// revision asks for is not repeated.
    reconciled_fully: bool,
}

enum Step {
    Cancelled,
    Deadline,
    Batch(ChangeBatch<ChangeEvent>),
}

impl<S: WatchControl, C: Clock> WatchPipeline<S, C> {
    pub(crate) fn new(
        source: S,
        coordinator: ChangeCoordinator,
        clock: C,
        warnings: Arc<dyn WarningSink>,
    ) -> Self {
        Self {
            source,
            coordinator,
            clock,
            warnings,
            reconciled_fully: false,
        }
    }

    /// Runs until `cancel` fires or its sender is dropped, then closes the source and returns.
    ///
    /// Pinned by `should_publish_one_revision_when_events_burst_within_window`,
    /// `should_publish_after_overflow_when_channel_overflowed`,
    /// `should_stop_task_when_cancelled`, `should_arm_no_timer_when_nothing_is_pending` and
    /// `should_settle_pending_and_stop_polling_when_source_closes`.
    pub(crate) async fn run(mut self, mut cancel: oneshot::Receiver<()>) {
        let mut open = true;
        loop {
            match self.next_step(&mut cancel, open).await {
                Step::Cancelled => break,
                Step::Deadline => self.settle(),
                Step::Batch(batch) => open = self.receive(batch),
            }
        }
        self.source.close();
    }

    /// Waits for the first of cancellation, the deadline and a batch, polled in that order:
    /// a source that is always ready can neither hold off a stop nor postpone a due revision.
    /// Both inner futures are rebuilt on every call; dropping the loser loses nothing (the
    /// `ChangeSource` contract is cancellation safe).
    async fn next_step(&mut self, cancel: &mut oneshot::Receiver<()>, open: bool) -> Step {
        // The timer is armed from the coordinator's deadline only; `remaining_from` rounds the
        // wait so it never ends before the deadline, so `settle` never finds it in the future
        // and the loop cannot spin.
        let wait = self
            .coordinator
            .deadline()
            .map(|deadline| deadline.remaining_from(self.clock.now()));
        let mut timer = pin!(wait.map(sleep));
        let mut batch = pin!(open.then(|| self.source.next()));
        poll_fn(|cx| {
            if Pin::new(&mut *cancel).poll(cx).is_ready() {
                return Poll::Ready(Step::Cancelled);
            }
            if let Some(timer) = timer.as_mut().as_pin_mut()
                && timer.poll(cx).is_ready()
            {
                return Poll::Ready(Step::Deadline);
            }
            if let Some(next) = batch.as_mut().as_pin_mut()
                && let Poll::Ready(received) = next.poll(cx)
            {
                return Poll::Ready(Step::Batch(received));
            }
            Poll::Pending
        })
        .await
    }

    /// Hands one batch to the coordinator; false once the source has closed.
    fn receive(&mut self, batch: ChangeBatch<ChangeEvent>) -> bool {
        let scope = if matches!(batch, ChangeBatch::Overflow) {
            ReconcileScope::Full
        } else {
            ReconcileScope::Changed
        };
        match self.coordinator.receive(batch, self.clock.now()) {
            Received::Recorded(outcome) => {
                self.report(&outcome.events);
                // At once, not at the deadline: a new directory is watched before the edits
                // that follow its creation, and the revision still waits for the window.
                if outcome.needs_reconcile {
                    self.reconcile(scope);
                }
                true
            }
            Received::Closed => false,
        }
    }

    fn settle(&mut self) {
        let outcome = self.coordinator.settle(self.clock.now());
        self.report(&outcome.events);
        if let Some(settled) = outcome.settled {
            if settled.rescan && !self.reconciled_fully {
                self.reconcile(ReconcileScope::Full);
            }
            self.reconciled_fully = false;
        }
    }

    /// Reconciles the watch set and reports what changed: each directory that newly cannot
    /// be watched is warned about and degrades the watcher; a complete set is readiness, so a
    /// degraded watcher recovers. Pinned by
    /// `should_warn_and_degrade_when_reconcile_cannot_watch_a_directory` and
    /// `should_recover_when_reconcile_completes_the_watch_set`.
    fn reconcile(&mut self, scope: ReconcileScope) {
        self.reconciled_fully |= scope == ReconcileScope::Full;
        let reconciled = self.source.reconcile(scope);
        for problem in &reconciled.problems {
            self.warnings.warn("watch-registration-failed", problem);
        }
        if let Some(dirs) = reconciled.ready {
            // Refused only once the revisions are exhausted, which was already reported.
            if let Ok(events) = self.coordinator.watcher_ready(dirs) {
                self.report(&events);
            }
        } else if !reconciled.problems.is_empty() {
            let events = self
                .coordinator
                .watcher_failed(SourceFailure::RegistrationFailed);
            self.report(&events);
        }
    }

    fn report(&self, events: &[DomainEvent]) {
        for event in events {
            let detail = self.source.failure_detail();
            if let Some((code, message)) = warning_for(event, detail.as_deref()) {
                self.warnings.warn(code, &message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use preview_application::ReloadState;
    use preview_domain::{ChangeKind, Component, DebounceWindow, InstanceId, RelPath};
    use preview_testkit::{ScriptedChangeSource, ScriptedHandle};
    use tokio::runtime::{Builder, Runtime};
    use tokio::task::JoinHandle;

    use super::*;

    const WINDOW_MS: u64 = 150;

    #[derive(Default)]
    struct Log {
        nexts: usize,
        reconciles: Vec<ReconcileScope>,
        /// What the next reconciles answer, in order; nothing changed once empty.
        answers: VecDeque<Reconciled>,
        closed: bool,
    }

    /// A [`WatchControl`] over the testkit's scripted source that records what the pipeline
    /// asked of it.
    struct Scripted {
        inner: ScriptedChangeSource,
        log: Arc<Mutex<Log>>,
        detail: Option<String>,
    }

    impl ChangeSource for Scripted {
        type Event = ChangeEvent;

        fn next(&mut self) -> impl Future<Output = ChangeBatch<ChangeEvent>> + Send {
            self.log.lock().expect("unpoisoned").nexts += 1;
            self.inner.next()
        }
    }

    impl WatchControl for Scripted {
        fn reconcile(&mut self, scope: ReconcileScope) -> Reconciled {
            let mut log = self.log.lock().expect("unpoisoned");
            log.reconciles.push(scope);
            log.answers.pop_front().unwrap_or_default()
        }

        fn failure_detail(&self) -> Option<String> {
            self.detail.clone()
        }

        fn close(&mut self) {
            self.log.lock().expect("unpoisoned").closed = true;
        }
    }

    /// A [`TokioClock`] that counts its readings.
    struct Counting {
        clock: TokioClock,
        reads: Arc<AtomicUsize>,
    }

    impl Clock for Counting {
        fn now(&self) -> Tick {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.clock.now()
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, String)>>);

    impl WarningSink for Recorder {
        fn warn(&self, code: &str, message: &str) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push((code.to_owned(), message.to_owned()));
        }
    }

    struct Harness {
        handle: ScriptedHandle,
        reload: ReloadState,
        log: Arc<Mutex<Log>>,
        reads: Arc<AtomicUsize>,
        warnings: Arc<Recorder>,
        cancel: Option<oneshot::Sender<()>>,
        task: JoinHandle<()>,
    }

    /// A current-thread runtime whose clock moves only when a test advances it (or when every
    /// task is idle, to the earliest timer).
    fn paused() -> Runtime {
        Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("a runtime")
    }

    /// Spawns a pipeline on the current runtime.
    fn start(detail: Option<&str>) -> Harness {
        let (inner, handle) = ScriptedChangeSource::new();
        let log = Arc::new(Mutex::new(Log::default()));
        let reads = Arc::new(AtomicUsize::new(0));
        let warnings = Arc::new(Recorder::default());
        let reload = ReloadState::new(InstanceId::parse("test-1").expect("valid id"));
        let coordinator = ChangeCoordinator::new(
            DebounceWindow::from_millis(WINDOW_MS).expect("in range"),
            reload.clone(),
        );
        let source = Scripted {
            inner,
            log: Arc::clone(&log),
            detail: detail.map(str::to_owned),
        };
        let clock = Counting {
            clock: TokioClock::start(),
            reads: Arc::clone(&reads),
        };
        let sink: Arc<dyn WarningSink> = warnings.clone();
        let pipeline = WatchPipeline::new(source, coordinator, clock, sink);
        let (cancel, cancelled) = oneshot::channel();
        let task = tokio::spawn(pipeline.run(cancelled));
        Harness {
            handle,
            reload,
            log,
            reads,
            warnings,
            cancel: Some(cancel),
            task,
        }
    }

    impl Harness {
        fn revision(&self) -> u64 {
            self.reload.current().revision().get()
        }

        fn log<T>(&self, read: impl FnOnce(&Log) -> T) -> T {
            read(&self.log.lock().expect("unpoisoned"))
        }

        fn warnings(&self) -> Vec<(String, String)> {
            self.warnings.0.lock().expect("unpoisoned").clone()
        }

        /// Cancels the task and waits for it to end.
        async fn stop(mut self) -> Log {
            if let Some(cancel) = self.cancel.take() {
                cancel.send(()).expect("the pipeline is running");
            }
            (&mut self.task).await.expect("the task does not panic");
            core::mem::take(&mut *self.log.lock().expect("unpoisoned"))
        }
    }

    fn event(kind: ChangeKind, path: &str) -> ChangeEvent {
        ChangeEvent {
            kind,
            path: RelPath::new(
                path.split('/')
                    .map(|part| Component::new(part).expect("valid component"))
                    .collect(),
            ),
        }
    }

    /// Lets the pipeline take what was just pushed at the current instant, then moves the
    /// paused clock `millis` forward; the pipeline's timers that expire run before this returns.
    async fn after(millis: u64) {
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(millis)).await;
        tokio::task::yield_now().await;
    }

    #[test]
    fn should_publish_one_revision_when_events_burst_within_window() {
        paused().block_on(async {
            // given
            let h = start(None);
            let mut published = h.reload.subscribe();

            // when: three edits, each inside the window of the one before
            h.handle.push(vec![event(ChangeKind::Modify, "a.css")]);
            after(100).await;
            h.handle.push(vec![event(ChangeKind::Modify, "b.js")]);
            after(100).await;
            h.handle.push(vec![event(ChangeKind::Modify, "a.css")]);
            after(WINDOW_MS - 1).await;
            let before = h.revision();
            after(2).await;
            let settled = h.revision();
            after(10_000).await;

            // then
            assert_eq!(before, 0, "nothing is published inside the window");
            assert_eq!(settled, 1, "the burst is one revision");
            assert_eq!(h.revision(), 1, "and nothing after it");
            let seen = published.changed().await.expect("open");
            assert!(!seen.changes.is_full());
            let paths: Vec<_> = seen
                .changes
                .changes()
                .iter()
                .map(|p| p.url_path().to_owned())
                .collect();
            assert_eq!(paths, ["/a.css", "/b.js"]);
            h.stop().await;
        });
    }

    #[test]
    fn should_publish_after_overflow_when_channel_overflowed() {
        paused().block_on(async {
            // given
            let h = start(None);
            let mut published = h.reload.subscribe();

            // when
            h.handle.overflow();
            after(WINDOW_MS + 1).await;

            // then: one full revision, the watch set reconciled in full once
            assert_eq!(h.revision(), 1);
            let seen = published.changed().await.expect("open");
            assert!(seen.changes.is_full());
            assert_eq!(h.log(|log| log.reconciles.clone()), [ReconcileScope::Full]);

            // and changes after the loss are still published
            h.handle.push(vec![event(ChangeKind::Modify, "index.html")]);
            after(WINDOW_MS + 1).await;
            assert_eq!(h.revision(), 2);
            h.stop().await;
        });
    }

    #[test]
    fn should_stop_task_when_cancelled() {
        paused().block_on(async {
            // given: a change still pending
            let h = start(None);
            h.handle.push(vec![event(ChangeKind::Modify, "a.css")]);
            after(1).await;

            // when
            let log = h.stop().await;

            // then
            assert!(log.closed, "the source is closed when the task stops");
        });
    }

    #[test]
    fn should_stop_task_when_cancel_sender_is_dropped() {
        paused().block_on(async {
            // given
            let mut h = start(None);

            // when
            drop(h.cancel.take());

            // then
            (&mut h.task).await.expect("the task does not panic");
            assert!(h.log(|log| log.closed));
        });
    }

    #[test]
    fn should_arm_no_timer_when_nothing_is_pending() {
        paused().block_on(async {
            // given
            let h = start(None);

            // when: idle for an hour, then an hour after a batch the debouncer ignores
            after(3_600_000).await;
            let idle_reads = h.reads.load(Ordering::SeqCst);
            h.handle.push(vec![event(ChangeKind::Modify, ".git/index")]);
            after(3_600_000).await;

            // then: one reading for the batch, no timer, no wake-up and no revision
            assert_eq!(idle_reads, 0, "no timer is armed with nothing pending");
            assert_eq!(h.reads.load(Ordering::SeqCst), 1);
            assert_eq!(h.log(|log| log.nexts), 2, "one wait per batch, no spinning");
            assert_eq!(h.revision(), 0);
            h.stop().await;
        });
    }

    #[test]
    fn should_settle_pending_and_stop_polling_when_source_closes() {
        paused().block_on(async {
            // given
            let h = start(None);
            h.handle.push(vec![event(ChangeKind::Modify, "a.css")]);

            // when
            h.handle.close();
            after(WINDOW_MS + 1).await;
            after(3_600_000).await;

            // then: the pending change is published, and the closed source is not polled
            // again (it would answer `Closed` at once, for ever)
            assert_eq!(h.revision(), 1);
            assert_eq!(h.log(|log| log.nexts), 2);
            assert!(!h.task.is_finished(), "the task waits for its stop");
            h.stop().await;
        });
    }

    #[test]
    fn should_warn_once_and_keep_publishing_when_watcher_fails_at_runtime() {
        paused().block_on(async {
            // given
            let h = start(Some("file watching failed at `blog`: permission denied"));

            // when
            h.handle.fail(SourceFailure::RuntimeFailure);
            h.handle.fail(SourceFailure::RuntimeFailure);
            h.handle.push(vec![event(ChangeKind::Modify, "a.css")]);
            after(WINDOW_MS + 1).await;

            // then
            let warnings = h.warnings();
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert_eq!(warnings[0].0, "watch-failed");
            assert!(
                warnings[0]
                    .1
                    .starts_with("file watching failed at `blog`: permission denied; "),
                "{warnings:?}"
            );
            assert_eq!(h.revision(), 1, "pending and later changes still publish");
            h.stop().await;
        });
    }

    #[test]
    fn should_not_warn_again_when_registration_failure_is_delivered() {
        paused().block_on(async {
            // given
            let h = start(None);

            // when
            h.handle.fail(SourceFailure::RegistrationFailed);
            after(1).await;

            // then: start-up already named each directory
            assert!(h.warnings().is_empty());
            h.stop().await;
        });
    }

    #[test]
    fn should_reconcile_when_directory_structure_changes() {
        paused().block_on(async {
            // given
            let h = start(None);

            // when: a directory change, then an event too coarse to attribute
            h.handle
                .push(vec![event(ChangeKind::DirectoryChanged, "blog")]);
            after(WINDOW_MS + 1).await;
            h.handle.push(vec![event(ChangeKind::Ambiguous, "blog/x")]);
            after(WINDOW_MS + 1).await;
            h.handle.push(vec![event(ChangeKind::Modify, "a.css")]);
            after(WINDOW_MS + 1).await;

            // then: the change at once; the coarse event at once and in full when its
            // revision asks for a rescan; a file edit never
            assert_eq!(
                h.log(|log| log.reconciles.clone()),
                [
                    ReconcileScope::Changed,
                    ReconcileScope::Changed,
                    ReconcileScope::Full
                ]
            );
            assert_eq!(h.revision(), 3);
            h.stop().await;
        });
    }

    #[test]
    fn should_warn_and_degrade_when_reconcile_cannot_watch_a_directory() {
        paused().block_on(async {
            // given
            let h = start(Some("file watching failed: permission denied"));
            h.log
                .lock()
                .expect("unpoisoned")
                .answers
                .push_back(Reconciled {
                    ready: None,
                    problems: vec!["cannot watch directory `new/locked`: permission denied".into()],
                });

            // when
            h.handle.push(vec![event(ChangeKind::Create, "new")]);
            after(WINDOW_MS + 1).await;
            h.handle.fail(SourceFailure::RuntimeFailure);
            after(1).await;

            // then: the directory is named once and the change publishes; the watcher is
            // already degraded, so the later runtime failure is no new degradation to warn of
            assert_eq!(
                h.warnings(),
                [(
                    "watch-registration-failed".to_owned(),
                    "cannot watch directory `new/locked`: permission denied".to_owned()
                )]
            );
            assert_eq!(h.revision(), 1);
            h.stop().await;
        });
    }

    #[test]
    fn should_recover_when_reconcile_completes_the_watch_set() {
        paused().block_on(async {
            // given: degraded by a directory that could not be watched
            let h = start(Some("file watching failed: permission denied"));
            {
                let mut log = h.log.lock().expect("unpoisoned");
                log.answers.push_back(Reconciled {
                    ready: None,
                    problems: vec!["cannot watch directory `a`: permission denied".into()],
                });
                log.answers.push_back(Reconciled {
                    ready: Some(4),
                    problems: Vec::new(),
                });
            }
            h.handle.push(vec![event(ChangeKind::Create, "a")]);
            after(WINDOW_MS + 1).await;

            // when
            h.handle.push(vec![event(ChangeKind::Rename, "b")]);
            after(WINDOW_MS + 1).await;
            h.handle.fail(SourceFailure::RuntimeFailure);
            after(1).await;

            // then: watching again, so the runtime failure degrades it anew and is warned of
            let codes: Vec<String> = h.warnings().into_iter().map(|(code, _)| code).collect();
            assert_eq!(codes, ["watch-registration-failed", "watch-failed"]);
            assert_eq!(h.revision(), 2);
            h.stop().await;
        });
    }

    #[test]
    fn should_read_elapsed_millis_when_tokio_time_advances() {
        paused().block_on(async {
            // given
            let clock = TokioClock::start();
            let first = clock.now();

            // when
            after(1_234).await;

            // then
            assert_eq!(first.as_millis(), 0);
            assert_eq!(clock.now().as_millis(), 1_234);
        });
    }

    #[test]
    fn should_warn_with_stable_code_when_watcher_degrades() {
        // given
        let degraded = |reason| DomainEvent::WatcherDegraded { reason };

        // when
        let runtime = warning_for(&degraded(DegradeReason::RuntimeFailure), None);
        let exhausted = warning_for(&degraded(DegradeReason::RevisionExhausted), None);
        let registration = warning_for(&degraded(DegradeReason::RegistrationFailed), None);
        let recovered = warning_for(&DomainEvent::WatcherRecovered, None);

        // then
        let (code, message) = runtime.expect("a warning");
        assert_eq!(code, "watch-failed");
        assert!(message.starts_with("file watching failed; "), "{message}");
        assert_eq!(exhausted.map(|(code, _)| code), Some("reload-exhausted"));
        assert_eq!(registration, None);
        assert_eq!(recovered, None);
    }
}
