//! `cargo xtask check`: the full blocking gate (foundation.md section 9, ADR 0019).
//!
//! It composes the registered stages: the `quick` stages other than its fast tests (format
//! checks of the workspace and the fuzz targets, architecture with fixtures, Clippy),
//! everything `test` runs, everything `conformance` runs (FORMAL-09, ADR 0060), then
//! `coverage` (TEST-06, ADR 0062) and the `changed-lines` gate over its report (TEST-07, ADR
//! 0064), then `security`. A stage that does not exist yet is absent, never skipped; one that
//! cannot run shows `UNAVAILABLE`.

use crate::process::{Stage, finish, run_report};
use crate::{changed_lines, conformance, coverage, docs, quick, security, test, tools};

/// The stages `check` runs, in order. Their names are catalog ids with `in_check` set.
pub fn stages() -> Vec<Stage> {
    let mut stages: Vec<Stage> = quick::stages()
        .into_iter()
        .filter(|s| !matches!(s.name, "unit-tests" | "doctests"))
        .collect();
    stages.push(docs::stage());
    stages.extend(test::stages());
    stages.extend(conformance::stages(false));
    stages.push(coverage::stage());
    stages.push(changed_lines::stage());
    stages.extend(security::stages());
    stages
}

/// The stages to run: all, or those named by `--only`, in the gate's own order (so `coverage`
/// precedes `changed-lines` however the flags are ordered). An unknown name is an error that
/// lists the valid ones, never an empty green run.
fn select(all: Vec<Stage>, only: &[String]) -> Result<Vec<Stage>, String> {
    if only.is_empty() {
        return Ok(all);
    }
    let unknown: Vec<_> = only
        .iter()
        .filter(|name| !all.iter().any(|s| s.name == name.as_str()))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        let names: Vec<_> = all.iter().map(|s| s.name).collect();
        return Err(format!(
            "no check stage named {}; choose from {}",
            unknown.join(", "),
            names.join(", ")
        ));
    }
    Ok(all
        .into_iter()
        .filter(|s| only.iter().any(|name| name == s.name))
        .collect())
}

