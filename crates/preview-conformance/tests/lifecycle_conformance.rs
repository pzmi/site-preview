//! Lifecycle conformance (FORMAL-05, foundation.md section 6.3): `formal/lifecycle.qnt`'s
//! traces replayed against the real composition `Server`, compared after every step.
//!
//! - The named scenarios, exported from `formal/lifecycle_traces.qnt` into
//!   `formal/traces/regression/lifecycle-*.itf.json` (the command is in that file's header).
//! - Fresh seeded traces of `serverStepMostlyBound`, generated into
//!   `formal/traces/generated/` with the locked Quint; the seed is printed, and
//!   `QUINT_SEED=<seed>` repeats the run.

use std::path::{Path, PathBuf};

use preview_conformance::drivers::lifecycle::{LifecycleDriver, project};
use preview_conformance::generate::{Family, formal_dir, seed};
use preview_conformance::{Cause, Divergence, DriverError, Trace, replay};
use tempfile::TempDir;

/// The named scenarios of `lifecycleScenarios` that run against the `Server`.
const NAMED: [&str; 6] = [
    "bindFailure",
    "doubleStop",
    "refusedBeforeStart",
    "restartAfterStop",
    "startServeStop",
    "watcherFailsDuringStart",
];

/// The regression traces' recorded seed (the scripts are deterministic).
const REGRESSION_SEED: &str = "0x5ce1";

/// Foundation.md section 6.4's initial budget: about 30 traces of 25 steps per model.
const GENERATED: Family<'static> = Family {
    file: "lifecycle.qnt",
    main: "lifecycle",
    step: "serverStepMostlyBound",
    invariant: "allInvariants",
    traces: 30,
    max_steps: 25,
    prefix: "lifecycle-",
};

fn site() -> TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><title>Conformance</title><h1>Hello</h1>",
    )
    .expect("an index page");
    dir
}

/// Every `lifecycle-*.itf.json` in `formal/traces/regression/`, sorted.
fn regression_traces() -> Vec<PathBuf> {
    let dir = formal_dir().join("traces").join("regression");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("formal/traces/regression")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("lifecycle-") && n.ends_with(".itf.json"))
        })
        .collect();
    files.sort();
    files
}

/// Replays the trace at `path` against a fresh driver over a fresh site.
fn replay_file(path: &Path, seed: &str) -> Result<(), Box<Divergence>> {
    let trace = Trace::load(path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .with_seed(seed);
    let root = site();
    let mut driver = LifecycleDriver::new(root.path());
    replay(&trace, &mut driver, project)
}

/// Panics with the full report of the first divergence, if any.
fn assert_no_divergence(outcomes: Vec<Result<(), Box<Divergence>>>) {
    if let Some(Err(divergence)) = outcomes.into_iter().find(Result::is_err) {
        panic!("{divergence}");
    }
}

#[test]
fn should_replay_with_zero_divergence_when_trace_is_a_named_scenario() {
    // given
    let traces = regression_traces();
    let names: Vec<String> = traces
        .iter()
        .filter_map(|p| p.file_name()?.to_str()?.strip_suffix(".itf.json"))
        .map(|n| n.trim_start_matches("lifecycle-").to_owned())
        .collect();
    assert_eq!(names, NAMED, "the committed named lifecycle traces");

    // when: each replays, every step compared
    let outcomes: Vec<_> = traces
        .iter()
        .map(|path| replay_file(path, REGRESSION_SEED))
        .collect();

    // then
    assert_no_divergence(outcomes);
}

#[test]
fn should_replay_with_zero_divergence_when_traces_are_generated_with_a_seed() {
    // given
    let seed = seed();
    println!("lifecycle generated traces: {}", GENERATED.describe(&seed));
    let traces = GENERATED
        .generate(&seed)
        .unwrap_or_else(|err| panic!("{err}"));

    // when
    let outcomes: Vec<_> = traces.iter().map(|path| replay_file(path, &seed)).collect();

    // then
    assert_no_divergence(outcomes);
    println!(
        "lifecycle generated traces: {} replayed, seed {seed}",
        traces.len()
    );
}

#[test]
fn should_diverge_at_the_step_when_driver_lacks_the_action() {
    // given: a named trace whose second step the driver is told is unknown
    let path = formal_dir().join("traces/regression/lifecycle-startServeStop.itf.json");
    let mut text = std::fs::read_to_string(&path).expect("the trace");
    let renamed = text.replacen(
        r#""mbt::actionTaken":"request""#,
        r#""mbt::actionTaken":"requestTwice""#,
        1,
    );
    assert_ne!(renamed, text, "the trace names a request step");
    text = renamed;
    let trace = Trace::from_json_str(&text, "renamed").expect("the trace decodes");
    let root = site();
    let mut driver = LifecycleDriver::new(root.path());

    // when
    let divergence = replay(&trace, &mut driver, project).expect_err("an unknown action");

    // then
    assert_eq!(
        (divergence.step, divergence.action.as_str()),
        (2, "requestTwice")
    );
    assert_eq!(
        divergence.cause,
        Cause::Driver(DriverError::UnknownAction("requestTwice".to_owned()))
    );
}
