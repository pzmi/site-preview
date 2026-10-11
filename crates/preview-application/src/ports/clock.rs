//! The [`Clock`] port: the current instant as a domain [`Tick`].

use preview_domain::Tick;

/// The source of "now" for the debounce and the watcher pipeline.
///
/// The domain never reads a clock (ADR 0012); the application asks this port and passes the
/// [`Tick`] on.
///
/// # Contract
///
/// - **Monotonic.** Successive calls on one clock never go backwards, whatever happens to
///   the wall clock; a real clock is backed by a monotonic source, never by system time.
/// - **One origin.** Ticks are compared only with ticks of the same clock. Everything that
///   feeds one debouncer uses one clock.
/// - **Cheap and non-blocking.** `now` performs no I/O and may be called on any thread
///   (`Send + Sync`).
///
/// Synchronous and object safe, so it can be held as a generic or as `&dyn Clock`.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Tick;
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;
    use std::sync::Mutex;

    use super::*;

    /// A clock that advances by one millisecond per reading.
    struct Stepping(Mutex<Cell<u64>>);

    impl Clock for Stepping {
        fn now(&self) -> Tick {
            let guard = self.0.lock().expect("an uncontended test lock");
            let millis = guard.get();
            guard.set(millis + 1);
            Tick::from_millis(millis)
        }
    }

    #[test]
    fn should_read_monotonic_ticks_when_clock_is_a_trait_object() {
        // given
        let clock: &dyn Clock = &Stepping(Mutex::new(Cell::new(40)));
        // when
        let first = clock.now();
        let second = clock.now();
        // then
        assert!(first <= second);
        assert_eq!(first.as_millis(), 40);
    }
}
