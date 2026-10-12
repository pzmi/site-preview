//! `cargo xtask conformance`: the Quint models in `formal/` (foundation.md section 6, ADR 0052).
//!
//! FORMAL-01 introduces three stages; FORMAL-09 adds regression freshness, generated traces,
//! Rust replay and the negative controls, and registers the command in `check` (ADR 0019):
//!
//! - `formal-typecheck` (`--typecheck` runs only this one): `quint typecheck` on every
//!   `formal/*.qnt`;
//! - `formal-scenarios`: `quint test` on each module of `formal/scenarios.qnt`, matching the
//!   `run` names that module declares, and counting them: `quint test` passes when nothing
//!   matches, so a run that silently stops being collected must fail here;
//! - `formal-simulate`: `quint run` of each model under each of its step relations, checking
//!   every conjunct of the model's `val allInvariants = and { ... }` (the one list of its
//!   invariants, so a new one cannot be left out), over [`SAMPLES`] traces of at most
//!   [`MAX_STEPS`] steps. The seed is printed; `QUINT_SEED` repeats it.
//!
//! Quint is the version `formal/package-lock.json` locks (`tools/versions.toml` notes it),
//! run with `npx --prefix formal --no`, which never fetches a package from the registry, and
//! always with the TypeScript backend: the default Rust backend downloads an evaluator binary
//! from GitHub at first use, which no lockfile pins (ADR 0052). Node is a development tool
//! only; the shipped binary never needs it. A missing Node or Quint fails with the command
//! that installs it (ADR 0010), never a skip.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::process::{Stage, finish, run_report};
use crate::{repo, tools};

/// One Quint model and how it is simulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// File name in `formal/`; the main module has the same name without `.qnt`.
    pub file: &'static str,
    /// The step relations to simulate (`quint run --step`).
    pub steps: &'static [&'static str],
}

/// The models `formal-simulate` runs. FORMAL-02 and FORMAL-03 add `watcher` and `reload`.
pub const MODELS: &[Model] = &[Model {
    file: "lifecycle.qnt",
    steps: &["step", "serverStep"],
}];

/// The file of named runs, one module per model.
const SCENARIOS: &str = "scenarios.qnt";

/// Traces per simulation (FORMAL-01's acceptance budget).
pub const SAMPLES: u32 = 1000;

/// Steps per trace (foundation.md section 6.4's initial budget).
pub const MAX_STEPS: u32 = 25;

/// The evaluator that comes from the lockfile (ADR 0052).
const BACKEND: [&str; 2] = ["--backend", "typescript"];

/// The stages `conformance` runs, in order; `typecheck_only` keeps the first.
pub fn stages(typecheck_only: bool) -> Vec<Stage> {
    let mut stages = vec![
        Stage::in_process(
            "formal-typecheck",
            "npx --prefix formal --no -- quint typecheck formal/*.qnt",
            typecheck,
        )
        .needs("node"),
    ];
    if !typecheck_only {
        stages.push(
            Stage::in_process(
                "formal-scenarios",
                "npx --prefix formal --no -- quint test --backend typescript --main <module> --match <its runs> formal/scenarios.qnt",
                scenarios,
            )
            .needs("node"),
        );
        stages.push(
            Stage::in_process(
                "formal-simulate",
                "npx --prefix formal --no -- quint run --backend typescript --step <relation> --invariants <all> --max-samples 1000 --max-steps 25 formal/<model>.qnt",
                simulate,
            )
            .needs("node"),
        );
    }
    stages
}

/// Runs `conformance` (or only its typecheck) and returns the exit code.
pub fn run(typecheck_only: bool) -> i32 {
    finish(&run_report(&stages(typecheck_only), &tools::require))
}

/// `formal/` in the repository.
fn formal_dir() -> PathBuf {
    repo::root().join("formal")
}

