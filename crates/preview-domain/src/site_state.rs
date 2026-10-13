//! Server state published to browsers: which server instance, and which revision of it.
//!
//! The domain has no clock and no randomness (D11), so an [`InstanceId`] is built from values
//! the caller supplies (process id, startup nanoseconds). It identifies one server instance,
//! never a visitor.

use core::fmt;

const INSTANCE_ID_MAX_LEN: usize = 64;

/// Why an [`InstanceId`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceIdError {
    /// The id is empty.
    Empty,
    /// The id is longer than 64 bytes.
    TooLong,
    /// The id holds a byte outside `[A-Za-z0-9-]`.
    InvalidCharacter,
}

impl fmt::Display for InstanceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "instance id is empty",
            Self::TooLong => "instance id is longer than 64 characters",
            Self::InvalidCharacter => "instance id contains a character outside [A-Za-z0-9-]",
        })
    }
}

impl core::error::Error for InstanceIdError {}

/// Non-secret identifier of one server process start, `[A-Za-z0-9-]{1,64}`.
///
/// It lets a browser tell a restarted server from a revision reset (plan §7.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstanceId(String);

impl InstanceId {
    /// Validates `value` against `[A-Za-z0-9-]{1,64}`.
    ///
    /// Pinned by `should_refuse_instance_id_when_empty_too_long_or_invalid_character`.
    ///
    /// # Errors
    /// Returns the first rule the value breaks.
    pub fn parse(value: &str) -> Result<Self, InstanceIdError> {
        if value.is_empty() {
            return Err(InstanceIdError::Empty);
        }
        if value.len() > INSTANCE_ID_MAX_LEN {
            return Err(InstanceIdError::TooLong);
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(InstanceIdError::InvalidCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    /// Builds `"<pid>-<startup_nanos>"` from caller-supplied parts; the domain reads no clock.
    ///
    /// Always valid: at most 10 + 1 + 39 characters. Pinned by
    /// `should_build_valid_id_when_parts_are_maximal`.
    #[must_use]
    pub fn from_parts(pid: u32, startup_nanos: u128) -> Self {
        Self(format!("{pid}-{startup_nanos}"))
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Refusal: the revision counter is at `u64::MAX` and cannot advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionExhausted;

impl fmt::Display for RevisionExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("revision counter is exhausted")
    }
}

impl core::error::Error for RevisionExhausted {}

/// Monotonic change counter of one server instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    /// The revision of a freshly started instance.
    pub const ZERO: Self = Self(0);

    /// The numeric value, as sent on the wire.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The following revision.
    ///
    /// Never wraps: at `u64::MAX` it refuses (pinned by
    /// `should_refuse_next_when_revision_is_max`).
    ///
    /// # Errors
    /// [`RevisionExhausted`] at `u64::MAX`.
    pub const fn next(self) -> Result<Self, RevisionExhausted> {
        match self.0.checked_add(1) {
            Some(n) => Ok(Self(n)),
            None => Err(RevisionExhausted),
        }
    }
}

impl From<u64> for Revision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// The latest state of the served site: instance and revision (plan §7.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SiteState {
    instance_id: InstanceId,
    revision: Revision,
}

impl SiteState {
    /// State of a new instance, at [`Revision::ZERO`].
    #[must_use]
    pub fn initial(instance_id: InstanceId) -> Self {
        Self {
            instance_id,
            revision: Revision::ZERO,
        }
    }

    /// Rebuilds a state from known parts (for example one read off the wire).
    #[must_use]
    pub fn new(instance_id: InstanceId, revision: Revision) -> Self {
        Self {
            instance_id,
            revision,
        }
    }

    /// The same instance one revision later.
    ///
    /// # Errors
    /// [`RevisionExhausted`] when the revision is `u64::MAX`.
    pub fn advanced(&self) -> Result<Self, RevisionExhausted> {
        Ok(Self {
            instance_id: self.instance_id.clone(),
            revision: self.revision.next()?,
        })
    }

