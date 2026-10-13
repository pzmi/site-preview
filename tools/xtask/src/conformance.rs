//! `cargo xtask conformance`: the Quint models in `formal/` and their replay against the real
//! code (foundation.md section 6, ADR 0052, ADR 0060). `check` runs every stage (ADR 0019):
//!
//! - `formal-typecheck` (`--typecheck` runs only this one): `quint typecheck` on every
//!   `formal/*.qnt`;
//! - `formal-scenarios`: `quint test` on each module of `formal/scenarios.qnt`, matching the
//!   `run` names that module declares, and counting them: `quint test` passes when nothing
//!   matches, so a run that silently stops being collected must fail here;
//! - `formal-simulate`: `quint run` of each model under each of its step relations, checking
//!   every conjunct of the model's `val allInvariants = and { ... }` (the one list of its
//!   invariants, so a new one cannot be left out), over [`SAMPLES`] traces of at most
//!   [`MAX_STEPS`] steps;
//! - `formal-freshness`: every committed trace in `formal/traces/regression/` is regenerated
//!   from `formal/traces/regression/manifest.toml` (its model, step, length and seed) into
//!   `formal/traces/generated/regression/` and compared byte for byte, Quint's creation date
//!   aside; a difference fails with `stale regression trace <file>`;
//! - `formal-generate`: [`GENERATED_TRACES`] fresh traces of [`MAX_STEPS`] steps per
//!   [`FAMILIES`] entry into `formal/traces/generated/`, with the seed, the Quint version and
//!   each family's count and length in `formal/traces/generated/manifest.json`;
//! - `conformance-replay`: the `preview-conformance` tests, replaying the committed traces and
//!   the generated ones the manifest lists against the real code (the `unit-tests` stage
//!   excludes that crate, so the replay runs once per gate);
//! - `conformance-controls`: its negative controls, faulty implementations each caught at an
//!   asserted step (ADR 0059).
//!
//! One seed serves the whole run and is printed; `QUINT_SEED` repeats it.
//!
//! Quint is the version `formal/package-lock.json` locks (`tools/versions.toml` notes it),
//! run with `npx --prefix formal --no`, which never fetches a package from the registry, and
//! always with the TypeScript backend: the default Rust backend downloads an evaluator binary
//! from GitHub at first use, which no lockfile pins (ADR 0052). Node is a development tool
//! only; the shipped binary never needs it. A missing Node or Quint fails with the command
//! that installs it (ADR 0010), never a skip.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::catalog::json_str;
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

/// The models `formal-simulate` runs.
pub const MODELS: &[Model] = &[
    Model {
        file: "lifecycle.qnt",
        steps: &["step", "serverStep", "serverStepMostlyBound"],
    },
    Model {
        file: "reload.qnt",
        steps: &["step"],
    },
    Model {
        file: "watcher.qnt",
        steps: &["step"],
    },
];

/// The file of named runs, one module per model.
const SCENARIOS: &str = "scenarios.qnt";

/// Traces per simulation (FORMAL-01's acceptance budget).
pub const SAMPLES: u32 = 1000;

/// Steps per trace (foundation.md section 6.4's initial budget).
pub const MAX_STEPS: u32 = 25;

/// The evaluator that comes from the lockfile (ADR 0052).
pub(crate) const BACKEND: [&str; 2] = ["--backend", "typescript"];

/// One family of generated traces: a model sampled under one step relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    /// The family name, also the file-name prefix in `formal/traces/generated/`.
    pub name: &'static str,
    /// The model file in `formal/`.
    pub file: &'static str,
    /// Its main module.
    pub main: &'static str,
    /// The step relation sampled (the one each driver replays).
    pub step: &'static str,
}

/// The families `formal-generate` writes, one per conformance driver. The lifecycle driver
/// replays the composition's `Server`, sampled so that a start fails its bind one time in
/// eight (ADR 0056).
pub const FAMILIES: &[Family] = &[
    Family {
        name: "lifecycle",
        file: "lifecycle.qnt",
        main: "lifecycle",
        step: "serverStepMostlyBound",
    },
    Family {
        name: "reload",
        file: "reload.qnt",
        main: "reload",
        step: "step",
    },
    Family {
        name: "watcher",
        file: "watcher.qnt",
        main: "watcher",
        step: "step",
    },
];

/// Generated traces per family (foundation.md section 6.4's initial budget).
pub const GENERATED_TRACES: u32 = 30;

/// The invariant every generated state satisfies.
const GENERATED_INVARIANT: &str = "allInvariants";

/// The committed regression traces and their manifest, from the repository root.
const REGRESSION_DIR: &str = "formal/traces/regression";

