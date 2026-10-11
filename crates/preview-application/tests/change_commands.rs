//! The change commands driven the way the watcher pipeline drives them: batches from a
//! [`ScriptedChangeSource`], time from a [`FakeClock`], revisions observed through a
//! [`ReloadState`] subscription (plan.md sections 8.2 to 8.4).

use preview_application::{
    ChangeCoordinator, ChangeSource, Clock, Received, RecordOutcome, ReloadState, SettleOutcome,
    SourceFailure,
};
use preview_domain::{
    AssetKind, ChangeEvent, ChangeKind, Component, DebounceWindow, DegradeReason, DomainEvent,
    FullReason, RelPath,
};
use preview_testkit::{FakeClock, FixedInstanceIds, ScriptedChangeSource, ScriptedHandle};

const WINDOW_MS: u64 = 150;

struct Pipeline {
    source: ScriptedChangeSource,
    feed: ScriptedHandle,
    clock: FakeClock,
    coordinator: ChangeCoordinator,
}

impl Pipeline {
    fn new() -> Self {
        let (source, feed) = ScriptedChangeSource::new();
        let instance = FixedInstanceIds::new(4242, 1).next_id();
        let window = DebounceWindow::from_millis(WINDOW_MS).expect("in range");
        Self {
            source,
            feed,
            clock: FakeClock::new(),
            coordinator: ChangeCoordinator::new(window, ReloadState::new(instance)),
        }
    }

    /// Takes the next scripted batch and hands it to the coordinator at the clock's time.
    async fn deliver(&mut self) -> RecordOutcome {
        let batch = self.source.next().await;
        match self.coordinator.receive(batch, self.clock.now()) {
            Received::Recorded(outcome) => outcome,
            Received::Closed => panic!("the scripted source closed unexpectedly"),
        }
    }

    fn settle(&mut self) -> SettleOutcome {
        self.coordinator.settle(self.clock.now())
    }

    fn revision(&self) -> u64 {
        self.coordinator.reload_state().current().revision().get()
    }
}

fn event(kind: ChangeKind, parts: &[&str]) -> ChangeEvent {
    ChangeEvent {
        kind,
        path: RelPath::new(
            parts
                .iter()
                .map(|p| Component::new(p).expect("valid test component"))
                .collect(),
        ),
    }
}

#[tokio::test]
async fn should_publish_one_revision_when_burst_settles() {
    // given
    let mut p = Pipeline::new();
    let mut subscription = p.coordinator.reload_state().subscribe();
    // when
    for name in [
        "index.html",
        "about.html",
        "index.html",
        "site.css",
        "index.html",
    ] {
        p.feed.push(vec![event(ChangeKind::Modify, &[name])]);
        let outcome = p.deliver().await;
        assert_eq!(outcome.recorded, 1);
        p.clock.advance(WINDOW_MS - 1);
        assert_eq!(p.settle().settled, None, "settled inside the window");
    }
    p.clock.advance(1);
    let settled = p.settle();
    p.clock.advance(10 * WINDOW_MS);
    let after = p.settle();
    // then
    let published = settled.settled.expect("the burst settled");
    assert_eq!(published.state.revision().get(), 1);
    assert_eq!(
        settled.events,
        [DomainEvent::RevisionPublished(published.state.clone())]
    );
    assert_eq!(after, SettleOutcome::default());
    assert_eq!(p.revision(), 1);
    let seen = subscription.changed().await.expect("open");
    assert_eq!(seen.state, published.state);
    assert_eq!(seen.changes.reason(), Some(FullReason::DocumentChanged));
    let paths: Vec<_> = seen
        .changes
        .changes()
        .iter()
        .map(|c| c.url_path().to_owned())
        .collect();
    assert_eq!(paths, ["/about.html", "/index.html", "/site.css"]);
}

