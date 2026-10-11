//! The one **global** trailing-edge debounce of plan §8.3, as a pure state machine.
//!
//! Every relevant change (and every watcher overflow) marks the site dirty and moves the
//! deadline to `now + window`; once a poll sees the deadline reached, the machine fires one
//! [`Settled`] and is clean again. The fire carries the [`ChangeSet`] accumulated since the
//! previous one (ADR 0030), and that set is never both empty and hot: every recorded change
//! adds a path or marks the set full (pinned by
//! `should_fire_full_or_with_changes_when_schedule_is_arbitrary`). There is no internal clock and no timer: the caller passes
//! the current [`Tick`] to every call and sleeps until the returned [`Deadline`] itself.
//!
//! Rejected alternatives: a leading-edge or per-file debounce (plan §8.3 asks for one global
//! trailing edge), and a maximum settling interval (plan §8.3 defers it until editor
//! workloads show indefinite postponement).
//!
//! Ticks are expected to be monotonic. A tick earlier than one already seen is treated as the
//! latest tick seen, so the deadline never moves backwards and a pending change is never
//! dropped (pinned by `should_not_move_deadline_backwards_when_tick_goes_backwards`).

use core::time::Duration;

use crate::change::{ChangeEvent, Classification, classify};
use crate::change_set::{ChangeSet, FullReason};
use crate::time::{DebounceWindow, Tick};

/// The instant at which a dirty debouncer becomes ready to fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Deadline(Tick);

impl Deadline {
    /// The tick at which a poll fires.
    #[must_use]
    pub const fn at(self) -> Tick {
        self.0
    }

    /// How long a caller at `now` should wait before polling again; zero once reached.
    #[must_use]
    pub const fn remaining_from(self, now: Tick) -> Duration {
        now.until(self.0)
    }
}

/// The outcome of one settled debounce window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// An overflow (or an unattributable event) was recorded since the previous fire: the
    /// caller must rescan every eligible directory instead of trusting the changes it recorded
    /// (plan §8.3). `changes` is then full with [`FullReason::Rescan`] or an earlier reason.
    pub rescan: bool,
    /// The changes since the previous fire; full or non-empty.
    pub changes: ChangeSet,
}

/// What a poll at a given tick asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// Nothing changed since the last fire; wait for the next change.
    Idle,
    /// A change is pending; poll again at the deadline.
    Wait(Deadline),
    /// The window has passed since the latest change; publish once. The debouncer is clean.
    Fire(Settled),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    deadline: Tick,
    rescan: bool,
}

/// The global trailing-edge debounce machine (plan §8.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Debouncer {
    window: DebounceWindow,
    latest: Tick,
    pending: Option<Pending>,
    changes: ChangeSet,
}

impl Debouncer {
    /// A clean debouncer that waits `window` after the latest change.
    #[must_use]
    pub const fn new(window: DebounceWindow) -> Self {
        Self {
            window,
            latest: Tick::from_millis(0),
            pending: None,
            changes: ChangeSet::new(),
        }
    }

    /// The configured quiet window.
    #[must_use]
    pub const fn window(&self) -> DebounceWindow {
        self.window
    }