/// The manifest of the committed regression traces.
const REGRESSION_MANIFEST: &str = "manifest.toml";

/// Where `formal-generate` writes, from the repository root (ignored by Git).
const GENERATED_DIR: &str = "formal/traces/generated";

/// The generated traces' manifest, from the repository root.
pub const GENERATED_MANIFEST: &str = "formal/traces/generated/manifest.json";

/// Set for `conformance-replay` so the replay tests read [`GENERATED_MANIFEST`] instead of
/// generating their own traces (`preview_conformance::generate::MANIFEST_ENV`).
pub const GENERATED_ENV: &str = "CONFORMANCE_GENERATED_MANIFEST";

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
        stages.push(
            Stage::in_process(
                "formal-freshness",
                "npx --prefix formal --no -- quint run --backend typescript --mbt <each trace of formal/traces/regression/manifest.toml>, compared with the committed file",
                freshness,
            )
            .needs("node"),
        );
        stages.push(
            Stage::in_process(
                "formal-generate",
                "npx --prefix formal --no -- quint run --backend typescript --mbt --step <relation> --invariant allInvariants --n-traces 30 --max-steps 25 formal/<model>.qnt, recorded in formal/traces/generated/manifest.json",
                generate,
            )
            .needs("node"),
        );
        stages.push(
            Stage::new("conformance-replay", "cargo", &REPLAY_ARGS)
                .needs("cargo-nextest")
                .env(GENERATED_ENV, GENERATED_MANIFEST),
        );
        stages.push(
            Stage::new("conformance-controls", "cargo", &CONTROLS_ARGS).needs("cargo-nextest"),
        );
    }
    stages
}

/// The replay tests of `preview-conformance`: every test but the negative controls.
const REPLAY_ARGS: [&str; 9] = [
    "nextest",
    "run",
    "--locked",
    "--profile",
    "ci",
    "-p",
    "preview-conformance",
    "-E",
    "not binary(negative_controls)",
];

/// The negative controls of `preview-conformance` (ADR 0059).
const CONTROLS_ARGS: [&str; 9] = [
    "nextest",
    "run",
    "--locked",
    "--profile",
    "ci",
    "-p",
    "preview-conformance",
    "-E",
    "binary(negative_controls)",
];

/// Runs `conformance` (or only its typecheck) and returns the exit code.
pub fn run(typecheck_only: bool) -> i32 {
    finish(&run_report(&stages(typecheck_only), &tools::require))
}

/// `formal/` in the repository.
pub(crate) fn formal_dir() -> PathBuf {
    repo::root().join("formal")
}

/// The locally installed Quint, never one fetched from the registry: `--no` refuses to
/// install, and `--` keeps npx from taking Quint's own flags (`--version`) as its own.
pub(crate) fn quint(formal: &Path) -> Command {
    let mut command = Command::new("npx");
    command
        .arg("--prefix")
        .arg(formal)
        .args(["--no", "--", "quint"])
        .current_dir(formal);
    command
}

