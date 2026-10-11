//! Deterministic [`InstanceId`]s.

use preview_domain::InstanceId;

/// An endless sequence of valid instance ids: `<pid>-<n>` for `n` counting up from the start
/// value, so two runs of a test see the same ids and two ids in one run differ.
#[derive(Debug, Clone)]
pub struct FixedInstanceIds {
    pid: u32,
    next_nanos: u128,
}

impl FixedInstanceIds {
    /// Ids for process id `pid`, the first with startup nanoseconds `first_nanos`.
    #[must_use]
    pub fn new(pid: u32, first_nanos: u128) -> Self {
        Self {
            pid,
            next_nanos: first_nanos,
        }
    }

    /// The next id; the same as `Iterator::next`, without the `Option`.
    pub fn next_id(&mut self) -> InstanceId {
        let id = InstanceId::from_parts(self.pid, self.next_nanos);
        self.next_nanos = self.next_nanos.saturating_add(1);
        id
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
        Some(self.next_id())
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
    fn should_stay_valid_when_nanos_are_maximal() {
        // given
        let mut ids = FixedInstanceIds::new(u32::MAX, u128::MAX);
        // when
        let (first, second) = (ids.next_id(), ids.next_id());
        // then
        assert!(InstanceId::parse(first.as_str()).is_ok());
        assert!(InstanceId::parse(second.as_str()).is_ok());
    }
}
