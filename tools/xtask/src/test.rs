//! `cargo xtask test`: every ordinary test level. Today that is the workspace tests under the
//! `ci` nextest profile and the doctests; the `client-js` stage runs the dependency-free `node --test` suites beside the embedded
//! scripts; the `browser` stage runs the Playwright specs (E2E-04).

use crate::process::{Stage, finish, run_report};
use crate::{browser, tools};

/// Generated cases per property under the `ci` profile (foundation.md section 5): the floor.
const PROPTEST_CASES_CI: u32 = 1024;

/// The case count the unit-tests stage passes on: the developer's `PROPTEST_CASES` when it
/// raises the floor, otherwise the floor. A lower or unparsable value never weakens the gate.
/// Pinned by `should_keep_developer_proptest_cases_when_set_at_or_above_the_floor`.
fn proptest_cases(developer: Option<&str>) -> String {
    developer
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|cases| *cases >= PROPTEST_CASES_CI)
        .unwrap_or(PROPTEST_CASES_CI)
        .to_string()
}

/// The stages `test` runs, in order. Their names are catalog ids.
pub fn stages() -> Vec<Stage> {
    stages_with(std::env::var("PROPTEST_CASES").ok().as_deref())
}

/// [`stages`] with the developer's `PROPTEST_CASES`, if any, given.
fn stages_with(developer_cases: Option<&str>) -> Vec<Stage> {
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
                // The conformance replay runs once, in `conformance` (ADR 0060).
                "--exclude",
                "preview-conformance",
            ],
        )
        .needs("cargo-nextest")
        // nextest 0.9.148 has no profile-level env table, so the case count is set here.
        .env("PROPTEST_CASES", &proptest_cases(developer_cases))
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
        browser::stage(),
    ]
}

/// The stages to run: all, or only the one named by `--only`. An unknown name is an error
/// that lists the valid ones, never an empty green run.
fn select(all: Vec<Stage>, only: Option<&str>) -> Result<Vec<Stage>, String> {
    let Some(name) = only else { return Ok(all) };
    if !all.iter().any(|s| s.name == name) {
        let names: Vec<_> = all.iter().map(|s| s.name).collect();
        return Err(format!(
            "no test stage named {name}; choose one of {}",
            names.join(", ")
        ));
    }
    Ok(all.into_iter().filter(|s| s.name == name).collect())
}

/// Runs `test` (or one of its stages) and returns the exit code.
pub fn run(only: Option<&str>) -> i32 {
    match select(stages(), only) {
        Ok(stages) => finish(&run_report(&stages, &tools::require)),
        Err(err) => {
            eprintln!("xtask: {err}");
            2
        }
    }
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
        assert_eq!(names, ["unit-tests", "doctests", "client-js", "browser"]);
        assert!(format!("{:?}", stages[0].action).contains("\"ci\""));
        assert_eq!(stages[0].tool, Some("cargo-nextest"));
    }

    #[test]
    fn should_set_proptest_cases_and_deny_config_warnings_when_unit_stage_is_built() {
        // given
        let stages = stages_with(None);

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
    fn should_keep_developer_proptest_cases_when_set_at_or_above_the_floor() {
        // given
        let values = [None, Some("1024"), Some("4096"), Some("16"), Some("many")];

        // when
        let cases = values.map(proptest_cases);
        let stage = format!("{:?}", stages_with(Some("4096"))[0].action);

        // then
        assert_eq!(cases, ["1024", "1024", "4096", "1024", "1024"]);
        assert!(stage.contains("(\"PROPTEST_CASES\", \"4096\")"), "{stage}");
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
    fn should_keep_only_the_named_stage_when_only_is_given() {
        // given
        let all = stages();

        // when
        let picked = select(all, Some("browser")).unwrap();

        // then
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].name, "browser");
    }

    #[test]
    fn should_list_valid_stages_when_only_names_an_unknown_one() {
        // given
        let all = stages();

        // when
        let err = select(all, Some("nope")).unwrap_err();

        // then
        assert!(
            err.contains("unit-tests") && err.contains("browser"),
            "{err}"
        );
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