/// Runs `check` (or the stages named by `--only`), prints the stage table and returns the
/// exit code: 2 for an unknown stage name.
pub fn run(only: &[String]) -> i32 {
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
    use std::collections::{BTreeMap, BTreeSet};

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
            "fuzz-fmt",
            "architecture",
            "clippy",
            "unit-tests",
            "integration-tests",
            "doctests",
            "formal-typecheck",
            "formal-scenarios",
            "formal-simulate",
            "formal-freshness",
            "formal-generate",
            "conformance-replay",
            "conformance-controls",
            "coverage",
            "changed-lines",
            "deny",
            "secrets",
            "workflows",
            "action-pins",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
    }

    #[test]
    fn should_keep_every_stage_in_check_order_when_only_names_some() {
        // given
        let only = ["changed-lines".to_owned(), "coverage".to_owned()];

        // when
        let picked = select(stages(), &only).unwrap();

        // then
        let names: Vec<_> = picked.iter().map(|s| s.name).collect();
        assert_eq!(names, ["coverage", "changed-lines"]);
    }

    #[test]
    fn should_run_every_stage_when_only_is_empty() {
        // given
        let total = stages().len();

        // when
        let picked = select(stages(), &[]).unwrap();

        // then
        assert_eq!(picked.len(), total);
    }

    #[test]
    fn should_list_valid_stages_when_only_names_an_unknown_one() {
        // given
        let only = ["coverage".to_owned(), "nope".to_owned()];

        // when
        let err = select(stages(), &only).unwrap_err();

        // then
        assert!(
            err.contains("nope") && err.contains("clippy") && err.contains("coverage"),
            "{err}"
        );
    }

    #[test]
    fn should_exit_two_when_only_names_an_unknown_stage() {
        // given
        let only = ["nope".to_owned()];

        // when
        let code = run(&only);

        // then
        assert_eq!(code, 2);
    }

    #[test]
    fn should_parse_only_flags_repeated_and_comma_separated_when_check_is_invoked() {
        use crate::cli::{Cli, Command};
        use clap::Parser;

        // given
        let args = ["xtask", "check", "--only", "a,b", "--only", "c"];

        // when
        let cli = Cli::try_parse_from(args).unwrap();

        // then
        let Command::Check { only } = cli.command else {
            panic!("not check")
        };
        assert_eq!(only, ["a", "b", "c"]);
    }

    /// The `cargo xtask check --only` stages of each job of `ci.yml`, and each job's `needs`.
    fn ci_jobs() -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
        let text = std::fs::read_to_string(crate::repo::root().join(".github/workflows/ci.yml"))
            .expect("ci.yml");
        let mut stages: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut needs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut in_jobs = false;
        let mut job = String::new();
        let mut in_needs = false;
        for line in text.lines() {
            if line == "jobs:" {
                in_jobs = true;
            } else if in_jobs && !line.starts_with(' ') && !line.is_empty() {
                in_jobs = false;
            }
            if !in_jobs {
                continue;
            }
            if let Some(id) = line
                .strip_prefix("  ")
                .and_then(|l| l.strip_suffix(':'))
                .filter(|l| !l.starts_with([' ', '#']))
            {
                job = id.to_owned();
                stages.entry(job.clone()).or_default();
                needs.entry(job.clone()).or_default();
                in_needs = false;
            } else if let Some(args) = line.split("cargo xtask check --only ").nth(1) {
                // `--only a,b` and `--only a --only b` both name stages
                let args = args.replace("--only ", "");
                let list = stages.get_mut(&job).unwrap();
                list.extend(
                    args.split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned),
                );
            } else if line.trim() == "needs:" {
                in_needs = true;
            } else if in_needs && let Some(name) = line.trim().strip_prefix("- ") {
                needs.get_mut(&job).unwrap().push(name.to_owned());
            } else {
                in_needs = false;
            }
        }
        (stages, needs)
    }

    #[test]
    fn should_cover_each_in_check_stage_once_when_ci_jobs_are_read() {
        // given
        let (jobs, _) = ci_jobs();
        let mut count: BTreeMap<&str, usize> = catalog::CATALOG
            .iter()
            .filter(|c| c.in_check)
            .map(|c| (c.id, 0))
            .collect();

        // when
        let mut unknown = Vec::new();
        for (job, names) in &jobs {
            for name in names {
                match count.get_mut(name.as_str()) {
                    Some(n) => *n += 1,
                    None => unknown.push(format!("{job}: {name}")),
                }
            }
        }

        // then
        assert!(unknown.is_empty(), "not in_check stages: {unknown:?}");
        let wrong: Vec<_> = count.iter().filter(|(_, n)| **n != 1).collect();
        assert!(
            wrong.is_empty(),
            "stages not in exactly one ci.yml job: {wrong:?}"
        );
    }

    #[test]
    fn should_need_every_other_job_when_required_job_is_read() {
        // given
        let (jobs, needs) = ci_jobs();

        // when
        let mut want: Vec<_> = jobs.keys().filter(|j| *j != "required").cloned().collect();
        let mut got = needs.get("required").cloned().unwrap_or_default();
        want.sort();
        got.sort();

        // then
        assert!(jobs.contains_key("required"), "{jobs:?}");
        assert_eq!(got, want);
    }

    #[test]
    fn should_run_the_platform_job_on_the_four_native_runners_when_ci_is_read() {
        // given
        let text = std::fs::read_to_string(crate::repo::root().join(".github/workflows/ci.yml"))
            .expect("ci.yml");

        // when
        let matrix = text
            .lines()
            .find(|l| l.trim_start().starts_with("os: ["))
            .expect("the platform matrix");

        // then
        for runner in [
            "ubuntu-24.04",
            "ubuntu-24.04-arm",
            "macos-15",
            "macos-15-intel",
        ] {
            assert!(
                matrix.contains(&format!("{runner},")) || matrix.contains(&format!("{runner}]")),
                "{runner}: {matrix}"
            );
        }
        assert!(text.contains("cargo xtask test --only platform-tests"));
        assert!(
            !text.to_ascii_lowercase().contains("windows"),
            "D2: no Windows"
        );
    }
}
