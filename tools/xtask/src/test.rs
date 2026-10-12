//! `cargo xtask test`: every ordinary test level. Today that is the workspace tests under the
//! `ci` nextest profile and the doctests; the `client-js` stage runs the dependency-free `node --test` suites beside the embedded
//! scripts; the `browser` stage joins here when its task registers it (D18, ADR 0019).

use crate::process::{Stage, finish, run_report};
use crate::tools;

/// Generated cases per property under the `ci` profile (foundation.md section 5).
const PROPTEST_CASES_CI: &str = "1024";

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
        .needs("cargo-nextest")
        // nextest 0.9.148 has no profile-level env table, so the case count is set here.
        .env("PROPTEST_CASES", PROPTEST_CASES_CI)
        // A config key nextest ignores is a silent no-op; fail closed on it.
        .deny_stderr("ignoring unknown configuration key"),
        Stage::new(
            "doctests",
            "cargo",
            &["test", "--locked", "--workspace", "--doc"],
        ),
        Stage::new(
            "client-js",
            "node",
            &["--test", "crates/*/assets/*.test.mjs"],
        )
        .needs("node"),
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
        assert_eq!(names, ["unit-tests", "doctests", "client-js"]);
        assert!(format!("{:?}", stages[0].action).contains("\"ci\""));
        assert_eq!(stages[0].tool, Some("cargo-nextest"));
    }

    #[test]
    fn should_set_proptest_cases_and_deny_config_warnings_when_unit_stage_is_built() {
        // given
        let stages = stages();

        // when
        let debug = format!("{:?}", stages[0].action);

        // then
        assert!(debug.contains("(\"PROPTEST_CASES\", \"1024\")"), "{debug}");
        assert!(
            debug.contains("ignoring unknown configuration key"),
            "{debug}"
        );
    }

    #[test]
    fn should_require_node_when_client_js_stage_is_built() {
        // given
        let stages = stages();

        // when
        let stage = stages.iter().find(|s| s.name == "client-js").unwrap();

        // then
        assert_eq!(stage.tool, Some("node"));
        assert!(format!("{:?}", stage.action).contains("--test"));
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