/// The pinned Quint version from `tools/versions.toml`.
pub(crate) fn pinned_quint() -> Result<String, String> {
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
pub(crate) fn require_quint(formal: &Path) -> Result<(), String> {
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
pub(crate) fn echo(output: &Output) -> bool {
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    output.status.success()
}

/// Runs `command`, reporting a failure to start it as a failed stage.
pub(crate) fn output_of(mut command: Command, label: &str) -> Option<Output> {
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
pub(crate) fn fail(err: &str) -> i32 {
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
    let outputs = in_parallel(&files, |file| {
        let mut command = quint(&formal);
        command.args(["typecheck", file]);
        output_of(command, "quint typecheck")
    });
    let mut failed = Vec::new();
    for (file, output) in files.iter().zip(outputs) {
        let ok = output.is_some_and(|o| echo(&o));
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
    let outputs = in_parallel(&modules, |(module, runs)| {
        let mut command = quint(&formal);
        command
            .arg("test")
            .args(BACKEND)
            .args(["--main", module, "--match"])
            .arg(format!("^({})$", runs.join("|")))
            .arg(SCENARIOS);
        output_of(command, "quint test")
    });
    let mut failed = Vec::new();
    for ((module, runs), output) in modules.iter().zip(outputs) {
        let Some(output) = output else {
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

/// The run's seed, for simulation and generation alike: `QUINT_SEED` when set (to repeat a
/// run), else a fresh one, chosen once per process.
pub(crate) fn seed() -> &'static str {
    static SEED: OnceLock<String> = OnceLock::new();
    SEED.get_or_init(|| {
        std::env::var("QUINT_SEED")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                format!("{:#x}", nanos & u128::from(u64::MAX))
            })
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
    let mut runs = Vec::new();
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
        runs.extend(
            model
                .steps
                .iter()
                .map(|step| (model, *step, invariants.clone())),
        );
    }
    let outputs = in_parallel(&runs, |(model, step, invariants)| {
        let mut command = quint(&formal);
        command.args(simulate_args(model, step, invariants, seed));
        output_of(command, "quint run")
    });
    let mut failed = Vec::new();
    for ((model, step, _), output) in runs.iter().zip(outputs) {
        let Some(output) = output else {
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
    if failed.is_empty() {
        0
    } else {
        fail(&format!(
            "invariant violated or simulation failed (seed {seed}): {}",
            failed.join(", ")
        ))
    }
}

/// Runs `work` over `items` on as many threads as the machine has cores (each Quint run is
/// one Node process, mostly single-threaded), each thread taking the next item when it is
/// free, and returns the results in item order.
pub(crate) fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    in_parallel_up_to(usize::MAX, items, work)
}

/// [`in_parallel`] on at most `limit` threads (a JVM run is memory-bound, not core-bound).
pub(crate) fn in_parallel_up_to<T: Sync, R: Send>(
    limit: usize,
    items: &[T],
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let width = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(limit.max(1))
        .min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..width {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = work(item);
                    done.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((i, result));
                }
            });
        }
    });
    let mut done = done
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// One committed regression trace and the `quint run` that regenerates it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionTrace {
    /// The file in `formal/traces/regression/`.
    pub file: String,
    /// The model file in `formal/` it is exported from.
    pub model: String,
    /// `--main`, when not the file's own module.
    pub main: Option<String>,
    /// `--init`, when not `init`.
    pub init: Option<String>,
    /// `--step`: a script of a `*_traces.qnt` module, or a model's step relation.
    pub step: String,
    /// `--max-steps`: a script's length.
    pub max_steps: u32,
    /// `--seed`.
    pub seed: String,
    /// `--invariant`, checked on every exported state.
    pub invariant: Option<String>,
}

/// `formal/traces/regression/manifest.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegressionManifest {
    trace: Vec<RegressionTrace>,
}

/// Parses the regression manifest; a file listed twice is an error.
fn regression_manifest(text: &str) -> Result<Vec<RegressionTrace>, String> {
    let manifest: RegressionManifest =
        toml::from_str(text).map_err(|e| format!("{REGRESSION_DIR}/{REGRESSION_MANIFEST}: {e}"))?;
    let mut seen = std::collections::BTreeSet::new();
    if let Some(dup) = manifest
        .trace
        .iter()
        .find(|t| !seen.insert(t.file.as_str()))
    {
        return Err(format!(
            "{REGRESSION_DIR}/{REGRESSION_MANIFEST} lists {} twice",
            dup.file
        ));
    }
    Ok(manifest.trace)
}

/// The `*.itf.json` files in `dir`, sorted.
fn itf_files(dir: &Path) -> Result<Vec<String>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.ends_with(".itf.json"))
        .collect();
    files.sort();
    Ok(files)
}

/// The mismatches between the manifest and the committed files: a trace without an entry
/// (nothing would regenerate it) and an entry without a trace.
fn unlisted(entries: &[RegressionTrace], files: &[String]) -> Vec<String> {
    let listed: std::collections::BTreeSet<&str> =
        entries.iter().map(|t| t.file.as_str()).collect();
    let present: std::collections::BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let mut problems: Vec<String> = present
        .difference(&listed)
        .map(|f| format!("{REGRESSION_DIR}/{f} has no entry in {REGRESSION_MANIFEST}"))
        .collect();
    problems.extend(
        listed
            .difference(&present)
            .map(|f| format!("{REGRESSION_MANIFEST} lists {f}, which is not in {REGRESSION_DIR}")),
    );
    problems
}

/// The `quint run` arguments that regenerate `trace` into `out`, run from the repository
/// root (the ITF records the model's path as given, `formal/<model>`).
fn regenerate_args(trace: &RegressionTrace, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "run".to_owned(),
        format!("formal/{}", trace.model),
        BACKEND[0].to_owned(),
        BACKEND[1].to_owned(),
    ];
    if let Some(main) = &trace.main {
        args.extend(["--main".to_owned(), main.clone()]);
    }
    if let Some(init) = &trace.init {
        args.extend(["--init".to_owned(), init.clone()]);
    }
    args.extend([
        "--step".to_owned(),
        trace.step.clone(),
        "--max-steps".to_owned(),
        trace.max_steps.to_string(),
        "--mbt".to_owned(),
        "--seed".to_owned(),
        trace.seed.clone(),
    ]);
    if let Some(invariant) = &trace.invariant {
        args.extend(["--invariant".to_owned(), invariant.clone()]);
    }
    args.push("--out-itf".to_owned());
    args.push(out.display().to_string());
    args
}

