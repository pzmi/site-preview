//! Deterministic [`InstanceId`]s.

use preview_domain::InstanceId;

/// A sequence of valid instance ids: `<pid>-<n>` for `n` counting up from the start value,
/// so two runs of a test see the same ids and two ids in one run never repeat. The sequence
/// ends after `n = u128::MAX`: the iterator then yields `None` and [`Self::next_id`] panics,
/// rather than handing out a duplicate.
#[derive(Debug, Clone)]
pub struct FixedInstanceIds {
    pid: u32,
    next_nanos: Option<u128>,
}

impl FixedInstanceIds {
    /// Ids for process id `pid`, the first with startup nanoseconds `first_nanos`.
    #[must_use]
    pub fn new(pid: u32, first_nanos: u128) -> Self {
        Self {
            pid,
            next_nanos: Some(first_nanos),
        }
    }

    /// The next id; the same as `Iterator::next`, without the `Option`.
    ///
    /// # Panics
    ///
    /// When the sequence is exhausted (after `n = u128::MAX`): a test must never be handed
    /// the same id twice (`should_panic_when_next_id_is_taken_after_exhaustion`).
    pub fn next_id(&mut self) -> InstanceId {
        self.next()
            .expect("FixedInstanceIds is exhausted: no id is ever handed out twice")
    }
}

impl Default for FixedInstanceIds {
    /// Process id 1, counting from 1.
    fn default() -> Self {
        Self::new(1, 1)
    }
}

impl Iterator for FixedInstanceIds {
    type Item = InstanceId;

    fn next(&mut self) -> Option<InstanceId> {
        let nanos = self.next_nanos?;
        self.next_nanos = nanos.checked_add(1);
        Some(InstanceId::from_parts(self.pid, nanos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_repeat_sequence_when_built_with_same_parts() {
        // given
        let mut first = FixedInstanceIds::new(42, 100);
        let mut second = FixedInstanceIds::new(42, 100);
        // when
        let a: Vec<_> = (0..3).map(|_| first.next_id()).collect();
        let b: Vec<_> = (0..3).map(|_| second.next_id()).collect();
        // then
        assert_eq!(a, b);
        assert_eq!(a[0].as_str(), "42-100");
    }

    #[test]
    fn should_yield_distinct_valid_ids_when_called_repeatedly() {
        // given
        let ids = FixedInstanceIds::default();
        // when
        let taken: Vec<_> = ids.take(4).collect();
        // then
        for (i, id) in taken.iter().enumerate() {
            assert_eq!(InstanceId::parse(id.as_str()).as_ref(), Ok(id));
            assert!(taken[..i].iter().all(|earlier| earlier != id));
        }
    }

    #[test]
    fn should_end_sequence_when_nanos_are_maximal() {
        // given
        let mut ids = FixedInstanceIds::new(u32::MAX, u128::MAX);
        // when
        let (last, after) = (ids.next(), ids.next());
        // then
        let last = last.expect("the maximal value is still handed out once");
        assert!(InstanceId::parse(last.as_str()).is_ok());
        assert_eq!(after, None);
        assert_eq!(ids.next(), None, "an ended sequence stays ended");
    }

    #[test]
    #[should_panic(expected = "exhausted")]
    fn should_panic_when_next_id_is_taken_after_exhaustion() {
        // given
        let mut ids = FixedInstanceIds::new(1, u128::MAX);
        ids.next_id();
        // when
        let _ = ids.next_id();
        // then: unreachable, the call above panics
    }
}
