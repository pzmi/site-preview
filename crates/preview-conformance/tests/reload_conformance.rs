//! Reload conformance (FORMAL-07, foundation.md section 6.3): `formal/reload.qnt`'s traces
//! replayed against the real `ReloadState` and SSE handler in process, compared after every
//! step (ADR 0017, ADR 0057).
//!
//! - The named scenarios, exported from `formal/reload_traces.qnt` into
//!   `formal/traces/regression/reload-*.itf.json` (the command is in that file's header).
//! - Seeded random traces of `step` in `formal/traces/generated/`: under `cargo xtask
//!   conformance`, the ones its `formal-generate` stage recorded in `manifest.json`; run
//!   directly, fresh ones from the locked Quint. The seed is printed, and `QUINT_SEED=<seed>`
//!   repeats the run.

use std::path::{Path, PathBuf};

use preview_conformance::drivers::reload::{ReloadDriver, project};
use preview_conformance::generate::{Family, formal_dir};
use preview_conformance::{Cause, Divergence, Trace, replay};

/// The named scenarios of `reloadScenarios`, each scripted in `reload_traces.qnt`.
const NAMED: [&str; 7] = [
    "coalescedCssThenJsFull",
    "cssOnlyHot",
    "disconnectEditReconnect",
    "editDuringFetch",
    "htmlFull",
    "restartSamePort",
    "scriptNotAcceptedReloads",
];

/// The regression traces' recorded seed (the scripts are deterministic).
const REGRESSION_SEED: &str = "0x5ce1";

/// Foundation.md section 6.4's initial budget: about 30 traces of 25 steps per model.
const GENERATED: Family<'static> = Family {
    file: "reload.qnt",
    main: "reload",
    step: "step",
    invariant: "allInvariants",
    traces: 30,
    max_steps: 25,
    prefix: "reload-",
};

/// Every `reload-*.itf.json` in `formal/traces/regression/`, sorted.
fn regression_traces() -> Vec<PathBuf> {
    let dir = formal_dir().join("traces").join("regression");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("formal/traces/regression")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("reload-") && n.ends_with(".itf.json"))
        })
        .collect();
    files.sort();
    files
}

fn load(path: &Path, seed: &str) -> Trace {
    Trace::load(path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .with_seed(seed)
}

/// Replays the trace at `path` against a fresh driver.
fn replay_file(path: &Path, seed: &str) -> Result<(), Box<Divergence>> {
    let trace = load(path, seed);
    let mut driver = ReloadDriver::new();
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
        .map(|n| n.trim_start_matches("reload-").to_owned())
        .collect();
    assert_eq!(names, NAMED, "the committed named reload traces");

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
    let (seed, traces) = GENERATED.traces().unwrap_or_else(|err| panic!("{err}"));
    println!("reload generated traces: {}", GENERATED.describe(&seed));

    // when
    let outcomes: Vec<_> = traces.iter().map(|path| replay_file(path, &seed)).collect();

    // then
    assert_no_divergence(outcomes);
    println!(
        "reload generated traces: {} replayed, seed {seed}",
        traces.len()
    );
}

#[test]
fn should_diverge_at_the_send_when_the_model_expects_a_different_event() {
    // given: the coalesced trace with the model's `full` event edited to a hot css one, as a
    // server that ignored the gap would send it (`noHotApplyAcrossGap`)
    let path = formal_dir().join("traces/regression/reload-coalescedCssThenJsFull.itf.json");
    let trace = load(&path, REGRESSION_SEED);
    let send = trace
        .states
        .iter()
        .position(|s| s.action_taken() == Some("send"))
        .expect("the trace sends");
    let text = std::fs::read_to_string(&path).expect("the trace");
    let gap_event = r##""full":true,"kinds":{"#set":[]},"name":{"tag":"ReloadEvent""##;
    let hot_event = r##""full":false,"kinds":{"#set":[{"tag":"Js","value":{"#tup":[]}}]},"name":{"tag":"ReloadEvent""##;
    assert!(
        text.contains(gap_event),
        "the trace has the coalesced event"
    );
    let edited = text.replace(gap_event, hot_event);
    let trace = Trace::from_json_str(&edited, "edited").expect("the trace decodes");
    let mut driver = ReloadDriver::new();

    // when
    let divergence = replay(&trace, &mut driver, project).expect_err("the real server differs");

    // then
    assert_eq!(
        (divergence.step, divergence.action.as_str()),
        (send, "send")
    );
    assert_eq!(divergence.cause, Cause::Mismatch);
}