/// Blanks the value of `"key":` in `header`: a string up to its closing quote, or a number.
fn blank(header: &str, key: &str, empty: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = header.find(&needle)? + needle.len();
    let rest = &header[start..];
    let len = if let Some(string) = rest.strip_prefix('"') {
        string.find('"')? + 2
    } else {
        rest.find(|c: char| !c.is_ascii_digit())?
    };
    Some(format!("{}{empty}{}", &header[..start], &rest[len..]))
}

/// The ITF text with the top-level `#meta.description` and `#meta.timestamp` blanked: the
/// creation date Quint writes, the only part that differs between two runs with one seed
/// (ADR 0053). Everything else, the states and the rest of `#meta`, is kept as it is, so the
/// comparison stays byte for byte. `None` when the file does not start with that header.
fn normalize(text: &str) -> Option<String> {
    let header_end = text.find("\"vars\":")?;
    let (header, body) = text.split_at(header_end);
    if !header.starts_with("{\"#meta\":{") {
        return None;
    }
    let header = blank(header, "description", "\"\"")?;
    let header = blank(&header, "timestamp", "0")?;
    Some(header + body)
}

/// How a regenerated regression trace compared with the committed one.
#[derive(Debug, PartialEq, Eq)]
enum Freshness {
    Fresh,
    Stale,
}

/// Compares a committed trace with its regeneration.
fn compare(committed: &str, fresh: &str) -> Result<Freshness, String> {
    let committed = normalize(committed).ok_or("the committed trace has no ITF #meta header")?;
    let fresh = normalize(fresh).ok_or("Quint wrote no ITF #meta header")?;
    Ok(if committed == fresh {
        Freshness::Fresh
    } else {
        Freshness::Stale
    })
}

