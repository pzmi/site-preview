//! `cargo xtask security`: the dependency policy stage (foundation.md section 8, D23).
//!
//! Today that is `cargo deny --locked check` against the root `deny.toml`: advisories,
//! licences, bans and sources. Secret scanning and workflow linting join here when SEC-02
//! registers them (D18, ADR 0019). A missing `cargo-deny` is `UNAVAILABLE`, never a pass (D9).

use crate::process::{Stage, finish, run_report};
use crate::tools;

/// The stages `security` runs, in order. Their names are catalog ids.
pub fn stages() -> Vec<Stage> {
    vec![deny_stage()]
}

fn deny_stage() -> Stage {
    Stage::new("security", "cargo", &["deny", "--locked", "check"]).needs("cargo-deny")
}

/// Runs `security` and returns the exit code.
pub fn run() -> i32 {
    finish(&run_report(&stages(), &tools::require))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    #[test]
    fn should_run_locked_deny_check_when_security_is_built() {
        // given
        let stages = stages();

        // when
        let rendered = format!("{:?}", stages[0].action);

        // then
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].tool, Some("cargo-deny"));
        for want in ["\"deny\"", "\"--locked\"", "\"check\""] {
            assert!(rendered.contains(want), "{want} missing from {rendered}");
        }
    }

    #[test]
    fn should_run_only_cataloged_stages_when_security_is_built() {
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