/// The locally installed Quint, never one fetched from the registry: `--no` refuses to
/// install, and `--` keeps npx from taking Quint's own flags (`--version`) as its own.
fn quint(formal: &Path) -> Command {
    let mut command = Command::new("npx");
    command
        .arg("--prefix")
        .arg(formal)
        .args(["--no", "--", "quint"])
        .current_dir(formal);
    command
}

/// The pinned Quint version from `tools/versions.toml`.
fn pinned_quint() -> Result<String, String> {
    tools::manifest()?
        .into_iter()
        .find(|t| t.name == "quint")
        .map(|t| t.version)
        .ok_or_else(|| "tools/versions.toml has no quint entry".to_owned())
}

/// The D9 hint for a Quint that is not installed at `version`.
fn install_hint(version: &str) -> String {
    format!(
        "run 'npm ci --prefix formal' (Quint {version} is locked by formal/package-lock.json; \
         Node itself comes from https://nodejs.org/en/download)"
    )
}

/// Decides from `quint --version`'s result (`None`: npx could not run) whether the pinned
/// `version` is usable.
fn classify(probe: Option<(bool, &str)>, version: &str) -> Result<(), String> {
    match probe {
        None => Err(format!(
            "cannot run npx: missing Node or npm; {}",
            install_hint(version)
        )),
        Some((false, _)) => Err(format!(
            "missing quint {version} in formal/node_modules: {}",
            install_hint(version)
        )),
        Some((true, out)) if out.split_whitespace().any(|t| t == version) => Ok(()),
        Some((true, out)) => Err(format!(
            "wrong version of quint (found '{}', want {version}): {}",
            out.lines().next().unwrap_or("").trim(),
            install_hint(version)
        )),
    }
}

/// Fails with the D9 hint unless `formal/node_modules` holds the pinned Quint.
fn require_quint(formal: &Path) -> Result<(), String> {
    let version = pinned_quint()?;
    let probe = quint(formal).arg("--version").output().ok().map(|o| {
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    });
    classify(probe.as_ref().map(|(ok, t)| (*ok, t.as_str())), &version)
}

/// Prints a finished command's output and returns whether it exited 0.
fn echo(output: &Output) -> bool {
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    output.status.success()
}

/// Runs `command`, reporting a failure to start it as a failed stage.
fn output_of(mut command: Command, label: &str) -> Option<Output> {
    match command.output() {
        Ok(output) => Some(output),
        Err(err) => {
            eprintln!("xtask: cannot run {label}: {err}");
            None
        }
    }
}

/// Every `*.qnt` in `dir`, sorted.
fn models_in(dir: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read formal/: {e}"))?;
    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.ends_with(".qnt"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err("no .qnt file in formal/".to_owned());
    }
    Ok(files)
}

/// Prints `err` and returns the stage's failure code.
fn fail(err: &str) -> i32 {
    eprintln!("xtask: {err}");
    1
}

fn typecheck() -> i32 {
    let formal = formal_dir();
    if let Err(err) = require_quint(&formal) {
        return fail(&err);
    }
    let files = match models_in(&formal) {
        Ok(files) => files,
        Err(err) => return fail(&err),
    };
    let mut failed = Vec::new();
    for file in &files {
        let mut command = quint(&formal);
        command.args(["typecheck", file]);
        let ok = output_of(command, "quint typecheck").is_some_and(|o| echo(&o));
        if ok {
            println!("typecheck ok: formal/{file}");
        } else {
            failed.push(file.as_str());
        }
    }
    if failed.is_empty() {
        0
    } else {
        fail(&format!("quint typecheck failed: {}", failed.join(", ")))
    }
}

/// The `run` names of each module in a scenarios file, in order. A module without a run is
/// an error, so a file that stops declaring runs never passes as zero tests.
fn scenario_runs(text: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut modules: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            modules.push((name, Vec::new()));
        } else if let Some(rest) = trimmed.strip_prefix("run ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let Some((_, runs)) = modules.last_mut() else {
                return Err(format!("run {name} outside a module"));
            };
            runs.push(name);
        }
    }
    if modules.is_empty() {
        return Err(format!("no module in formal/{SCENARIOS}"));
    }
    if let Some((module, _)) = modules.iter().find(|(_, runs)| runs.is_empty()) {
        return Err(format!(
            "module {module} in formal/{SCENARIOS} declares no run"
        ));
    }
    Ok(modules)
}