/// Regenerates one committed trace into `out_dir` and compares it.
fn check_fresh(root: &Path, trace: &RegressionTrace, out_dir: &Path) -> Result<Freshness, String> {
    let out = out_dir.join(&trace.file);
    let mut command = quint(&root.join("formal"));
    command.current_dir(root).args(regenerate_args(trace, &out));
    let output = command
        .output()
        .map_err(|e| format!("cannot run quint: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "quint run failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let committed = std::fs::read_to_string(root.join(REGRESSION_DIR).join(&trace.file))
        .map_err(|e| format!("cannot read the committed trace: {e}"))?;
    let fresh =
        std::fs::read_to_string(&out).map_err(|e| format!("cannot read {}: {e}", out.display()))?;
    compare(&committed, &fresh)
}

/// Removes every `*.itf.json` from `dir` (a previous run's output).
fn clear_traces(dir: &Path, prefix: &str) -> Result<(), String> {
    for file in itf_files(dir)? {
        if file.starts_with(prefix) {
            let path = dir.join(&file);
            std::fs::remove_file(&path)
                .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn freshness() -> i32 {
    let root = repo::root();
    if let Err(err) = require_quint(&root.join("formal")) {
        return fail(&err);
    }
    let dir = root.join(REGRESSION_DIR);
    let entries = match std::fs::read_to_string(dir.join(REGRESSION_MANIFEST))
        .map_err(|e| format!("cannot read {REGRESSION_DIR}/{REGRESSION_MANIFEST}: {e}"))
        .and_then(|text| regression_manifest(&text))
    {
        Ok(entries) => entries,
        Err(err) => return fail(&err),
    };
    let files = match itf_files(&dir) {
        Ok(files) => files,
        Err(err) => return fail(&err),
    };
    let problems = unlisted(&entries, &files);
    if !problems.is_empty() {
        return fail(&problems.join("\n"));
    }
    let out_dir = root.join(GENERATED_DIR).join("regression");
    if let Err(err) = std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))
        .and_then(|()| clear_traces(&out_dir, ""))
    {
        return fail(&err);
    }
    let results = in_parallel(&entries, |trace| check_fresh(&root, trace, &out_dir));
    let mut failed = 0;
    for (trace, result) in entries.iter().zip(results) {
        match result {
            Ok(Freshness::Fresh) => println!("fresh: {REGRESSION_DIR}/{}", trace.file),
            Ok(Freshness::Stale) => {
                failed += 1;
                eprintln!(
                    "xtask: stale regression trace {REGRESSION_DIR}/{file}: regenerating it with \
                     seed {seed} gives {GENERATED_DIR}/regression/{file}; if the model or script \
                     change is intended, review that file and commit it in place",
                    file = trace.file,
                    seed = trace.seed
                );
            }
            Err(err) => {
                failed += 1;
                eprintln!(
                    "xtask: cannot regenerate {REGRESSION_DIR}/{}: {err}",
                    trace.file
                );
            }
        }
    }
    if failed == 0 {
        println!(
            "formal-freshness: {} regression traces fresh",
            entries.len()
        );
        0
    } else {
        fail(&format!(
            "{failed} of {} regression traces not fresh",
            entries.len()
        ))
    }
}

/// The `quint run` arguments that write `family`'s traces to `pattern` (run from `formal/`).
fn generate_args(family: &Family, seed: &str, pattern: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "run",
        family.file,
        BACKEND[0],
        BACKEND[1],
        "--mbt",
        "--main",
        family.main,
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend([
        "--step".to_owned(),
        family.step.to_owned(),
        "--invariant".to_owned(),
        GENERATED_INVARIANT.to_owned(),
        "--max-samples".to_owned(),
        GENERATED_TRACES.to_string(),
        "--n-traces".to_owned(),
        GENERATED_TRACES.to_string(),
        "--max-steps".to_owned(),
        MAX_STEPS.to_string(),
        "--seed".to_owned(),
        seed.to_owned(),
        "--out-itf".to_owned(),
        pattern.to_owned(),
    ]);
    args
}

/// What one family's generation produced: its trace files (relative to the generated
/// directory), or the failure.
type Generated = Result<Vec<String>, String>;

/// `formal/traces/generated/manifest.json`: the run's seed and Quint version, and per
/// family its model, step relation, invariant, trace count, maximum steps, status and files.
fn generated_manifest(seed: &str, quint: &str, families: &[(Family, Generated)]) -> String {
    let rows: Vec<String> = families
        .iter()
        .map(|(family, generated)| {
            let (status, files) = match generated {
                Ok(files) => ("ok", files.as_slice()),
                Err(_) => ("failed", &[][..]),
            };
            let files: Vec<String> = files.iter().map(|f| json_str(f)).collect();
            format!(
                "    {{\"family\": {}, \"model\": {}, \"main\": {}, \"step\": {}, \
                 \"invariant\": {}, \"traces\": {GENERATED_TRACES}, \"max_steps\": {MAX_STEPS}, \
                 \"status\": {}, \"files\": [{}]}}",
                json_str(family.name),
                json_str(family.file),
                json_str(family.main),
                json_str(family.step),
                json_str(GENERATED_INVARIANT),
                json_str(status),
                files.join(", ")
            )
        })
        .collect();
    format!(
        "{{\n  \"seed\": {},\n  \"quint\": {},\n  \"backend\": \"typescript\",\n  \"families\": [\n{}\n  ]\n}}\n",
        json_str(seed),
        json_str(quint),
        rows.join(",\n")
    )
}

/// Generates one family's traces into `out` with `seed`.
fn generate_family(formal: &Path, out: &Path, family: &Family, seed: &str) -> Generated {
    let prefix = format!("{}-", family.name);
    clear_traces(out, &prefix)?;
    let pattern = format!("traces/generated/{prefix}{{seq}}.itf.json");
    let mut command = quint(formal);
    command.args(generate_args(family, seed, &pattern));
    let output = command
        .output()
        .map_err(|e| format!("cannot run quint: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "quint run failed (an invariant violation is in the traces written):\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let files: Vec<String> = itf_files(out)?
        .into_iter()
        .filter(|f| f.starts_with(&prefix))
        .collect();
    if files.len() == GENERATED_TRACES as usize {
        Ok(files)
    } else {
        Err(format!(
            "quint wrote {} traces, not {GENERATED_TRACES}",
            files.len()
        ))
    }
}

fn generate() -> i32 {
    let root = repo::root();
    let formal = root.join("formal");
    if let Err(err) = require_quint(&formal) {
        return fail(&err);
    }
    let quint_version = match pinned_quint() {
        Ok(version) => version,
        Err(err) => return fail(&err),
    };
    let out = root.join(GENERATED_DIR);
    let manifest = root.join(GENERATED_MANIFEST);
    if let Err(err) =
        std::fs::create_dir_all(&out).and_then(|()| match std::fs::remove_file(&manifest) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        })
    {
        return fail(&format!("cannot prepare {GENERATED_DIR}: {err}"));
    }
    let seed = seed();
    println!("formal-generate: seed {seed} (QUINT_SEED={seed} repeats it)");
    let results = in_parallel(FAMILIES, |family| {
        generate_family(&formal, &out, family, seed)
    });
    let families: Vec<(Family, Generated)> = FAMILIES.iter().copied().zip(results).collect();
    if let Err(err) = std::fs::write(
        &manifest,
        generated_manifest(seed, &quint_version, &families),
    ) {
        return fail(&format!("cannot write {GENERATED_MANIFEST}: {err}"));
    }
    let mut failed = Vec::new();
    for (family, generated) in &families {
        match generated {
            Ok(files) => println!(
                "{} --step {}: {} traces of at most {MAX_STEPS} steps",
                family.file,
                family.step,
                files.len()
            ),
            Err(err) => {
                eprintln!("xtask: {} --step {}: {err}", family.file, family.step);
                failed.push(family.name);
            }
        }
    }
    if failed.is_empty() {
        println!("formal-generate: recorded in {GENERATED_MANIFEST}");
        0
    } else {
        fail(&format!(
            "generation failed (seed {seed}): {}; traces kept in {GENERATED_DIR}",
            failed.join(", ")
        ))
    }
}

/// The names in a model's `val allInvariants = and { ... }` block.
pub(crate) fn all_invariants(text: &str) -> Option<Vec<String>> {
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
    fn should_run_every_stage_in_check_when_conformance_is_built() {
        // given
        let typecheck_only = false;

        // when
        let stages = stages(typecheck_only);

        // then
        let names: Vec<_> = stages.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "formal-typecheck",
                "formal-scenarios",
                "formal-simulate",
                "formal-freshness",
                "formal-generate",
                "conformance-replay",
                "conformance-controls",
            ]
        );
        for stage in &stages {
            let listed = crate::catalog::find(stage.name).expect("listed in the catalog");
            assert!(listed.in_check, "{} is in check (FORMAL-09)", stage.name);
            let want = if stage.name.starts_with("formal-") {
                "node"
            } else {
                "cargo-nextest"
            };
            assert_eq!(stage.tool, Some(want), "{}", stage.name);
        }
    }

    #[test]
    fn should_replay_from_the_manifest_and_split_the_controls_when_replay_stages_are_built() {
        // given
        let stages = stages(false);

        // when
        let replay = format!("{:?}", stages[5].action);
        let controls = format!("{:?}", stages[6].action);

        // then
        assert!(
            replay.contains(
                "(\"CONFORMANCE_GENERATED_MANIFEST\", \"formal/traces/generated/manifest.json\")"
            ),
            "{replay}"
        );
        assert!(
            replay.contains("\"not binary(negative_controls)\""),
            "{replay}"
        );
        assert!(
            controls.contains("\"binary(negative_controls)\""),
            "{controls}"
        );
        assert!(!controls.contains("not binary"), "{controls}");
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
    fn should_check_the_task_invariants_when_reload_is_simulated() {
        // given
        let text = std::fs::read_to_string(formal_dir().join("reload.qnt")).expect("model");

        // when
        let invariants = all_invariants(&text).expect("allInvariants block");

        // then
        for name in [
            "monotonicRevision",
            "noUncommittedClaim",
            "atMostOneReloadPerChange",
            "noHotApplyAcrossGap",
            "htmlChangeIsFull",
            "hotApplyAdvancesBaseline",
        ] {
            assert!(invariants.iter().any(|i| i == name), "{name} missing");
            assert!(text.contains(&format!("val {name} =")), "{name} undefined");
        }
        assert!(text.contains("temporal reconnectObservesNewer ="));
        assert!(MODELS.iter().any(|m| m.file == "reload.qnt"));
    }

    #[test]
    fn should_check_the_task_invariants_when_watcher_is_simulated() {
        // given
        let text = std::fs::read_to_string(formal_dir().join("watcher.qnt")).expect("model");

        // when
        let invariants = all_invariants(&text).expect("allInvariants block");

        // then
        assert!(MODELS.iter().any(|m| m.file == "watcher.qnt"));
        for name in [
            "onlyEligibleTriggerReload",
            "noLostEligibleChange",
            "boundedReloadsPerBurst",
            "failureIsExplicit",
        ] {
            assert!(invariants.iter().any(|i| i == name), "{name} missing");
            assert!(text.contains(&format!("val {name} =")), "{name} undefined");
        }
    }

    #[test]
    fn should_list_the_watcher_scenarios_when_the_file_is_parsed() {
        // given
        let text = std::fs::read_to_string(formal_dir().join(SCENARIOS)).expect("scenarios");

        // when
        let modules = scenario_runs(&text).expect("parsed");

        // then
        let watcher = &modules
            .iter()
            .find(|(m, _)| m == "watcherScenarios")
            .expect("watcherScenarios")
            .1;
        for name in [
            "burstOneReload",
            "gitOnlyNoReload",
            "atomicSave",
            "overflowRescan",
            "dirRecreate",
        ] {
            assert!(watcher.iter().any(|r| r == name), "{name} missing");
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

    /// The committed regression manifest.
    fn committed_manifest() -> Vec<RegressionTrace> {
        let text =
            std::fs::read_to_string(repo::root().join(REGRESSION_DIR).join(REGRESSION_MANIFEST))
                .expect("the regression manifest");
        regression_manifest(&text).expect("the manifest parses")
    }

    #[test]
    fn should_list_each_committed_trace_once_when_the_regression_manifest_is_read() {
        // given
        let entries = committed_manifest();

        // when
        let files = itf_files(&repo::root().join(REGRESSION_DIR)).expect("the traces");

        // then
        assert_eq!(unlisted(&entries, &files), Vec::<String>::new());
        assert!(files.len() >= 25, "{files:?}");
    }

    #[test]
    fn should_report_both_directions_when_manifest_and_files_differ() {
        // given
        let entries = regression_manifest(
            "[[trace]]\nfile = \"a.itf.json\"\nmodel = \"m.qnt\"\nstep = \"s\"\nmax_steps = 1\nseed = \"1\"\n",
        )
        .expect("parses");
        let files = vec!["b.itf.json".to_owned()];

        // when
        let problems = unlisted(&entries, &files);

        // then
        assert_eq!(
            problems,
            [
                "formal/traces/regression/b.itf.json has no entry in manifest.toml",
                "manifest.toml lists a.itf.json, which is not in formal/traces/regression",
            ]
        );
    }

    #[test]
    fn should_refuse_the_manifest_when_a_file_is_listed_twice_or_a_key_is_unknown() {
        // given
        let entry = "[[trace]]\nfile = \"a.itf.json\"\nmodel = \"m.qnt\"\nstep = \"s\"\nmax_steps = 1\nseed = \"1\"\n";
        let twice = format!("{entry}{entry}");
        let unknown = format!("{entry}steps = 2\n");

        // when
        let duplicate = regression_manifest(&twice);
        let typo = regression_manifest(&unknown);

        // then
        assert_eq!(
            duplicate,
            Err("formal/traces/regression/manifest.toml lists a.itf.json twice".to_owned())
        );
        assert!(typo.expect_err("unknown key").contains("steps"));
    }

    #[test]
    fn should_match_each_script_and_its_comment_when_traces_modules_are_read() {
        // given
        let entries = committed_manifest();
        let formal = formal_dir();

        // when: every script of a `*_traces.qnt` module, with the length and initial action
        // its doc comment states
        let mut scripts = Vec::new();
        for file in models_in(&formal).expect("models") {
            if !file.ends_with("_traces.qnt") {
                continue;
            }
            let text = std::fs::read_to_string(formal.join(&file)).expect("the script file");
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let Some(name) = line
                    .trim()
                    .strip_prefix("action ")
                    .and_then(|r| r.strip_suffix(" = any {"))
                else {
                    continue;
                };
                let comment = lines[i - 1];
                let steps: u32 = comment
                    .rsplit(' ')
                    .nth(1)
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| panic!("{file}: no length above {name}: {comment}"));
                let init = comment
                    .contains("`initUnwatchable`")
                    .then(|| "initUnwatchable".to_owned())
                    .or_else(|| comment.contains("`init`").then(|| "init".to_owned()));
                scripts.push((file.clone(), name.to_owned(), steps, init));
            }
        }

        // then: each has exactly one entry with those parameters
        assert!(scripts.len() >= 24, "{scripts:?}");
        for (file, name, steps, init) in &scripts {
            let matching: Vec<_> = entries
                .iter()
                .filter(|t| t.model == *file && t.step == *name)
                .collect();
            assert_eq!(matching.len(), 1, "{file} {name}");
            let entry = matching[0];
            assert_eq!(entry.max_steps, *steps, "{file} {name}");
            assert_eq!(entry.init, *init, "{file} {name}");
            assert_eq!(entry.seed, "0x5ce1", "{file} {name}");
            assert_eq!(
                entry.invariant.as_deref(),
                Some("allInvariants"),
                "{file} {name}"
            );
            let module = entry.main.as_deref().expect("a --main");
            assert!(module.ends_with("Traces"), "{module}");
        }
    }

    #[test]
    fn should_build_the_documented_command_when_a_trace_is_regenerated() {
        // given
        let trace = RegressionTrace {
            file: "watcher-x.itf.json".to_owned(),
            model: "watcher_traces.qnt".to_owned(),
            main: Some("watcherTraces".to_owned()),
            init: Some("initUnwatchable".to_owned()),
            step: "x".to_owned(),
            max_steps: 5,
            seed: "0x5ce1".to_owned(),
            invariant: Some("allInvariants".to_owned()),
        };

        // when
        let args = regenerate_args(&trace, Path::new("out/w.itf.json"));

        // then
        assert_eq!(
            args.join(" "),
            "run formal/watcher_traces.qnt --backend typescript --main watcherTraces \
             --init initUnwatchable --step x --max-steps 5 --mbt --seed 0x5ce1 \
             --invariant allInvariants --out-itf out/w.itf.json"
        );
    }

    const HEADER: &str = "{\"#meta\":{\"format\":\"ITF\",\"source\":\"formal/a.qnt\",\"status\":\"ok\",\"description\":\"Created by Quint on Mon Oct 12 2026\",\"timestamp\":1791817960688},\"vars\":[\"x\"],\"states\":[{\"x\":1}]}";

    #[test]
    fn should_be_fresh_when_only_the_creation_date_differs() {
        // given
        let other = HEADER
            .replace("Mon Oct 12 2026", "Tue Oct 13 2026")
            .replace("1791817960688", "1791904360000");

        // when
        let result = compare(HEADER, &other);

        // then
        assert_eq!(result, Ok(Freshness::Fresh));
    }

    #[test]
    fn should_be_stale_when_any_other_byte_differs() {
        // given
        let state = HEADER.replace("{\"x\":1}", "{\"x\":2}");
        let source = HEADER.replace("formal/a.qnt", "a.qnt");
        let spacing = HEADER.replace("\"states\":[", "\"states\": [");

        // when
        let results = [&state, &source, &spacing].map(|t| compare(HEADER, t));

        // then
        assert_eq!(
            results,
            [
                Ok(Freshness::Stale),
                Ok(Freshness::Stale),
                Ok(Freshness::Stale)
            ]
        );
    }

    #[test]
    fn should_refuse_to_compare_when_the_meta_header_is_missing() {
        // given
        let bare = "{\"vars\":[],\"states\":[]}";

        // when
        let result = compare(bare, HEADER);

        // then
        assert_eq!(
            result,
            Err("the committed trace has no ITF #meta header".to_owned())
        );
    }

    #[test]
    fn should_record_seed_quint_and_family_budget_when_the_generated_manifest_is_written() {
        // given
        let families = vec![
            (FAMILIES[0], Ok(vec!["lifecycle-0.itf.json".to_owned()])),
            (FAMILIES[1], Err("violated".to_owned())),
        ];

        // when
        let json = generated_manifest("0x2a", "0.33.0", &families);

        // then
        assert!(json.contains("\"seed\": \"0x2a\""), "{json}");
        assert!(json.contains("\"quint\": \"0.33.0\""), "{json}");
        assert!(
            json.contains(
                "{\"family\": \"lifecycle\", \"model\": \"lifecycle.qnt\", \"main\": \"lifecycle\", \
                 \"step\": \"serverStepMostlyBound\", \"invariant\": \"allInvariants\", \
                 \"traces\": 30, \"max_steps\": 25, \"status\": \"ok\", \
                 \"files\": [\"lifecycle-0.itf.json\"]}"
            ),
            "{json}"
        );
        assert!(
            json.contains("\"status\": \"failed\", \"files\": []"),
            "{json}"
        );
    }

    #[test]
    fn should_sample_each_driver_step_relation_when_traces_are_generated() {
        // given
        let family = FAMILIES[0];

        // when
        let line =
            generate_args(&family, "0x2a", "traces/generated/lifecycle-{seq}.itf.json").join(" ");

        // then
        assert_eq!(
            line,
            "run lifecycle.qnt --backend typescript --mbt --main lifecycle --step \
             serverStepMostlyBound --invariant allInvariants --max-samples 30 --n-traces 30 \
             --max-steps 25 --seed 0x2a --out-itf traces/generated/lifecycle-{seq}.itf.json"
        );
        let names: Vec<_> = FAMILIES.iter().map(|f| f.name).collect();
        assert_eq!(names, ["lifecycle", "reload", "watcher"]);
        assert!(FAMILIES.iter().all(|f| {
            MODELS
                .iter()
                .any(|m| m.file == f.file && m.steps.contains(&f.step))
        }));
    }

    #[test]
    fn should_keep_item_order_when_work_runs_in_parallel() {
        // given
        let items: Vec<u32> = (0..20).collect();

        // when
        let results = in_parallel(&items, |n| n * 2);

        // then
        assert_eq!(results, (0..20).map(|n| n * 2).collect::<Vec<_>>());
    }
}