#[tokio::test]
async fn should_publish_css_change_set_when_only_stylesheet_edited() {
    // given
    let mut p = Pipeline::new();
    // when
    p.feed.push(vec![
        event(ChangeKind::Create, &["css", ".site.css.swp"]),
        event(ChangeKind::Modify, &["css", ".site.css.swp"]),
        event(ChangeKind::Rename, &["css", "site.css"]),
    ]);
    let recorded = p.deliver().await;
    p.clock.advance(WINDOW_MS);
    let outcome = p.settle();
    // then
    assert_eq!(recorded.recorded, 1, "the swap file is ignored");
    let settled = outcome.settled.expect("published");
    assert!(!settled.rescan);
    assert!(!settled.changes.is_full());
    let changes: Vec<_> = settled
        .changes
        .changes()
        .iter()
        .map(|c| (c.url_path().to_owned(), c.kind()))
        .collect();
    assert_eq!(changes, [("/css/site.css".to_owned(), AssetKind::Css)]);
    assert_eq!(
        p.coordinator.reload_state().subscribe().initial().changes,
        settled.changes
    );
}

#[tokio::test]
async fn should_not_publish_when_only_dot_git_changes() {
    // given
    let mut p = Pipeline::new();
    // when
    p.feed.push(vec![
        event(ChangeKind::Modify, &[".git", "index"]),
        event(ChangeKind::Create, &[".git", "objects", "ab", "cdef"]),
        event(ChangeKind::Ambiguous, &[".git", "refs"]),
        event(ChangeKind::DirectoryChanged, &[".git", "logs"]),
    ]);
    let outcome = p.deliver().await;
    p.clock.advance(10 * WINDOW_MS);
    let settled = p.settle();
    // then
    assert_eq!(outcome, RecordOutcome::default());
    assert_eq!(p.coordinator.deadline(), None);
    assert_eq!(settled, SettleOutcome::default());
    assert_eq!(p.revision(), 0);
}

#[tokio::test]
async fn should_publish_with_rescan_when_overflow_signalled() {
    // given
    let mut p = Pipeline::new();
    p.feed.push(vec![event(ChangeKind::Modify, &["site.css"])]);
    p.deliver().await;
    p.clock.advance(50);
    // when
    p.feed.overflow();
    let outcome = p.deliver().await;
    p.clock.advance(WINDOW_MS);
    let settle = p.settle();
    // then
    assert!(outcome.needs_reconcile);
    assert_eq!(
        outcome.next_deadline.map(|d| d.at().as_millis()),
        Some(50 + WINDOW_MS)
    );
    let settled = settle
        .settled
        .expect("an overflow publishes at least one revision");
    assert!(settled.rescan);
    assert_eq!(settled.changes.reason(), Some(FullReason::Rescan));
    assert_eq!(settled.state.revision().get(), 1);
}

#[tokio::test]
async fn should_emit_degraded_when_watcher_fails() {
    // given
    let mut p = Pipeline::new();
    p.coordinator.watcher_ready(3).expect("accepted");
    p.feed
        .push(vec![event(ChangeKind::Modify, &["index.html"])]);
    p.deliver().await;
    // when
    p.feed.fail(SourceFailure::RuntimeFailure);
    let first = p.deliver().await;
    p.feed.fail(SourceFailure::RuntimeFailure);
    let repeated = p.deliver().await;
    p.clock.advance(WINDOW_MS);
    let settle = p.settle();
    // then
    assert_eq!(
        first.events,
        [DomainEvent::WatcherDegraded {
            reason: DegradeReason::RuntimeFailure
        }]
    );
    assert!(repeated.events.is_empty(), "a repeated failure is absorbed");
    assert!(!p.coordinator.health().is_live());
    assert!(
        settle.settled.is_some(),
        "changes recorded before the failure still publish"
    );
}

#[tokio::test]
async fn should_stop_when_source_closes() {
    // given
    let mut p = Pipeline::new();
    p.feed.close();
    // when
    let batch = p.source.next().await;
    let received = p.coordinator.receive(batch, p.clock.now());
    // then
    assert_eq!(received, Received::Closed);
}
