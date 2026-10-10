//! Positive fixture: domain code that names only allowed paths. An `Instant` passed in is a
//! value, not a clock read; `cfg` branches for every platform and test-only code are read.

mod clock;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub use clock::{Deadline, Span};

include!("generated.rs");

macro_rules! newtype {
    ($name:ident) => {
        pub struct $name(pub u64);
    };
}

newtype!(Revision);

pub fn remaining(now: Instant, deadline: Deadline) -> Duration {
    deadline.0.saturating_duration_since(now)
}

pub fn index(names: &[&str]) -> BTreeMap<usize, String> {
    names.iter().map(|n| n.to_string()).enumerate().collect()
}

#[cfg(target_os = "linux")]
pub const PLATFORM: &str = "linux";

#[cfg(not(target_os = "linux"))]
pub const PLATFORM: &str = "other";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_index_names_when_given_a_list() {
        // given
        let names = ["a"];

        // when
        let indexed = index(&names);

        // then
        assert_eq!(indexed.len(), std::env::args().count().min(1));
    }
}
