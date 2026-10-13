//! The model a driver reads (ADR 0070): the reload driver declares `browsers.*.doc` and is
//! handed nothing else, so a model that differs everywhere else drives it the same way.

use preview_conformance::drivers::reload::ReloadDriver;
use preview_conformance::generate::regression;
use preview_conformance::{Driver, ModelView, Params, State, Value};

/// `value` with everything the patterns' remaining `steps` do not reach replaced by a value
/// of another shape: a pattern step `*` keeps every map key, a named step one field or key.
fn perturb(value: &Value, patterns: &[Vec<&str>]) -> Value {
    if patterns.iter().any(Vec::is_empty) {
        return value.clone();
    }
    let below = |key: &str| -> Vec<Vec<&str>> {
        patterns
            .iter()
            .filter(|p| p[0] == "*" || p[0] == key)
            .map(|p| p[1..].to_vec())
            .collect()
    };
    match value {
        Value::Record(fields) => Value::Record(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), perturb_or_replace(field, &below(name))))
                .collect(),
        ),
        Value::Map(entries) => Value::Map(
            entries
                .iter()
                .map(|(key, entry)| {
                    let reach = key.as_str().map(below).unwrap_or_default();
                    (key.clone(), perturb_or_replace(entry, &reach))
                })
                .collect(),
        ),
        _ => replaced(),
    }
}

fn perturb_or_replace(value: &Value, patterns: &[Vec<&str>]) -> Value {
    if patterns.is_empty() {
        replaced()
    } else {
        perturb(value, patterns)
    }
}

fn replaced() -> Value {
    Value::Str("perturbed".to_owned())
}

/// `state` with every model variable perturbed outside `reads`; the `mbt::` variables (the
/// step's action and picks) are kept.
fn perturbed(state: &State, reads: &[&str]) -> State {
    let patterns: Vec<Vec<&str>> = reads.iter().map(|r| r.split('.').collect()).collect();
    let vars = state
        .vars
        .iter()
        .map(|(name, value)| {
            if name.starts_with("mbt::") {
                return (name.clone(), value.clone());
            }
            let reach: Vec<Vec<&str>> = patterns
                .iter()
                .filter(|p| p[0] == name)
                .map(|p| p[1..].to_vec())
                .collect();
            (name.clone(), perturb_or_replace(value, &reach))
        })
        .collect();
    State {
        index: state.index,
        vars,
    }
}

/// Applies every step of `states` to a fresh driver through the view `replay` builds, and
/// returns each step's observation.
fn drive(states: &[State]) -> Vec<<ReloadDriver as Driver>::Obs> {
    let mut driver = ReloadDriver::new();
    states
        .iter()
        .map(|state| {
            let action = state.action_taken().expect("an action");
            let params = Params::new(state.nondet_picks().expect("picks"));
            let view = ModelView::new(state, ReloadDriver::MODEL_READS);
            driver
                .apply_with_model(action, &params, &view)
                .unwrap_or_else(|err| panic!("step {}: {err}", state.index));
            driver.observe()
        })
        .collect()
}

#[test]
fn should_drive_the_same_way_when_every_undeclared_model_path_is_perturbed() {
    for name in [
        "reload-editDuringFetch",
        "reload-restartSamePort",
        "reload-scriptNotAcceptedReloads",
    ] {
        // given: the trace, and a copy whose model differs everywhere but `browsers.*.doc`
        let trace = regression(name).unwrap_or_else(|err| panic!("{err}"));
        let edited: Vec<State> = trace
            .states
            .iter()
            .map(|s| perturbed(s, ReloadDriver::MODEL_READS))
            .collect();
        assert!(
            trace
                .states
                .iter()
                .zip(&edited)
                .all(|(a, b)| a.var("committed") != b.var("committed")
                    && a.var("browsers") != b.var("browsers")),
            "{name}: the copy differs"
        );

        // when
        let views_agree = trace.states.iter().zip(&edited).all(|(a, b)| {
            ModelView::new(a, ReloadDriver::MODEL_READS)
                == ModelView::new(b, ReloadDriver::MODEL_READS)
        });
        let original = drive(&trace.states);
        let perturbed = drive(&edited);

        // then: the driver saw the same model and the real code behaved the same
        assert!(views_agree, "{name}: the views differ");
        assert_eq!(original, perturbed, "{name}");
    }
}

#[test]
fn should_close_the_stream_when_the_declared_page_navigates() {
    // given: `reload-editDuringFetch`, whose b1 reloads at its first `receive`
    let trace = regression("reload-editDuringFetch").unwrap_or_else(|err| panic!("{err}"));
    let receive = trace
        .states
        .iter()
        .position(|s| s.action_taken() == Some("receive"))
        .expect("a receive");

    // when
    let observed = drive(&trace.states[..=receive]);

    // then: b1's stream was open before the receive and is closed after it
    let before = &observed[receive - 1];
    let after = &observed[receive];
    assert!(before.streams.contains_key("b1"));
    assert!(!after.streams.contains_key("b1"));
}