    /// Whether a change is pending that the next fire will settle.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.pending.is_some()
    }

    /// The pending deadline, if dirty.
    #[must_use]
    pub fn deadline(&self) -> Option<Deadline> {
        self.pending.map(|p| Deadline(p.deadline))
    }

    /// Records one change event at `now`, classifying it with
    /// [`classify`](crate::classify) first, and returns that classification.
    ///
    /// An ignored event leaves the machine untouched. Anything else is added to the pending
    /// [`ChangeSet`] (see [`ChangeSet::accumulate`]) and makes the machine dirty, with the
    /// deadline moved to `now + window` (read it with [`Self::deadline`]); a `Rescan`
    /// classification also requests a rescan, like [`Self::mark_overflow`] (pinned by
    /// `should_request_rescan_when_event_is_ambiguous` and
    /// `should_stay_clean_when_event_is_ignored`). The classification is not an argument, so
    /// no caller can record an ineligible path as relevant.
    ///
    /// A change recorded exactly at the deadline, before the poll that would fire, extends
    /// it: the window is measured from the latest change (pinned by
    /// `should_extend_when_change_recorded_exactly_at_deadline_before_poll`).
    pub fn record(&mut self, now: Tick, event: &ChangeEvent) -> Classification {
        let classification = classify(event);
        if !matches!(classification, Classification::Ignored(_)) {
            self.changes.accumulate_classified(event, classification);
            self.touch(now, classification == Classification::Rescan);
        }
        classification
    }

    /// Records a lost-notification overflow at `now`: dirty like [`Self::record`], the next
    /// fire requests a full rescan, and its change set is full with [`FullReason::Rescan`]
    /// (pinned by `should_request_rescan_when_overflow_marked`).
    pub fn mark_overflow(&mut self, now: Tick) -> Deadline {
        self.changes.mark_full(FullReason::Rescan);
        self.touch(now, true)
    }

    /// Fires once the deadline is reached (`now >= deadline`), clearing the dirty state.
    ///
    /// Pinned by `should_fire_once_when_burst_settles` and
    /// `should_fire_when_poll_is_exactly_at_deadline`.
    pub fn poll(&mut self, now: Tick) -> Poll {
        let now = self.observe(now);
        match self.pending {
            None => Poll::Idle,
            Some(p) if now >= p.deadline => {
                self.pending = None;
                Poll::Fire(Settled {
                    rescan: p.rescan,
                    changes: self.changes.take(),
                })
            }
            Some(p) => Poll::Wait(Deadline(p.deadline)),
        }
    }

    fn touch(&mut self, now: Tick, rescan: bool) -> Deadline {
        let now = self.observe(now);
        let deadline = now.saturating_add(self.window);
        let rescan = rescan || self.pending.is_some_and(|p| p.rescan);
        self.pending = Some(Pending { deadline, rescan });
        Deadline(deadline)
    }

    /// Clamps `now` to the latest tick seen, so time never runs backwards inside the machine.
    fn observe(&mut self, now: Tick) -> Tick {
        self.latest = self.latest.max(now);
        self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::ChangeKind;
    use crate::change_set::{ChangedPath, MAX_CHANGES};
    use crate::path::Component;
    use crate::rel_path::RelPath;
    use proptest::prelude::*;

    fn t(ms: u64) -> Tick {
        Tick::from_millis(ms)
    }

    fn debouncer(ms: u64) -> Debouncer {
        Debouncer::new(DebounceWindow::from_millis(ms).expect("valid window"))
    }

    fn css() -> ChangeEvent {
        ChangeEvent {
            kind: ChangeKind::Modify,
            path: RelPath::new(vec![Component::new("a.css").expect("valid")]),
        }
    }

    /// Records a relevant `a.css` modification, the common case of these tests.
    fn rec(d: &mut Debouncer, now: Tick) -> Deadline {
        let event = css();
        d.record(now, &event);
        d.deadline()
            .expect("a relevant change dirties the debouncer")
    }

    fn css_set() -> ChangeSet {
        let mut set = ChangeSet::new();
        let event = css();
        set.accumulate(&event);
        set
    }

    fn clean() -> Poll {
        Poll::Fire(Settled {
            rescan: false,
            changes: css_set(),
        })
    }

    fn rescan(changes: ChangeSet) -> Poll {
        let mut changes = changes;
        changes.mark_full(FullReason::Rescan);
        Poll::Fire(Settled {
            rescan: true,
            changes,
        })
    }

    #[test]
    fn should_fire_once_when_burst_settles() {
        // given
        let mut d = debouncer(150);
        for ms in [0, 10, 20, 30, 40] {
            rec(&mut d, t(ms));
        }
        // when
        let early = d.poll(t(100));
        let settled = d.poll(t(190));
        let after = [d.poll(t(191)), d.poll(t(10_000))];
        // then
        assert_eq!(early, Poll::Wait(Deadline(t(190))));
        assert_eq!(settled, clean());
        assert_eq!(after, [Poll::Idle, Poll::Idle]);
        assert!(!d.is_dirty());
    }

    #[test]
    fn should_extend_deadline_when_event_arrives_before_deadline() {
        // given
        let mut d = debouncer(150);
        let first = rec(&mut d, t(0));
        // when
        let second = rec(&mut d, t(149));
        let at_first_deadline = d.poll(t(150));
        // then
        assert_eq!(first.at(), t(150));
        assert_eq!(second.at(), t(299));
        assert_eq!(at_first_deadline, Poll::Wait(second));
        assert_eq!(d.poll(t(299)), clean());
    }

    #[test]
    fn should_not_fire_when_no_event_recorded() {
        // given
        let mut d = debouncer(10);
        // when
        let polls = [d.poll(t(0)), d.poll(t(10)), d.poll(t(u64::MAX))];
        // then
        assert_eq!(polls, [Poll::Idle, Poll::Idle, Poll::Idle]);
        assert_eq!(d.deadline(), None);
    }

    #[test]
    fn should_request_rescan_when_overflow_marked() {
        // given
        let mut d = debouncer(150);
        rec(&mut d, t(0));
        d.mark_overflow(t(50));
        rec(&mut d, t(100));
        // when
        let fired = d.poll(t(250));
        rec(&mut d, t(300));
        let next = d.poll(t(450));
        // then
        assert_eq!(fired, rescan(css_set()));
        assert_eq!(next, clean());
    }

    #[test]
    fn should_fire_rescan_when_overflow_is_the_only_signal() {
        // given
        let mut d = debouncer(150);
        // when
        let deadline = d.mark_overflow(t(7));
        // then
        assert!(d.is_dirty());
        assert_eq!(d.poll(t(156)), Poll::Wait(deadline));
        assert_eq!(d.poll(t(157)), rescan(ChangeSet::new()));
    }

    #[test]
    fn should_fire_when_poll_is_exactly_at_deadline() {
        // given
        let mut d = debouncer(150);
        let deadline = rec(&mut d, t(1_000));
        // when
        let before = d.poll(t(1_149));
        let at = d.poll(deadline.at());
        // then
        assert_eq!(before, Poll::Wait(deadline));
        assert_eq!(at, clean());
    }

    #[test]
    fn should_extend_when_change_recorded_exactly_at_deadline_before_poll() {
        // given
        let mut d = debouncer(150);
        let first = rec(&mut d, t(0));
        // when
        let second = rec(&mut d, first.at());
        // then
        assert_eq!(d.poll(first.at()), Poll::Wait(second));
        assert_eq!(d.poll(t(300)), clean());
    }

    #[test]
    fn should_start_new_cycle_when_change_follows_fire_at_same_tick() {
        // given
        let mut d = debouncer(150);
        rec(&mut d, t(0));
        assert_eq!(d.poll(t(150)), clean());
        // when
        let deadline = rec(&mut d, t(150));
        // then
        assert_eq!(deadline.at(), t(300));
        assert_eq!(d.poll(t(300)), clean());
    }

    #[test]
    fn should_keep_deadline_when_event_repeats_at_same_tick() {
        // given
        let mut d = debouncer(150);
        let first = rec(&mut d, t(42));
        // when
        let again = rec(&mut d, t(42));
        // then
        assert_eq!(first, again);
    }

    #[test]
    fn should_not_move_deadline_backwards_when_tick_goes_backwards() {
        // given
        let mut d = debouncer(150);
        let first = rec(&mut d, t(1_000));
        // when
        let backwards = rec(&mut d, t(10));
        let poll_in_past = d.poll(t(20));
        // then
        assert_eq!(backwards, first);
        assert_eq!(poll_in_past, Poll::Wait(first));
        assert!(d.is_dirty());
        assert_eq!(d.poll(t(1_150)), clean());
    }

    #[test]
    fn should_fire_at_max_when_deadline_saturates() {
        // given
        let mut d = debouncer(150);
        let deadline = rec(&mut d, t(u64::MAX - 10));
        // when
        let before = d.poll(t(u64::MAX - 1));
        let at_max = d.poll(t(u64::MAX));
        // then
        assert_eq!(deadline.at(), t(u64::MAX));
        assert_eq!(before, Poll::Wait(deadline));
        assert_eq!(at_max, clean());
    }

    #[test]
    fn should_report_remaining_time_when_deadline_is_pending() {
        // given
        let mut d = debouncer(150);
        let deadline = rec(&mut d, t(100));
        // when
        let remaining = deadline.remaining_from(t(160));
        let overdue = deadline.remaining_from(t(400));
        // then
        assert_eq!(remaining, Duration::from_millis(90));
        assert_eq!(overdue, Duration::ZERO);
        assert_eq!(d.deadline(), Some(deadline));
        assert_eq!(d.window().as_millis(), 150);
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

    #[test]
    fn should_carry_accumulated_changes_when_fired() {
        // given
        let mut d = debouncer(150);
        let style = event(ChangeKind::Modify, &["css", "site.css"]);
        let script = event(ChangeKind::Modify, &["app.js"]);
        d.record(t(0), &style);
        d.record(t(10), &script);
        d.record(t(20), &style);

        // when
        let fired = d.poll(t(170));
        rec(&mut d, t(200));
        let next = d.poll(t(350));

        // then
        let Poll::Fire(settled) = fired else {
            panic!("expected a fire, got {fired:?}");
        };
        let urls: Vec<&str> = settled
            .changes
            .changes()
            .iter()
            .map(ChangedPath::url_path)
            .collect();
        assert_eq!(urls, ["/app.js", "/css/site.css"]);
        assert!(!settled.changes.is_full());
        assert!(!settled.rescan);
        assert_eq!(next, clean());
    }

    #[test]
    fn should_request_rescan_when_event_is_ambiguous() {
        // given
        let mut d = debouncer(150);
        let ambiguous = event(ChangeKind::Ambiguous, &["css", "site.css"]);

        // when
        let classification = d.record(t(0), &ambiguous);

        // then
        assert_eq!(classification, Classification::Rescan);
        assert_eq!(d.deadline().map(Deadline::at), Some(t(150)));
        assert_eq!(d.poll(t(150)), rescan(ChangeSet::new()));
    }

    #[test]
    fn should_stay_clean_when_event_is_ignored() {
        // given
        let mut d = debouncer(150);
        let ignored = event(ChangeKind::Modify, &[".git", "index"]);

        // when
        let classification = d.record(t(0), &ignored);

        // then
        assert!(matches!(classification, Classification::Ignored(_)));
        assert_eq!(d.deadline(), None);
        assert!(!d.is_dirty());
        assert_eq!(d.poll(t(1_000)), Poll::Idle);
    }

    #[test]
    fn should_fire_full_when_html_changes_in_window() {
        // given
        let mut d = debouncer(150);
        let page = event(ChangeKind::Modify, &["index.html"]);
        rec(&mut d, t(0));
        d.record(t(5), &page);

        // when
        let fired = d.poll(t(155));

        // then
        let Poll::Fire(settled) = fired else {
            panic!("expected a fire, got {fired:?}");
        };
        assert_eq!(settled.changes.reason(), Some(FullReason::DocumentChanged));
        assert!(!settled.rescan);
    }

    /// One step of a generated schedule: advance the clock by `gap` ms, then act.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        Record,
        Overflow,
        Poll,
    }

    fn op() -> impl Strategy<Value = (u64, Op)> {
        (
            0_u64..400,
            prop_oneof![
                Just(Op::Record),
                Just(Op::Overflow),
                Just(Op::Poll),
                Just(Op::Poll)
            ],
        )
    }

    /// Polls at `now` and checks a fire against the model: never before the window has
    /// passed since the latest change, and a rescan exactly when an overflow preceded it.
    fn check_poll(
        d: &mut Debouncer,
        window_ms: u64,
        now: u64,
        last: Option<u64>,
        overflow: &mut bool,
        fires: &mut usize,
    ) -> Result<(), TestCaseError> {
        if let Poll::Fire(settled) = d.poll(t(now)) {
            let Some(last) = last else {
                return Err(TestCaseError::fail(format!(
                    "fired at {now} with no change"
                )));
            };
            prop_assert!(
                now >= last + window_ms,
                "fired at {now} before {last}+{window_ms}"
            );
            prop_assert_eq!(settled.rescan, *overflow);
            *overflow = false;
            *fires += 1;
        }
        Ok(())
    }

    proptest! {
        #[test]
        fn should_bound_fires_by_quiet_gaps_when_schedule_is_arbitrary(
            window_ms in 10_u64..=300,
            ops in proptest::collection::vec(op(), 0..60),
            tail in 0_u64..1_000,
        ) {
            // given
            let window = DebounceWindow::from_millis(window_ms).expect("in range");
            let mut d = Debouncer::new(window);
            let mut now = 0_u64;
            let mut events: Vec<u64> = Vec::new();
            let mut fires = 0_usize;
            let mut overflow_since_fire = false;
            let mut last_event: Option<u64> = None;
            // when
            for (gap, op) in ops {
                now += gap;
                match op {
                    Op::Record => { rec(&mut d, t(now)); events.push(now); last_event = Some(now); }
                    Op::Overflow => {
                        d.mark_overflow(t(now));
                        events.push(now);
                        last_event = Some(now);
                        overflow_since_fire = true;
                    }
                    Op::Poll => check_poll(
                        &mut d, window_ms, now, last_event, &mut overflow_since_fire, &mut fires,
                    )?,
                }
            }
            let end = last_event.map_or(now, |last| last + window_ms).max(now) + tail;
            check_poll(&mut d, window_ms, end, last_event, &mut overflow_since_fire, &mut fires)?;
            // then
            let quiet_gaps = events.windows(2).filter(|w| w[1] - w[0] >= window_ms).count();
            prop_assert!(fires <= quiet_gaps + 1, "fires {fires} > quiet gaps {quiet_gaps} + 1");
            if events.is_empty() {
                prop_assert_eq!(fires, 0);
            } else {
                prop_assert!(fires >= 1);
            }
            prop_assert!(!d.is_dirty(), "a change was left pending after the window passed");
            prop_assert_eq!(d.poll(t(end + window_ms)), Poll::Idle);
        }

        #[test]
        fn should_never_lose_a_change_when_ticks_are_not_monotonic(
            window_ms in 10_u64..=10_000,
            ticks in proptest::collection::vec(any::<u64>(), 1..30),
        ) {
            // given
            let mut d = Debouncer::new(DebounceWindow::from_millis(window_ms).expect("in range"));
            let mut previous_deadline = None;
            // when
            for tick in &ticks {
                let deadline = rec(&mut d, t(*tick));
                prop_assert!(previous_deadline.is_none_or(|p| deadline >= p));
                previous_deadline = Some(deadline);
            }
            let fired = d.poll(t(u64::MAX));
            // then
            prop_assert_eq!(fired, clean());
            prop_assert!(!d.is_dirty());
        }

        #[test]
        fn should_fire_full_or_with_changes_when_schedule_is_arbitrary(
            steps in proptest::collection::vec(step(), 0..120),
        ) {
            // given
            let mut d = debouncer(150);
            let mut now = 0_u64;
            let mut fires = Vec::new();
            // when
            for (gap, step) in steps {
                now += gap;
                match step {
                    Step::Event(e) => { d.record(t(now), &e); }
                    Step::Overflow => { d.mark_overflow(t(now)); }
                    Step::Poll => {
                        if let Poll::Fire(settled) = d.poll(t(now)) {
                            fires.push(settled);
                        }
                    }
                }
            }
            if let Poll::Fire(settled) = d.poll(t(u64::MAX)) {
                fires.push(settled);
            }
            // then
            for settled in fires {
                let changes = &settled.changes;
                prop_assert!(changes.is_full() || !changes.changes().is_empty());
                prop_assert!(!settled.rescan || changes.is_full());
                prop_assert!(changes.changes().len() <= MAX_CHANGES);
                if !changes.is_full() {
                    prop_assert!(changes.changes().iter().all(|c| c.kind().is_hot()));
                }
            }
        }
    }

    #[derive(Debug, Clone)]
    enum Step {
        Event(ChangeEvent),
        Overflow,
        Poll,
    }

    fn step() -> impl Strategy<Value = (u64, Step)> {
        let kind = prop_oneof![
            4 => Just(ChangeKind::Modify),
            1 => Just(ChangeKind::Create),
            1 => Just(ChangeKind::Remove),
            1 => Just(ChangeKind::Rename),
            1 => Just(ChangeKind::DirectoryChanged),
            1 => Just(ChangeKind::AccessOnly),
            1 => Just(ChangeKind::Ambiguous),
        ];
        let name = (
            prop_oneof![Just("a"), Just("b"), Just(".git"), Just("node_modules")],
            "[a-z]{1,3}",
            prop_oneof![
                Just("css"),
                Just("js"),
                Just("png"),
                Just("html"),
                Just("txt")
            ],
        );
        let event = (kind, name).prop_map(|(kind, (dir, stem, ext))| {
            let file = format!("{stem}.{ext}");
            Step::Event(event(kind, &[dir, &file]))
        });
        (
            0_u64..300,
            prop_oneof![6 => event, 1 => Just(Step::Overflow), 3 => Just(Step::Poll)],
        )
    }
}
