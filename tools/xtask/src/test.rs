//! `cargo xtask test`: every ordinary test level. Today that is the workspace tests under the
//! `ci` nextest profile and the doctests; stages for `client-js` and `browser` join here when
//! the tasks that create them register them (D18, ADR 0019).

use crate::process::{Stage, finish, run_report};
use crate::tools;

/// The stages `test` runs, in order. Their names are catalog ids.
pub fn stages() -> Vec<Stage> {
    vec![
        Stage::new(
            "unit-tests",
            "cargo",
            &[
                "nextest",
                "run",
                "--workspace",
                "--locked",
                "--profile",
                "ci",
            ],
        )
        .needs("cargo-nextest"),
        Stage::new(
            "doctests",
            "cargo",
            &["test", "--locked", "--workspace", "--doc"],
        ),
    ]
}

/// Runs `test` and returns the exit code.
pub fn run() -> i32 {
    finish(&run_report(&stages(), &tools::require))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    #[test]
    fn should_use_ci_profile_and_doctests_when_test_is_built() {
        // given
        let stages = stages();

        // when
        let names: Vec<_> = stages.iter().map(|s| s.name).collect();

        // then
        assert_eq!(names, ["unit-tests", "doctests"]);
        assert!(format!("{:?}", stages[0].action).contains("\"ci\""));
        assert_eq!(stages[0].tool, Some("cargo-nextest"));
    }

    #[test]
    fn should_run_only_cataloged_stages_when_test_is_built() {
        // given
        let stages = stages();

        // when
        let uncataloged: Vec<_> = stages
            .iter()
            .filter(|s| catalog::find(s.name).is_none())
            .map(|s| s.name)
            .collect();

        // then
        assert!(uncataloged.is_empty(), "{uncataloged:?}");
    }
}
