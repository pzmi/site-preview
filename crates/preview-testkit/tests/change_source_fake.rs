//! The `ChangeSource` contract harness against the scripted fake, and its negative controls: a
//! source that swallows `Overflow` fails the harness at `overflow_before_later_changes` and
//! nowhere else (foundation.md section 5.1), as the same wrapper around the native watcher does
//! in preview-adapter-fs; a source that loses a batch when a `next` is dropped fails at
//! `dropped_next_loses_nothing`.

use core::num::NonZeroUsize;
use std::time::Duration;

use preview_application::{ChangeBatch, ChangeSource};
use preview_domain::ChangeEvent;
use preview_testkit::ScriptedSite;
use preview_testkit::contracts::{
    CHANGE_SOURCE_CASES, ChangeSourceFactory, WatchTree, change_source_contract,
    check_change_source_contract,
};

/// The deliberately broken source: passes every batch through except `Overflow`, which it
/// swallows, as a consumer-side "the rescan is noise" shortcut would.
struct DropsOverflow<S>(S);

impl<S: ChangeSource<Event = ChangeEvent>> ChangeSource for DropsOverflow<S> {
    type Event = ChangeEvent;

    async fn next(&mut self) -> ChangeBatch<ChangeEvent> {
        loop {
            match self.0.next().await {
                ChangeBatch::Overflow => {}
                other => return other,
            }
        }
    }
}

struct BrokenDropsOverflow;

impl ChangeSourceFactory for BrokenDropsOverflow {
    type Source = DropsOverflow<<ScriptedSite as ChangeSourceFactory>::Source>;
    type Site = ScriptedSite;

    const DEADLINE: Duration = Duration::ZERO;

    fn start(tree: &WatchTree, capacity: NonZeroUsize) -> (Self::Source, ScriptedSite) {
        let (source, site) = ScriptedSite::start(tree, capacity);
        (DropsOverflow(source), site)
    }

    fn write(site: &mut ScriptedSite, path: &str) {
        ScriptedSite::write(site, path);
    }

    fn burst(site: &mut ScriptedSite, paths: &[String]) {
        ScriptedSite::burst(site, paths);
    }

    fn stop(site: &mut ScriptedSite, source: &mut Self::Source) {
        ScriptedSite::stop(site, &mut source.0);
    }
}

/// The second broken source: takes a batch, then yields once before returning it, so a `next`
/// dropped at that yield loses the batch it took.
struct LosesOnDrop<S>(S);

/// Pending on the first poll, ready on the second.
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<()> {
        if self.0 {
            return core::task::Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        core::task::Poll::Pending
    }
}

impl<S: ChangeSource<Event = ChangeEvent>> ChangeSource for LosesOnDrop<S> {
    type Event = ChangeEvent;

    async fn next(&mut self) -> ChangeBatch<ChangeEvent> {
        let batch = self.0.next().await;
        if matches!(batch, ChangeBatch::Events(_)) {
            YieldOnce(false).await;
        }
        batch
    }
}

struct BrokenLosesOnDrop;

impl ChangeSourceFactory for BrokenLosesOnDrop {
    type Source = LosesOnDrop<<ScriptedSite as ChangeSourceFactory>::Source>;
    type Site = ScriptedSite;

    const DEADLINE: Duration = Duration::ZERO;

    fn start(tree: &WatchTree, capacity: NonZeroUsize) -> (Self::Source, ScriptedSite) {
        let (source, site) = ScriptedSite::start(tree, capacity);
        (LosesOnDrop(source), site)
    }

    fn write(site: &mut ScriptedSite, path: &str) {
        ScriptedSite::write(site, path);
    }

    fn stop(site: &mut ScriptedSite, source: &mut Self::Source) {
        ScriptedSite::stop(site, &mut source.0);
    }
}

#[test]
fn should_pass_contract_when_source_is_scripted() {
    // given
    // ScriptedSite scripts what the native watcher would report for each case.

    // when
    let report = check_change_source_contract::<ScriptedSite>();

    // then
    assert_eq!(report.cases_run(), CHANGE_SOURCE_CASES);
    report.assert_passed();
    change_source_contract::<ScriptedSite>();
}

#[test]
fn should_fail_contract_when_overflow_is_dropped() {
    // given
    // BrokenDropsOverflow wraps the scripted source and swallows every `Overflow`.

    // when
    let report = check_change_source_contract::<BrokenDropsOverflow>();

    // then
    assert_eq!(report.cases_run(), CHANGE_SOURCE_CASES);
    assert_eq!(
        report.failed_cases(),
        ["overflow_before_later_changes"],
        "{report}"
    );
    let detail = &report.failures()[0].detail;
    assert!(
        detail.contains("`late.html` was delivered without an `Overflow` before it"),
        "{detail}"
    );
}

#[test]
fn should_fail_contract_when_dropped_next_loses_a_batch() {
    // given
    // BrokenLosesOnDrop takes a batch and yields before returning it.

    // when
    let report = check_change_source_contract::<BrokenLosesOnDrop>();

    // then
    assert_eq!(report.cases_run(), CHANGE_SOURCE_CASES);
    assert_eq!(
        report.failed_cases(),
        ["dropped_next_loses_nothing"],
        "{report}"
    );
}
