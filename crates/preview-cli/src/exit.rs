//! Mapping from the outcome of a run to the process exit code.

use std::process::ExitCode;

/// How a run of the binary ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The command completed.
    Success,
    /// The command could not run: an unusable root, a bind failure or a fatal server error.
    Failure,
    /// The command line was invalid.
    Usage,
    /// A second SIGINT cut the shutdown drain short: 128 + 2, as a shell reports it.
    Interrupted,
    /// A second SIGTERM cut the shutdown drain short: 128 + 15.
    Terminated,
}

impl Outcome {
    /// The numeric exit code: 0 for success, 1 for a failure, 2 for a usage error, 128 plus
    /// the signal number for a forced stop (ADR 0045).
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Usage => 2,
            Self::Interrupted => 130,
            Self::Terminated => 143,
        }
    }
}

impl From<Outcome> for ExitCode {
    fn from(outcome: Outcome) -> Self {
        Self::from(outcome.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_map_outcomes_to_0_1_2_130_143_when_asked_for_codes() {
        // given
        let outcomes = [
            Outcome::Success,
            Outcome::Failure,
            Outcome::Usage,
            Outcome::Interrupted,
            Outcome::Terminated,
        ];
        // when
        let codes = outcomes.map(Outcome::code);
        // then
        assert_eq!(codes, [0, 1, 2, 130, 143]);
    }
}
