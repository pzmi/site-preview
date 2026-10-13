//! Time as a value: the domain never reads a clock (D11), so every instant is supplied by the
//! caller as a [`Tick`] and every duration the user configures is a validated
//! [`DebounceWindow`].

use core::fmt;
use core::time::Duration;

/// An opaque monotonic instant in milliseconds, supplied by the caller.
///
/// Only differences between ticks of one source mean anything; the origin is the caller's
/// choice (for example, milliseconds since the watcher task started). The domain never
/// creates one from a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(u64);

impl Tick {
    /// The instant `millis` milliseconds after the caller's origin.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the caller's origin.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// This instant plus `window`, saturating at the largest representable tick rather than
    /// wrapping (pinned by `should_saturate_deadline_when_tick_is_near_max`).
    #[must_use]
    pub const fn saturating_add(self, window: DebounceWindow) -> Self {
        Self(self.0.saturating_add(window.as_millis()))
    }

    /// The time from `self` until `later`, or zero when `later` is not after `self`.
    #[must_use]
    pub const fn until(self, later: Self) -> Duration {
        Duration::from_millis(later.0.saturating_sub(self.0))
    }
}

/// Why a [`DebounceWindow`] was refused (D22: 10 ms to 10 s inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebounceWindowError {
    /// Shorter than 10 ms, zero included.
    TooShort,
    /// Longer than 10 s.
    TooLong,
    /// Not a whole number of milliseconds.
    NotWholeMillis,
}

impl fmt::Display for DebounceWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooShort => "debounce is shorter than 10ms (allowed: 10ms to 10s)",
            Self::TooLong => "debounce is longer than 10s (allowed: 10ms to 10s)",
            Self::NotWholeMillis => {
                "debounce is not a whole number of milliseconds (allowed: 10ms to 10s)"
            }
        })
    }
}

impl core::error::Error for DebounceWindowError {}

/// The quiet interval the global debounce waits for after the latest relevant change
/// (plan §8.3), validated against D22: 10 ms to 10 s inclusive, whole milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DebounceWindow(u64);

impl DebounceWindow {
    /// The smallest accepted window, 10 ms.
    pub const MIN: Self = Self(10);
    /// The largest accepted window, 10 s.
    pub const MAX: Self = Self(10_000);
    /// The default window, 150 ms (plan §8.3).
    pub const DEFAULT: Self = Self(150);

    /// Validates a window given in milliseconds.
    ///
    /// Pinned by `should_refuse_window_when_zero_or_above_ten_seconds` and
    /// `should_accept_window_when_at_either_bound`.
    ///
    /// # Errors
    /// [`DebounceWindowError::TooShort`] below 10 ms, [`DebounceWindowError::TooLong`]
    /// above 10 s.
    pub const fn from_millis(millis: u64) -> Result<Self, DebounceWindowError> {
        if millis < Self::MIN.0 {
            Err(DebounceWindowError::TooShort)
        } else if millis > Self::MAX.0 {
            Err(DebounceWindowError::TooLong)
        } else {
            Ok(Self(millis))
        }
    }

    /// The window in milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// The window as a [`Duration`].
    #[must_use]
    pub const fn as_duration(self) -> Duration {
        Duration::from_millis(self.0)
    }
}

impl Default for DebounceWindow {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<Duration> for DebounceWindow {
    type Error = DebounceWindowError;

    /// Validates a [`Duration`]; a sub-millisecond remainder is refused rather than rounded,
    /// so the window used is exactly the one asked for (pinned by
    /// `should_refuse_window_when_duration_has_sub_millisecond_part`).
    fn try_from(value: Duration) -> Result<Self, Self::Error> {
        if !value.subsec_nanos().is_multiple_of(1_000_000) {
            return Err(DebounceWindowError::NotWholeMillis);
        }
        // A Duration above u64::MAX milliseconds is far above 10 s.
        let millis = u64::try_from(value.as_millis()).map_err(|_| DebounceWindowError::TooLong)?;
        Self::from_millis(millis)
    }
}

impl fmt::Display for DebounceWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_refuse_window_when_zero_or_above_ten_seconds() {
        // given
        let zero = 0;
        let just_below = 9;
        let just_above = 10_001;
        // when
        let results = [
            DebounceWindow::from_millis(zero),
            DebounceWindow::from_millis(just_below),
            DebounceWindow::from_millis(just_above),
            DebounceWindow::from_millis(u64::MAX),
        ];
        // then
        assert_eq!(
            results,
            [
                Err(DebounceWindowError::TooShort),
                Err(DebounceWindowError::TooShort),
                Err(DebounceWindowError::TooLong),
                Err(DebounceWindowError::TooLong),
            ]
        );
    }

    #[test]
    fn should_accept_window_when_at_either_bound() {
        // given
        let bounds = [10, 10_000];
        // when
        let windows = bounds.map(DebounceWindow::from_millis);
        // then
        assert_eq!(windows, [Ok(DebounceWindow::MIN), Ok(DebounceWindow::MAX)]);
        assert_eq!(DebounceWindow::default().as_millis(), 150);
    }

    #[test]
    fn should_refuse_window_when_duration_has_sub_millisecond_part() {
        // given
        let fractional = Duration::from_micros(150_500);
        let whole = Duration::from_secs(10);
        let huge = Duration::MAX;
        // when
        let results = [
            DebounceWindow::try_from(fractional),
            DebounceWindow::try_from(whole),
            DebounceWindow::try_from(huge),
        ];
        // then
        assert_eq!(
            results,
            [
                Err(DebounceWindowError::NotWholeMillis),
                Ok(DebounceWindow::MAX),
                Err(DebounceWindowError::NotWholeMillis),
            ]
        );
        assert_eq!(
            DebounceWindow::try_from(Duration::from_secs(u64::MAX)),
            Err(DebounceWindowError::TooLong)
        );
    }

    #[test]
    fn should_saturate_deadline_when_tick_is_near_max() {
        // given
        let near_max = Tick::from_millis(u64::MAX - 1);
        // when
        let later = near_max.saturating_add(DebounceWindow::DEFAULT);
        // then
        assert_eq!(later, Tick::from_millis(u64::MAX));
    }

    #[test]
    fn should_measure_zero_when_later_tick_is_not_after() {
        // given
        let t = Tick::from_millis(500);
        // when
        let forward = t.until(Tick::from_millis(650));
        let backward = t.until(Tick::from_millis(400));
        // then
        assert_eq!(forward, Duration::from_millis(150));
        assert_eq!(backward, Duration::ZERO);
    }

    #[test]
    fn should_render_window_in_milliseconds_when_displayed() {
        // given
        let window = DebounceWindow::DEFAULT;
        // when
        let text = window.to_string();
        let error = DebounceWindowError::TooLong.to_string();
        // then
        assert_eq!(text, "150ms");
        assert!(error.contains("10ms to 10s"));
    }

    #[test]
    fn should_return_the_origin_offset_when_tick_is_read() {
        // given
        let tick = Tick::from_millis(1234);

        // when
        let millis = tick.as_millis();

        // then
        assert_eq!(millis, 1234);
    }
}
