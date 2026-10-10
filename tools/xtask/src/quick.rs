//! `cargo xtask quick`: formatter check, Clippy and the unit/component tests.
//!
//! This is not the full gate; `check` (OPS-01) is.

use crate::process::{Stage, run_stages};

/// The stages `quick` runs, in order. Their ids match the `fmt-check`, `clippy` and
/// `unit-tests` catalog rows.
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
        Stage::new("unit-tests", "cargo", &["test", "--locked", "--workspace"]),
    ]
}

/// Runs `quick` and returns the exit code.
pub fn run() -> i32 {
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
