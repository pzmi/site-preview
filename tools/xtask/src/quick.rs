//! `cargo xtask quick`: formatter check, the architecture policy (ADR 0019), Clippy, the
//! unit/component tests and the doctests.
//!
//! This is not the full gate; `check` is (see `check.rs`).

use crate::process::{Stage, run_stages};
use crate::{architecture, tools};

/// The stages `quick` runs, in order. Their names are catalog ids.
pub fn stages() -> Vec<Stage> {
    vec![
        Stage::new("fmt-check", "cargo", &["fmt", "--all", "--check"]),
        Stage::in_process("architecture", "cargo xtask architecture", || {
            architecture::run(None)
        }),
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
    fn should_include_architecture_when_quick_is_built() {
        // given
        let stages = stages();

        // when
        let names: Vec<_> = stages.iter().map(|s| s.name).collect();

        // then
        assert!(names.contains(&"architecture"), "{names:?}");
        assert!(catalog::find("architecture").is_some_and(|c| c.in_check));
    }

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