    /// The instance id.
    #[must_use]
    pub fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// The revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// True when the instance or the revision differs, which is when a browser must reload.
    #[must_use]
    pub fn differs(&self, other: &Self) -> bool {
        self.instance_id != other.instance_id || self.revision != other.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> InstanceId {
        InstanceId::parse(s).expect("valid id")
    }

    #[test]
    fn should_report_difference_when_instance_changes_with_same_revision() {
        // given
        let a = SiteState::new(id("a-1"), Revision::from(3));
        let b = SiteState::new(id("b-2"), Revision::from(3));
        // when
        let differs = a.differs(&b);
        // then
        assert!(differs);
    }

    #[test]
    fn should_report_difference_when_revision_advances() {
        // given
        let a = SiteState::initial(id("a-1"));
        // when
        let b = a.advanced().expect("not exhausted");
        // then
        assert!(a.differs(&b));
        assert_eq!(b.revision().get(), 1);
        assert_eq!(b.instance_id(), a.instance_id());
    }

    #[test]
    fn should_report_no_difference_when_states_are_equal() {
        // given
        let a = SiteState::new(id("a-1"), Revision::from(7));
        let b = a.clone();
        // when
        let differs = a.differs(&b);
        // then
        assert!(!differs);
    }

    #[test]
    fn should_start_at_zero_when_state_is_initial() {
        // given
        let instance = id("a-1");
        // when
        let state = SiteState::initial(instance);
        // then
        assert_eq!(state.revision(), Revision::ZERO);
    }

    #[test]
    fn should_refuse_next_when_revision_is_max() {
        // given
        let max = Revision::from(u64::MAX);
        // when
        let next = max.next();
        // then
        assert_eq!(next, Err(RevisionExhausted));
    }

    #[test]
    fn should_refuse_advance_when_revision_is_max() {
        // given
        let state = SiteState::new(id("a-1"), Revision::from(u64::MAX));
        // when
        let advanced = state.advanced();
        // then
        assert_eq!(advanced, Err(RevisionExhausted));
    }

    #[test]
    fn should_reach_max_when_advancing_from_one_below() {
        // given
        let r = Revision::from(u64::MAX - 1);
        // when
        let next = r.next();
        // then
        assert_eq!(next, Ok(Revision::from(u64::MAX)));
    }

    #[test]
    fn should_refuse_instance_id_when_empty_too_long_or_invalid_character() {
        // given
        let long = "a".repeat(65);
        // when
        let results = [
            InstanceId::parse(""),
            InstanceId::parse(&long),
            InstanceId::parse("a b"),
            InstanceId::parse("a_b"),
            InstanceId::parse("é"),
        ];
        // then
        assert_eq!(results[0], Err(InstanceIdError::Empty));
        assert_eq!(results[1], Err(InstanceIdError::TooLong));
        assert_eq!(results[2], Err(InstanceIdError::InvalidCharacter));
        assert_eq!(results[3], Err(InstanceIdError::InvalidCharacter));
        assert_eq!(results[4], Err(InstanceIdError::InvalidCharacter));
    }

    #[test]
    fn should_accept_instance_id_when_exactly_sixty_four_valid_characters() {
        // given
        let value = format!("{}-Z9", "a".repeat(61));
        // when
        let parsed = InstanceId::parse(&value);
        // then
        assert_eq!(parsed.map(|i| i.to_string()), Ok(value));
    }

    #[test]
    fn should_build_valid_id_when_parts_are_maximal() {
        // given
        let (pid, nanos) = (u32::MAX, u128::MAX);
        // when
        let built = InstanceId::from_parts(pid, nanos);
        // then
        assert_eq!(InstanceId::parse(built.as_str()).as_ref(), Ok(&built));
    }

    #[test]
    fn should_format_id_as_pid_dash_nanos_when_built_from_parts() {
        // given
        let (pid, nanos) = (42, 1_700_000_000_123);
        // when
        let built = InstanceId::from_parts(pid, nanos);
        // then
        assert_eq!(built.as_str(), "42-1700000000123");
    }

    #[test]
    fn should_expose_the_number_when_revision_is_read() {
        // given
        let revision = Revision::from(7);

        // when
        let value = revision.get();

        // then
        assert_eq!((Revision::ZERO.get(), value), (0, 7));
    }

    #[test]
    fn should_describe_the_refusal_when_an_error_is_displayed() {
        // given
        let errors = [
            InstanceIdError::Empty.to_string(),
            InstanceIdError::TooLong.to_string(),
            InstanceIdError::InvalidCharacter.to_string(),
            RevisionExhausted.to_string(),
        ];

        // when
        let distinct: std::collections::BTreeSet<_> = errors.iter().collect();

        // then
        assert_eq!(distinct.len(), 4, "{errors:?}");
        assert_eq!(errors[3], "revision counter is exhausted");
        assert_eq!(errors[0], "instance id is empty");
    }
}
