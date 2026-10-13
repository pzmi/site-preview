//! Watcher conformance (FORMAL-06, foundation.md section 6.3): `formal/watcher.qnt`'s traces
//! replayed against the real `ChangeCoordinator`, debouncer and `ReloadState` on a fake
//! clock, compared after every step.
//!
//! - The named scenarios, exported from `formal/watcher_traces.qnt` into
//!   `formal/traces/regression/watcher-*.itf.json` (the command is in that file's header).
//! - Seeded random traces of `step` in `formal/traces/generated/`: under `cargo xtask
//!   conformance`, the ones its `formal-generate` stage recorded in `manifest.json`; run
//!   directly, fresh ones from the locked Quint. The seed is printed, and `QUINT_SEED=<seed>`
//!   repeats the run.

use std::path::PathBuf;

use preview_conformance::drivers::watcher::{WatcherDriver, project};
use preview_conformance::generate::{Family, formal_dir, regression};
use preview_conformance::{Cause, Divergence, DriverError, Trace, replay};

/// The named scenarios of `watcherScenarios`, each scripted in `watcher_traces.qnt`.
const NAMED: [&str; 15] = [
    "atomicSave",
    "burstOneReload",
    "dirRecreate",
    "gitOnlyNoReload",
    "overflowRescan",
    "recoveryAfterWatchLimit",
    "recoveryAloneReloads",
    "rootDeleteRecreate",
    "rootRemovalEscalates",
    "rootRemovedAfterEmptyReconcile",
    "runtimeFailureDegrades",
    "shutdownDropsPending",
    "startupCapDefersReadiness",
    "startupFailureBeforeOverflow",
    "startupRegistrationFailure",
];

/// Foundation.md section 6.4's initial budget: about 30 traces of 25 steps per model.
const GENERATED: Family<'static> = Family {
    file: "watcher.qnt",
    main: "watcher",
    step: "step",
    invariant: "allInvariants",
    traces: 30,
    max_steps: 25,
    prefix: "watcher-",
};

/// Every `watcher-*.itf.json` in `formal/traces/regression/`, sorted.
fn regression_traces() -> Vec<PathBuf> {
    let dir = formal_dir().join("traces").join("regression");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("formal/traces/regression")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("watcher-") && n.ends_with(".itf.json"))
        })
        .collect();
    files.sort();
    files
}

/// Replays `trace` against a fresh driver over the real coordinator.
fn replay_trace(trace: &Trace) -> Result<(), Box<Divergence>> {
    let mut driver =
        WatcherDriver::new(trace).unwrap_or_else(|err| panic!("{}: environment: {err}", trace.id));
    replay(trace, &mut driver, project)
}

/// Replays the committed trace `watcher-<name>`, with its manifest's seed and length.
fn replay_named(name: &str) -> Result<(), Box<Divergence>> {
    let trace = regression(&format!("watcher-{name}")).unwrap_or_else(|err| panic!("{err}"));
    replay_trace(&trace)
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
        .map(|n| n.trim_start_matches("watcher-").to_owned())
        .collect();
    assert_eq!(names, NAMED, "the committed named watcher traces");

    // when: each replays, every step compared
    let outcomes: Vec<_> = names.iter().map(|name| replay_named(name)).collect();

    // then
    assert_no_divergence(outcomes);
}

#[test]
fn should_replay_with_zero_divergence_when_traces_are_generated_with_a_seed() {
    // given
    let (seed, traces) = GENERATED.load().unwrap_or_else(|err| panic!("{err}"));
    println!("watcher generated traces: {}", GENERATED.describe(&seed));

    // when
    let outcomes: Vec<_> = traces.iter().map(replay_trace).collect();

    // then
    assert_no_divergence(outcomes);
    println!(
        "watcher generated traces: {} replayed, seed {seed}",
        traces.len()
    );
}

#[test]
fn should_diverge_at_the_step_when_driver_lacks_the_action() {
    // given: a named trace whose first settle the driver is told is unknown
    let path = formal_dir().join("traces/regression/watcher-burstOneReload.itf.json");
    let text = std::fs::read_to_string(&path).expect("the trace");
    let renamed = text.replacen(
        r#""mbt::actionTaken":"settle""#,
        r#""mbt::actionTaken":"settleTwice""#,
        1,
    );
    assert_ne!(renamed, text, "the trace names a settle step");
    let trace = Trace::from_json_str(&renamed, "renamed").expect("the trace decodes");

    // when
    let divergence = replay_trace(&trace).expect_err("an unknown action");

    // then
    assert_eq!(
        (divergence.step, divergence.action.as_str()),
        (10, "settleTwice")
    );
    assert_eq!(
        divergence.cause,
        Cause::Driver(DriverError::UnknownAction("settleTwice".to_owned()))
    );
}
