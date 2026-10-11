//! `cargo xtask security`: the dependency policy stage (foundation.md section 8, D23).
//!
//! Four stages: `cargo deny --locked check` against the root `deny.toml` (advisories,
//! licences, bans, sources), Gitleaks over the whole git history, `actionlint` over the
//! workflows, and the in-process check that every `uses:` is pinned to a full commit SHA.
//! A missing pinned tool is `UNAVAILABLE`, never a pass (D9).

use crate::action_pins;
use crate::process::{Stage, finish, run_report};
use crate::tools;

/// The stages `security` runs, in order. Their names are catalog ids.
pub fn stages() -> Vec<Stage> {
    vec![
        deny_stage(),
        secrets_stage(),
        Stage::new("workflows", "actionlint", &[]).needs("actionlint"),
        Stage::in_process(
            "action-pins",
            "check .github/workflows and .github/actions for unpinned uses:",
            action_pins::run,
        ),
    ]
}

/// History scan: every commit, findings redacted so a real secret never reaches a log.
fn secrets_stage() -> Stage {
    Stage::new(
        "secrets",
        "gitleaks",
        &[
            "git",
            "--no-banner",
            "--redact",
            "--config",
            ".gitleaks.toml",
        ],
    )
    .needs("gitleaks")
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
        assert_eq!(stages.len(), 4);
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
