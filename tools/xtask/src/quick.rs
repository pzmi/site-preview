//! `cargo xtask quick`: formatter check, Clippy, the unit/component tests and the doctests.
//!
//! This is not the full gate; `check` (OPS-01) is.

use crate::process::{Stage, run_stages};
use crate::tools;

/// The stages `quick` runs, in order. Their ids match the `fmt-check`, `clippy`,
/// `unit-tests` and `doctests` catalog rows.
pub fn stages() -> Vec<Stage> {
    vec![
        Stage::new("fmt-check", "cargo", &["fmt", "--all", "--check"]),
        Stage::new(
            "clippy",
            "cargo",
            &[
                "clippy",
                "--locked",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        Stage::new(
            "unit-tests",
            "cargo",
            &["nextest", "run", "--locked", "--workspace"],
        ),
        Stage::new(
            "doctests",
            "cargo",
            &["test", "--locked", "--workspace", "--doc"],
        ),
    ]
}

/// Runs `quick` and returns the exit code.
pub fn run() -> i32 {
    if let Err(err) = tools::require("cargo-nextest") {
        eprintln!("xtask: {err}");
        return 1;
    }
    run_stages(&stages())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    #[test]
    fn should_run_only_cataloged_stages_when_quick_is_built() {
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