/// The number before " passing" in `quint test`'s report, if any.
fn passing(report: &str) -> Option<usize> {
    report.lines().find_map(|line| {
        let (count, rest) = line.trim().split_once(' ')?;
        rest.starts_with("passing").then(|| count.parse().ok())?
    })
}

fn scenarios() -> i32 {
    let formal = formal_dir();
    if let Err(err) = require_quint(&formal) {
        return fail(&err);
    }
    let text = match std::fs::read_to_string(formal.join(SCENARIOS)) {
        Ok(text) => text,
        Err(err) => return fail(&format!("cannot read formal/{SCENARIOS}: {err}")),
    };
    let modules = match scenario_runs(&text) {
        Ok(modules) => modules,
        Err(err) => return fail(&err),
    };
    let mut failed = Vec::new();
    for (module, runs) in &modules {
        let mut command = quint(&formal);
        command
            .arg("test")
            .args(BACKEND)
            .args(["--main", module, "--match"])
            .arg(format!("^({})$", runs.join("|")))
            .arg(SCENARIOS);
        let Some(output) = output_of(command, "quint test") else {
            failed.push(module.clone());
            continue;
        };
        let exited = echo(&output);
        let passed = passing(&String::from_utf8_lossy(&output.stdout));
        if !exited || passed != Some(runs.len()) {
            eprintln!(
                "xtask: {module}: {} of {} runs passed",
                passed.unwrap_or(0),
                runs.len()
            );
            failed.push(module.clone());
        }
    }
    if failed.is_empty() {
        0
    } else {
        fail(&format!("scenarios failed: {}", failed.join(", ")))
    }
}

/// The simulation seed: `QUINT_SEED` when set (to repeat a run), else a fresh one.
fn seed() -> String {
    std::env::var("QUINT_SEED")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            format!("{:#x}", nanos & u128::from(u64::MAX))
        })
}

/// The `quint run` arguments for `model` under `step`, checking `invariants`.
fn simulate_args(model: &Model, step: &str, invariants: &[String], seed: &str) -> Vec<String> {
    let mut args: Vec<String> = ["run", model.file, BACKEND[0], BACKEND[1], "--step", step]
        .map(str::to_owned)
        .to_vec();
    args.push("--invariants".to_owned());
    args.extend(invariants.iter().cloned());
    args.extend([
        "--max-samples".to_owned(),
        SAMPLES.to_string(),
        "--max-steps".to_owned(),
        MAX_STEPS.to_string(),
        "--seed".to_owned(),
        seed.to_owned(),
    ]);
    args
}

fn simulate() -> i32 {
    let formal = formal_dir();
    if let Err(err) = require_quint(&formal) {
        return fail(&err);
    }
    let seed = seed();
    println!("formal-simulate: seed {seed} (QUINT_SEED={seed} repeats it)");
    let mut failed = Vec::new();
    for model in MODELS {
        let invariants = match std::fs::read_to_string(formal.join(model.file)) {
            Ok(text) => all_invariants(&text),
            Err(err) => return fail(&format!("cannot read formal/{}: {err}", model.file)),
        };
        let Some(invariants) = invariants.filter(|i| !i.is_empty()) else {
            return fail(&format!(
                "formal/{} has no 'val allInvariants = and {{ ... }}' listing its invariants",
                model.file
            ));
        };
        for step in model.steps {
            let mut command = quint(&formal);
            command.args(simulate_args(model, step, &invariants, &seed));
            let Some(output) = output_of(command, "quint run") else {
                failed.push(format!("{} --step {step}", model.file));
                continue;
            };
            if output.status.success() {
                let report = String::from_utf8_lossy(&output.stdout);
                let verdict = report
                    .lines()
                    .find(|l| l.starts_with("[ok]"))
                    .unwrap_or("[ok]");
                println!("{} --step {step}: {verdict}", model.file);
            } else {
                echo(&output);
                failed.push(format!("{} --step {step}", model.file));
            }
        }
    }
    if failed.is_empty() {
        0
    } else {
        fail(&format!(
            "invariant violated or simulation failed (seed {seed}): {}",
            failed.join(", ")
        ))
    }
}

