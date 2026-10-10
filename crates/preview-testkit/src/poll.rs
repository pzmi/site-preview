//! The only sanctioned polling primitive for tests.
//!
//! Deterministic tests never poll or sleep: they use injected clocks, channels or readiness
//! signals. Only tests that observe the real OS (file watcher events, sockets, subprocesses)
//! may wait for a condition, and only through [`wait_until`], which bounds the wait with a
//! deadline and names what was awaited. Timing is never a correctness oracle: the deadline
//! only limits how long a failing test hangs.

use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

/// Pause between two evaluations of the condition.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The condition did not become true before the deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeout {
    /// What the caller was waiting for.
    pub reason: String,
    /// The deadline that expired.
    pub deadline: Duration,
}

impl fmt::Display for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "timed out after {:?} waiting for: {}",
            self.deadline, self.reason
        )
    }
}

impl std::error::Error for Timeout {}

/// Polls `condition` until it returns true, or fails once `deadline` has elapsed.
///
/// `reason` says what is awaited and appears in the error, so a failure explains itself. The
/// condition is always evaluated at least once, and once more after the deadline passes.
///
/// ```
/// use std::time::Duration;
/// use preview_testkit::poll::wait_until;
///
/// let mut calls = 0;
/// wait_until(Duration::from_secs(5), "third call", || {
///     calls += 1;
///     calls == 3
/// })
/// .unwrap();
/// ```
///
/// # Errors
///
/// Returns [`Timeout`] carrying `reason` when the condition is still false at the deadline.
pub fn wait_until(
    deadline: Duration,
    reason: &str,
    mut condition: impl FnMut() -> bool,
) -> Result<(), Timeout> {
    let start = Instant::now();
    loop {
        if condition() {
            return Ok(());
        }
        if start.elapsed() >= deadline {
            return Err(Timeout {
                reason: reason.to_owned(),
                deadline,
            });
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_return_ok_when_condition_is_already_true() {
        // given
        let mut calls = 0;

        // when
        let result = wait_until(Duration::from_secs(5), "immediately true", || {
            calls += 1;
            true
        });

        // then
        assert_eq!(result, Ok(()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn should_return_ok_when_condition_becomes_true_before_deadline() {
        // given
        let mut calls = 0;

        // when
        let result = wait_until(Duration::from_secs(5), "fourth call", || {
            calls += 1;
            calls >= 4
        });

        // then
        assert_eq!(result, Ok(()));
        assert_eq!(calls, 4);
    }

    #[test]
    fn should_report_reason_when_deadline_expires() {
        // given
        let deadline = Duration::from_millis(30);

        // when
        let err = wait_until(deadline, "the watcher reports a change", || false).unwrap_err();

        // then
        assert_eq!(err.reason, "the watcher reports a change");
        assert_eq!(err.deadline, deadline);
        assert!(
            err.to_string().contains("the watcher reports a change"),
            "{err}"
        );
    }

    #[test]
    fn should_evaluate_condition_once_when_deadline_is_zero() {
        // given
        let mut calls = 0;

        // when
        let result = wait_until(Duration::ZERO, "never true", || {
            calls += 1;
            false
        });

        // then
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
