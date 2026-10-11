//! A manually advanced [`Clock`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use preview_application::Clock;
use preview_domain::Tick;

/// A clock that moves only when [`advance`](Self::advance) is called.
///
/// Clones share one reading, so a test keeps a handle while the code under test owns another.
/// It never goes backwards (the `Clock` contract): the only way to move it is forward.
#[derive(Debug, Clone, Default)]
pub struct FakeClock {
    millis: Arc<AtomicU64>,
}

impl FakeClock {
    /// A clock reading zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock reading `millis`.
    #[must_use]
    pub fn starting_at(millis: u64) -> Self {
        Self {
            millis: Arc::new(AtomicU64::new(millis)),
        }
    }

    /// Moves every clone forward by `ms`, saturating at `u64::MAX` rather than wrapping.
    pub fn advance(&self, ms: u64) {
        // The closure always returns `Some`, so the update cannot fail.
        let _ = self
            .millis
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                Some(now.saturating_add(ms))
            });
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Tick {
        Tick::from_millis(self.millis.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_hold_reading_when_not_advanced() {
        // given
        let clock = FakeClock::starting_at(40);
        // when
        let (first, second) = (clock.now(), clock.now());
        // then
        assert_eq!(first, second);
        assert_eq!(first.as_millis(), 40);
    }

    #[test]
    fn should_move_forward_when_advanced() {
        // given
        let clock = FakeClock::new();
        // when
        clock.advance(150);
        clock.advance(50);
        // then
        assert_eq!(clock.now().as_millis(), 200);
    }

    #[test]
    fn should_share_reading_when_clock_is_cloned() {
        // given
        let clock = FakeClock::new();
        let other = clock.clone();
        // when
        other.advance(7);
        // then
        assert_eq!(clock.now().as_millis(), 7);
    }

    #[test]
    fn should_saturate_when_advanced_past_maximum() {
        // given
        let clock = FakeClock::starting_at(u64::MAX - 1);
        let before = clock.now();
        // when
        clock.advance(10);
        // then
        assert!(clock.now() >= before);
        assert_eq!(clock.now().as_millis(), u64::MAX);
    }

    #[test]
    fn should_read_through_trait_object_when_clock_is_dyn() {
        // given
        let clock = FakeClock::starting_at(5);
        let dynamic: &dyn Clock = &clock;
        // when
        clock.advance(1);
        // then
        assert_eq!(dynamic.now().as_millis(), 6);
    }
}
