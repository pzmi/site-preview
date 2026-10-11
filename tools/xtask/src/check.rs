//! `cargo xtask check`: the full blocking gate (foundation.md section 9, ADR 0019).
//!
//! It composes the registered stages: the `quick` stages other than its fast tests (format
//! check, architecture with fixtures, Clippy), everything `test` runs, then `security`. A stage that does
//! not exist yet is absent, never skipped; one that cannot run shows `UNAVAILABLE`.

use crate::process::{Stage, finish, run_report};
use crate::{quick, security, test, tools};

/// The stages `check` runs, in order. Their names are catalog ids with `in_check` set.
pub fn stages() -> Vec<Stage> {
    let mut stages: Vec<Stage> = quick::stages()
        .into_iter()
        .filter(|s| !matches!(s.name, "unit-tests" | "doctests"))
        .collect();
    stages.extend(test::stages());
    stages.extend(security::stages());
    stages
}

/// Runs `check`, prints the stage table and returns the exit code.
pub fn run() -> i32 {
    finish(&run_report(&stages(), &tools::require))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use std::collections::BTreeSet;

    #[test]
    fn should_run_exactly_the_in_check_stages_when_check_is_built() {
        // given
        let built: BTreeSet<_> = stages().iter().map(|s| s.name).collect();

        // when
        let registered: BTreeSet<_> = catalog::CATALOG
            .iter()
            .filter(|c| c.in_check)
            .map(|c| c.id)
            .collect();

        // then
        assert_eq!(built, registered);
    }

    #[test]
    fn should_run_each_stage_once_when_check_is_built() {
        // given
        let stages = stages();

        // when
        let mut names: Vec<_> = stages.iter().map(|s| s.name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();

        // then
        assert_eq!(names.len(), total);
    }

    #[test]
    fn should_include_architecture_and_tests_when_check_is_built() {
        // given
        let stages = stages();

        // when
        let names: Vec<_> = stages.iter().map(|s| s.name).collect();

        // then
        for want in [
            "fmt-check",
            "architecture",
            "clippy",
            "unit-tests",
            "doctests",
            "deny",
            "secrets",
            "workflows",
            "action-pins",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
    }
}