/// The names in a model's `val allInvariants = and { ... }` block.
fn all_invariants(text: &str) -> Option<Vec<String>> {
    let start = text.find("val allInvariants = and {")?;
    let body = &text[start..];
    let body = &body[body.find('{')? + 1..body.find('}')?];
    Some(
        body.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_run_only_the_typecheck_when_typecheck_flag_is_set() {
        // given
        let typecheck_only = true;

        // when
        let stages = stages(typecheck_only);

        // then
        let names: Vec<_> = stages.iter().map(|s| s.name).collect();
        assert_eq!(names, ["formal-typecheck"]);
    }

    #[test]
    fn should_need_node_for_every_stage_when_conformance_is_built() {
        // given
        let typecheck_only = false;

        // when
        let stages = stages(typecheck_only);

        // then
        let names: Vec<_> = stages.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            ["formal-typecheck", "formal-scenarios", "formal-simulate"]
        );
        assert!(stages.iter().all(|s| s.tool == Some("node")));
        for stage in &stages {
            let listed = crate::catalog::find(stage.name).expect("listed in the catalog");
            assert!(!listed.in_check, "{} joins check in FORMAL-09", stage.name);
            let crate::process::Action::InProcess { display, .. } = stage.action else {
                panic!("{} runs in process", stage.name);
            };
            assert_eq!(listed.command, display, "{}", stage.name);
        }
    }

    #[test]
    fn should_fail_with_npm_ci_hint_when_quint_is_missing() {
        // given
        let probe = Some((false, ""));

        // when
        let result = classify(probe, "0.33.0");

        // then
        let message = result.expect_err("missing quint");
        assert!(message.contains("missing quint 0.33.0"), "{message}");
        assert!(message.contains("npm ci --prefix formal"), "{message}");
    }

    #[test]
    fn should_fail_with_node_hint_when_npx_cannot_run() {
        // given
        let probe = None;

        // when
        let result = classify(probe, "0.33.0");

        // then
        let message = result.expect_err("no npx");
        assert!(message.contains("Node"), "{message}");
        assert!(message.contains("npm ci --prefix formal"), "{message}");
    }

    #[test]
    fn should_report_wrong_version_when_another_quint_is_installed() {
        // given
        let probe = Some((true, "0.32.0\n"));

        // when
        let result = classify(probe, "0.33.0");

        // then
        let message = result.expect_err("wrong version");
        assert!(message.contains("found '0.32.0', want 0.33.0"), "{message}");
    }

    #[test]
    fn should_accept_quint_when_the_pinned_version_is_installed() {
        // given
        let probe = Some((true, "0.33.0\n"));

        // when
        let result = classify(probe, "0.33.0");

        // then
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn should_lock_the_manifest_version_when_package_files_are_read() {
        // given
        let version = pinned_quint().expect("quint in versions.toml");
        let formal = formal_dir();

        // when
        let package = std::fs::read_to_string(formal.join("package.json")).expect("package.json");
        let lock = std::fs::read_to_string(formal.join("package-lock.json")).expect("lockfile");
        let npmrc = std::fs::read_to_string(formal.join(".npmrc")).expect(".npmrc");

        // then
        let pin = format!("\"@informalsystems/quint\": \"{version}\"");
        assert!(package.contains(&pin), "{pin} missing from package.json");
        let locked = format!(
            "\"node_modules/@informalsystems/quint\": {{\n      \"version\": \"{version}\""
        );
        assert!(lock.contains(&locked), "{locked} missing from the lockfile");
        assert!(npmrc.lines().any(|l| l.trim() == "ignore-scripts=true"));
    }

    #[test]
    fn should_collect_runs_per_module_when_scenarios_are_parsed() {
        // given
        let text = "// header\nmodule a {\n  import x.* from \"./x\"\n  /// doc\n  run one =\n    init\n  run two = init\n}\n\nmodule b_2 {\n  run three = init\n}\n";

        // when
        let modules = scenario_runs(text).expect("parsed");

        // then
        assert_eq!(
            modules,
            [
                ("a".to_owned(), vec!["one".to_owned(), "two".to_owned()]),
                ("b_2".to_owned(), vec!["three".to_owned()]),
            ]
        );
    }

    #[test]
    fn should_refuse_scenarios_when_a_module_declares_no_run() {
        // given
        let text = "module a {\n  run one = init\n}\nmodule empty {\n}\n";

        // when
        let result = scenario_runs(text);

        // then
        assert_eq!(
            result,
            Err("module empty in formal/scenarios.qnt declares no run".to_owned())
        );
    }

    #[test]
    fn should_list_the_committed_scenarios_when_the_file_is_parsed() {
        // given
        let text = std::fs::read_to_string(formal_dir().join(SCENARIOS)).expect("scenarios");

        // when
        let modules = scenario_runs(&text).expect("parsed");

        // then
        let lifecycle = &modules
            .iter()
            .find(|(m, _)| m == "lifecycleScenarios")
            .expect("lifecycleScenarios")
            .1;
        for name in [
            "startServeStop",
            "bindFailure",
            "watcherFailsDuringStart",
            "doubleStop",
        ] {
            assert!(lifecycle.iter().any(|r| r == name), "{name} missing");
        }
    }

    #[test]
    fn should_count_passing_runs_when_quint_test_report_is_read() {
        // given
        let report =
            "\n  lifecycleScenarios\n    ok a passed 1 test(s)\n\n  6 passing (62ms)\n  1 failed\n";

        // when
        let count = passing(report);

        // then
        assert_eq!(count, Some(6));
        assert_eq!(passing("  lifecycleScenarios\n"), None);
    }

    #[test]
    fn should_check_the_task_invariants_when_lifecycle_is_simulated() {
        // given
        let text = std::fs::read_to_string(formal_dir().join("lifecycle.qnt")).expect("model");

        // when
        let invariants = all_invariants(&text).expect("allInvariants block");

        // then
        for name in [
            "noResponsesWhenStopped",
            "singleListenerOwner",
            "tasksReleasedAfterStop",
            "stopIdempotent",
            "failedStartupHoldsNothing",
        ] {
            assert!(invariants.iter().any(|i| i == name), "{name} missing");
            assert!(text.contains(&format!("val {name} =")), "{name} undefined");
        }
    }

    #[test]
    fn should_read_each_conjunct_when_all_invariants_is_parsed() {
        // given
        let text = "val x = 1\n  val allInvariants = and {\n    a,\n    bB,\n  }\n  val y = 2\n";

        // when
        let invariants = all_invariants(text);

        // then
        assert_eq!(invariants, Some(vec!["a".to_owned(), "bB".to_owned()]));
        assert_eq!(all_invariants("val other = and { a }"), None);
    }

    #[test]
    fn should_pass_the_budget_and_seed_when_simulation_args_are_built() {
        // given
        let model = &MODELS[0];

        // when
        let args = simulate_args(model, "serverStep", &["inv".to_owned()], "0x2a");

        // then
        let line = args.join(" ");
        assert!(
            line.starts_with(
                "run lifecycle.qnt --backend typescript --step serverStep --invariants inv "
            ),
            "{line}"
        );
        assert!(
            line.ends_with("--max-samples 1000 --max-steps 25 --seed 0x2a"),
            "{line}"
        );
    }
}
