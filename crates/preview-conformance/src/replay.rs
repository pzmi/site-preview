//! The explicit replay path (foundation.md section 6.3): feed each step of an ITF trace to a
//! [`Driver`] that applies it to the real code, project the model's state and the driver's
//! observation into the same type, and compare them after **every** step, the initial state
//! included. The first difference, refusal or undecodable step stops the replay with a
//! [`Divergence`].
//!
//! This module knows nothing about a particular model or harness library: a driver names the
//! actions it understands and decodes its own parameters from [`Params`], and the caller
//! supplies the projection of the model's state (ADR 0053). A driver that needs a choice the
//! model's environment made declares the model paths it reads ([`Driver::MODEL_READS`]) and
//! is handed a [`ModelView`] holding those paths only (ADR 0070).

use std::collections::BTreeMap;
use std::fmt;

use crate::itf::{State, Trace, Value};

/// The `nondet` values one step picked, by parameter name: `mbt::nondetPicks` with each
/// `Some(v)` unwrapped to `v` and every `None` (a parameter the action did not pick) absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params(BTreeMap<String, Value>);

impl Params {
    /// Parameters from name-value pairs (for drivers' own tests).
    pub fn new(picks: BTreeMap<String, Value>) -> Self {
        Self(picks)
    }

    /// The picked value, if the action picked `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    /// The picked value; [`DriverError::MissingParam`] if the action did not pick `name`.
    pub fn value(&self, name: &str) -> Result<&Value, DriverError> {
        self.get(name)
            .ok_or_else(|| DriverError::MissingParam(name.to_owned()))
    }

    /// A picked boolean.
    pub fn bool(&self, name: &str) -> Result<bool, DriverError> {
        let value = self.value(name)?;
        value
            .as_bool()
            .ok_or_else(|| DriverError::bad_param(name, "a bool", value))
    }

    /// A picked integer.
    pub fn int(&self, name: &str) -> Result<i128, DriverError> {
        let value = self.value(name)?;
        value
            .as_int()
            .ok_or_else(|| DriverError::bad_param(name, "an int", value))
    }

    /// A picked string.
    pub fn str(&self, name: &str) -> Result<&str, DriverError> {
        let value = self.value(name)?;
        value
            .as_str()
            .ok_or_else(|| DriverError::bad_param(name, "a string", value))
    }

    /// The tag of a picked variant (a model enum such as a file kind).
    pub fn tag(&self, name: &str) -> Result<&str, DriverError> {
        let value = self.value(name)?;
        value
            .tag()
            .ok_or_else(|| DriverError::bad_param(name, "a variant", value))
    }

    /// Whether nothing was picked.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The picked parameters, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value))
    }
}

impl fmt::Display for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, value)) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{name} = {value}")?;
        }
        Ok(())
    }
}

/// Why a driver could not apply a step. Each one is a divergence: the model took the step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverError {
    /// The driver does not know the action (a model change without a driver change).
    UnknownAction(String),
    /// The action needs a parameter the step did not pick.
    MissingParam(String),
    /// A picked parameter has the wrong shape.
    BadParam {
        /// The parameter's name.
        name: String,
        /// What the driver expected, as "a bool".
        expected: &'static str,
        /// What the trace held.
        found: String,
    },
    /// The real code refused or failed the stimulus the model accepted.
    Refused(String),
}

impl DriverError {
    /// A parameter of the wrong shape.
    pub fn bad_param(name: &str, expected: &'static str, found: &Value) -> Self {
        Self::BadParam {
            name: name.to_owned(),
            expected,
            found: found.to_string(),
        }
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAction(action) => write!(f, "the driver has no action {action:?}"),
            Self::MissingParam(name) => write!(f, "the step picked no parameter {name:?}"),
            Self::BadParam {
                name,
                expected,
                found,
            } => write!(f, "parameter {name:?} is not {expected}: {found}"),
            Self::Refused(reason) => write!(f, "the implementation refused: {reason}"),
        }
    }
}

impl std::error::Error for DriverError {}

/// Why the model's state could not be projected into an observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionError(pub String);

impl ProjectionError {
    /// A variable or field the projection needs is absent or has the wrong shape.
    pub fn missing(what: &str) -> Self {
        Self(format!("{what} is missing or has the wrong shape"))
    }
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectionError {}

/// Applies a model's actions to the real code and observes it.
///
/// `apply` is called once per ITF state, in order, starting with the first state's action
/// `init`, where the driver builds (or resets to) its initial implementation state. After
/// each call [`replay`] compares [`Driver::observe`] with the projection of the model's state.
/// The observation must be built from what the real code exposes, never recomputed from the
/// model (foundation.md section 6.3, step 4).
pub trait Driver {
    /// The canonical model-facing observation: the same type the model's state projects to.
    type Obs: PartialEq + fmt::Debug;

    /// The model paths [`Driver::apply_with_model`] may read, as [`ModelView`] patterns
    /// (`browsers.*.doc`): only choices the model's environment makes outside the
    /// implementation, never a variable the driver observes from the real code, which would
    /// make the comparison vacuous (ADR 0070). None by default.
    const MODEL_READS: &'static [&'static str] = &[];

    /// Applies one action with its picked parameters to the real code.
    fn apply(&mut self, action: &str, params: &Params) -> Result<(), DriverError>;

    /// [`Driver::apply`], with the model's state after the step at hand, restricted to
    /// [`Driver::MODEL_READS`]. [`replay`] calls this one; the default ignores `model` and calls
    /// [`Driver::apply`].
    ///
    /// For a step of the environment whose stimulus to the real code is a choice the model
    /// makes outside the implementation, such as a browser's decision to navigate away and so
    /// close its event stream (D16: the client is verified elsewhere, never reimplemented in
    /// Rust). `model` holds nothing but the declared paths, and a read of any other path is a
    /// [`DriverError::Refused`].
    ///
    /// # Errors
    ///
    /// As [`Driver::apply`].
    fn apply_with_model(
        &mut self,
        action: &str,
        params: &Params,
        model: &ModelView,
    ) -> Result<(), DriverError> {
        let _ = model;
        self.apply(action, params)
    }

    /// Projects the real code's current state.
    fn observe(&self) -> Self::Obs;
}

/// The part of a model state a driver declared it reads ([`Driver::MODEL_READS`]), and
/// nothing else: [`replay`] builds it from each state, so the rest of the state never reaches
/// the driver (ADR 0070).
///
/// A pattern is a variable name followed by `.`-separated steps: a record field by name, a map
/// entry by its string key, or `*` for every entry of a map. `browsers.*.doc` keeps each
/// browser's `doc` and nothing else of `browsers`, or of any other variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelView {
    patterns: &'static [&'static str],
    vars: BTreeMap<String, Value>,
}

impl ModelView {
    /// The paths of `state` that `patterns` name, pruned of everything else.
    pub fn new(state: &State, patterns: &'static [&'static str]) -> Self {
        let mut vars: BTreeMap<String, Value> = BTreeMap::new();
        for pattern in patterns {
            let steps: Vec<&str> = pattern.split('.').collect();
            let Some((var, rest)) = steps.split_first() else {
                continue;
            };
            let Some(kept) = state.var(var).and_then(|value| prune(value, rest)) else {
                continue;
            };
            let merged = match vars.remove(*var) {
                Some(earlier) => merge(earlier, kept),
                None => kept,
            };
            vars.insert((*var).to_owned(), merged);
        }
        Self { patterns, vars }
    }

    /// The value at `path` (a variable name, then record fields and map keys).
    ///
    /// # Errors
    ///
    /// [`DriverError::Refused`] when no declared pattern covers `path` (a driver reading what
    /// it did not declare), or when the model has nothing there.
    pub fn read(&self, path: &[&str]) -> Result<&Value, DriverError> {
        let shown = path.join(".");
        if !self.patterns.iter().any(|pattern| covers(pattern, path)) {
            return Err(DriverError::Refused(format!(
                "the driver read the model path {shown}, which its MODEL_READS does not declare"
            )));
        }
        let missing = || DriverError::Refused(format!("the model has no {shown}"));
        let (var, rest) = path.split_first().ok_or_else(missing)?;
        let mut value = self.vars.get(*var).ok_or_else(missing)?;
        for step in rest {
            value = match value {
                Value::Record(fields) => fields.get(*step),
                Value::Map(entries) => entries.get(&Value::Str((*step).to_owned())),
                _ => None,
            }
            .ok_or_else(missing)?;
        }
        Ok(value)
    }
}

/// Whether `pattern` covers the concrete `path` or a value inside it.
fn covers(pattern: &str, path: &[&str]) -> bool {
    let steps: Vec<&str> = pattern.split('.').collect();
    path.len() >= steps.len()
        && steps
            .iter()
            .zip(path)
            .all(|(step, part)| *step == "*" || step == part)
}

/// `value` with only what the pattern steps `rest` reach; `None` when they reach nothing.
fn prune(value: &Value, rest: &[&str]) -> Option<Value> {
    let Some((step, rest)) = rest.split_first() else {
        return Some(value.clone());
    };
    match value {
        Value::Record(fields) => {
            let kept = prune(fields.get(*step)?, rest)?;
            Some(Value::Record(BTreeMap::from([((*step).to_owned(), kept)])))
        }
        Value::Map(entries) => {
            let kept: BTreeMap<Value, Value> = entries
                .iter()
                .filter(|(key, _)| *step == "*" || key.as_str() == Some(*step))
                .filter_map(|(key, entry)| Some((key.clone(), prune(entry, rest)?)))
                .collect();
            (!kept.is_empty()).then_some(Value::Map(kept))
        }
        _ => None,
    }
}

/// The union of two pruned values of one variable.
fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Record(mut left), Value::Record(right)) => {
            for (name, value) in right {
                let merged = match left.remove(&name) {
                    Some(earlier) => merge(earlier, value),
                    None => value,
                };
                left.insert(name, merged);
            }
            Value::Record(left)
        }
        (Value::Map(mut left), Value::Map(right)) => {
            for (key, value) in right {
                let merged = match left.remove(&key) {
                    Some(earlier) => merge(earlier, value),
                    None => value,
                };
                left.insert(key, merged);
            }
            Value::Map(left)
        }
        // Two patterns that reach the same whole value keep it once.
        (_, whole) => whole,
    }
}

/// What went wrong at a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The observation differs from the model's state.
    Mismatch,
    /// The driver could not apply the step.
    Driver(DriverError),
    /// The model's state could not be projected.
    Projection(ProjectionError),
    /// The step itself is unusable (no action name, a malformed parameter record, no states).
    Trace(String),
}

/// The first step at which the implementation and the model disagree, with everything a
/// report needs (foundation.md section 6.3, step 6); [`fmt::Display`] is the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The model's source (`#meta.source`), when the trace records it.
    pub model: Option<String>,
    /// The seed the trace was generated with, when known.
    pub seed: Option<String>,
    /// The trace's id (its file name).
    pub trace_id: String,
    /// The index of the state whose step diverged (0 is `init`).
    pub step: usize,
    /// That step's action.
    pub action: String,
    /// That step's picked parameters, formatted.
    pub params: String,
    /// The model's projected state, pretty-printed, when it could be projected.
    pub expected: Option<String>,
    /// The implementation's observation, pretty-printed, when the step got that far.
    pub actual: Option<String>,
    /// What went wrong.
    pub cause: Cause,
}

/// Replays `trace` against `driver`, comparing after every step.
///
/// `project_expected` maps a model state to the driver's observation type. The replay stops
/// at the first divergence; a trace with no states is one (a vacuous replay must not pass),
/// and so is a trace whose length differs from the one its manifest records
/// ([`Trace::with_max_steps`]): a script or a generation cut short would otherwise pass on its
/// first states. The error is boxed because a report is large and the success path carries
/// nothing.
pub fn replay<D, P>(
    trace: &Trace,
    driver: &mut D,
    project_expected: P,
) -> Result<(), Box<Divergence>>
where
    D: Driver,
    P: Fn(&State) -> Result<D::Obs, ProjectionError>,
{
    let diverge = |step: usize, action: &str, params: String, cause: Cause| Divergence {
        model: trace.source.clone(),
        seed: trace.seed.clone(),
        trace_id: trace.id.clone(),
        step,
        action: action.to_owned(),
        params,
        expected: None,
        actual: None,
        cause,
    };
    if trace.states.is_empty() {
        return Err(Box::new(diverge(
            0,
            "",
            String::new(),
            Cause::Trace("the trace has no states".into()),
        )));
    }
    if let Some(steps) = trace.max_steps
        && trace.states.len() != steps + 1
    {
        let cause = Cause::Trace(format!(
            "the trace has {} states, not the {} its manifest's max_steps {steps} gives",
            trace.states.len(),
            steps + 1
        ));
        return Err(Box::new(diverge(0, "", String::new(), cause)));
    }
    for state in &trace.states {
        let step = state.index;
        let Some(action) = state.action_taken() else {
            let cause = Cause::Trace(format!(
                "no {} (generate the trace with --mbt)",
                crate::itf::ACTION_TAKEN
            ));
            return Err(Box::new(diverge(step, "", String::new(), cause)));
        };
        let Some(picks) = state.nondet_picks() else {
            let cause = Cause::Trace(format!(
                "{} is missing or not a record of options",
                crate::itf::NONDET_PICKS
            ));
            return Err(Box::new(diverge(step, action, String::new(), cause)));
        };
        let params = Params::new(picks);
        let expected = project_expected(state);
        let view = ModelView::new(state, D::MODEL_READS);
        let applied = driver.apply_with_model(action, &params, &view);
        let actual = driver.observe();
        let mut divergence = diverge(step, action, params.to_string(), Cause::Mismatch);
        divergence.actual = Some(format!("{actual:#?}"));
        match expected {
            Err(err) => {
                divergence.cause = Cause::Projection(err);
                return Err(Box::new(divergence));
            }
            Ok(expected) => {
                divergence.expected = Some(format!("{expected:#?}"));
                if let Err(err) = applied {
                    divergence.cause = Cause::Driver(err);
                    return Err(Box::new(divergence));
                }
                if expected != actual {
                    return Err(Box::new(divergence));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itf::Trace;

    const SAMPLE: &str =
        include_str!("../../../formal/traces/regression/itf-decoder-sample.itf.json");

    fn sample() -> Trace {
        Trace::from_json_str(SAMPLE, "itf-decoder-sample.itf.json")
            .expect("the sample decodes")
            .with_seed("11")
    }

    /// The toy observation: a projection of the lifecycle model's `state`, `res.listeners`
    /// and `response`.
    #[derive(Debug, PartialEq, Eq)]
    struct Obs {
        state: String,
        listeners: i128,
        response: String,
    }

    fn project(state: &State) -> Result<Obs, ProjectionError> {
        let tag = |name: &str| {
            state
                .var(name)
                .and_then(Value::tag)
                .map(str::to_owned)
                .ok_or_else(|| ProjectionError::missing(name))
        };
        Ok(Obs {
            state: tag("state")?,
            listeners: state
                .var("res")
                .and_then(|res| res.field("listeners"))
                .and_then(Value::as_int)
                .ok_or_else(|| ProjectionError::missing("res.listeners"))?,
            response: tag("response")?,
        })
    }

    /// Which deliberate fault the toy driver has.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        /// Keeps the listener after a stop.
        LeakOnStop,
        /// Leaves the first request after a start unanswered, then behaves.
        DropFirstRequest,
        /// Does not know the `request` action.
        NoRequestAction,
    }

    /// A toy stand-in for a real driver: it tests the harness, not the server.
    struct Toy {
        fault: Fault,
        state: &'static str,
        listeners: i128,
        response: &'static str,
        dropped: bool,
    }

    impl Toy {
        fn new(fault: Fault) -> Self {
            Self {
                fault,
                state: "Stopped",
                listeners: 0,
                response: "NoRequest",
                dropped: false,
            }
        }
    }

    impl Driver for Toy {
        type Obs = Obs;

        fn apply(&mut self, action: &str, params: &Params) -> Result<(), DriverError> {
            self.response = "NoRequest";
            match action {
                "init" => *self = Self::new(self.fault),
                "startServer" => {
                    let bind_ok = params.bool("bindOk")?;
                    params.bool("watcherOk")?;
                    if self.state != "Stopped" {
                        return Err(DriverError::Refused("already started".into()));
                    }
                    (self.state, self.listeners) = if bind_ok {
                        ("Serving", 1)
                    } else {
                        ("Failed", 0)
                    };
                }
                "stopServer" if self.state == "Serving" => {
                    self.state = "Stopped";
                    if self.fault != Fault::LeakOnStop {
                        self.listeners = 0;
                    }
                }
                "stopServer" | "refuseStart" => {}
                "request" if self.fault == Fault::NoRequestAction => {
                    return Err(DriverError::UnknownAction(action.to_owned()));
                }
                "request" | "openStream" => {
                    let drop_it = self.fault == Fault::DropFirstRequest && !self.dropped;
                    self.response = if self.state == "Serving" && !drop_it {
                        "Answered"
                    } else {
                        "Unanswered"
                    };
                    self.dropped |= drop_it && self.state == "Serving";
                }
                other => return Err(DriverError::UnknownAction(other.to_owned())),
            }
            Ok(())
        }

        fn observe(&self) -> Obs {
            Obs {
                state: self.state.to_owned(),
                listeners: self.listeners,
                response: self.response.to_owned(),
            }
        }
    }

    #[test]
    fn should_replay_every_step_when_the_driver_matches_the_model() {
        // given
        let trace = sample();
        let mut driver = Toy::new(Fault::None);

        // when
        let outcome = replay(&trace, &mut driver, project);

        // then
        assert_eq!(outcome, Ok(()));
        assert_eq!(driver.state, "Failed");
    }

    #[test]
    fn should_report_step_and_diff_when_observation_diverges() {
        // given
        let trace = sample();
        let mut driver = Toy::new(Fault::LeakOnStop);

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("the leak is caught");

        // then
        assert_eq!(divergence.step, 4);
        assert_eq!(divergence.action, "stopServer");
        assert_eq!(divergence.cause, Cause::Mismatch);
        let expected = divergence.expected.as_deref().expect("expected");
        let actual = divergence.actual.as_deref().expect("actual");
        assert!(expected.contains("listeners: 0,"), "{expected}");
        assert!(actual.contains("listeners: 1,"), "{actual}");
        let report = divergence.to_string();
        for line in [
            "model:    formal/lifecycle.qnt",
            "seed:     11",
            "trace:    itf-decoder-sample.itf.json",
            "step:     4",
            "action:   stopServer",
            "    -     listeners: 0,",
            "    +     listeners: 1,",
        ] {
            assert!(report.contains(line), "missing {line:?} in:\n{report}");
        }
        assert!(
            !report.contains("-     state:"),
            "unchanged lines are not diffed:\n{report}"
        );
    }

    #[test]
    fn should_stop_at_the_transient_step_when_a_later_state_agrees_again() {
        // given
        let trace = sample();
        let mut driver = Toy::new(Fault::DropFirstRequest);

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("caught mid-trace");

        // then
        assert_eq!(
            (divergence.step, divergence.action.as_str()),
            (2, "request")
        );
        assert_eq!(divergence.cause, Cause::Mismatch);
    }

    #[test]
    fn should_report_the_driver_error_when_the_driver_lacks_an_action() {
        // given
        let trace = sample();
        let mut driver = Toy::new(Fault::NoRequestAction);

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("refused");

        // then
        assert_eq!(divergence.step, 2);
        assert_eq!(
            divergence.cause,
            Cause::Driver(DriverError::UnknownAction("request".into()))
        );
        assert!(
            divergence
                .to_string()
                .contains("the driver has no action \"request\"")
        );
    }

    #[test]
    fn should_report_the_projection_error_when_the_model_state_lacks_a_variable() {
        // given
        let trace = sample();
        let mut driver = Toy::new(Fault::None);
        let project_missing =
            |_: &State| -> Result<Obs, ProjectionError> { Err(ProjectionError::missing("absent")) };

        // when
        let divergence = replay(&trace, &mut driver, project_missing).expect_err("unprojectable");

        // then
        assert_eq!(divergence.step, 0);
        assert_eq!(divergence.action, "init");
        assert!(matches!(divergence.cause, Cause::Projection(_)));
    }

    #[test]
    fn should_refuse_a_trace_without_mbt_variables_when_replaying() {
        // given
        let trace = Trace::from_json_str(
            r##"{"vars": ["x"], "states": [{"#meta": {"index": 0}, "x": 1}]}"##,
            "plain.itf.json",
        )
        .expect("decodes");
        let mut driver = Toy::new(Fault::None);

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("no action names");

        // then
        assert!(matches!(&divergence.cause, Cause::Trace(why) if why.contains("--mbt")));
    }

    #[test]
    fn should_refuse_an_empty_trace_when_replaying() {
        // given
        let trace = Trace::from_json_str(r#"{"states": []}"#, "empty.itf.json").expect("decodes");
        let mut driver = Toy::new(Fault::None);

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("vacuous");

        // then
        assert!(matches!(divergence.cause, Cause::Trace(_)));
    }

    #[test]
    fn should_name_the_missing_and_misshapen_params_when_decoding_picks() {
        // given
        let params = Params::new(BTreeMap::from([
            ("flag".to_owned(), Value::Bool(true)),
            ("count".to_owned(), Value::Int(3)),
        ]));

        // when
        let results = (
            params.bool("flag"),
            params.int("count"),
            params.bool("count"),
            params.str("nope"),
        );

        // then
        assert_eq!(results.0, Ok(true));
        assert_eq!(results.1, Ok(3));
        assert_eq!(
            results.2,
            Err(DriverError::bad_param("count", "a bool", &Value::Int(3)))
        );
        assert_eq!(results.3, Err(DriverError::MissingParam("nope".into())));
        assert_eq!(params.to_string(), "count = 3, flag = true");
    }

    /// A driver that reads one model path at every step and records what it read.
    struct Peeker {
        path: &'static [&'static str],
        read: Vec<Value>,
    }

    impl Driver for Peeker {
        type Obs = Obs;

        const MODEL_READS: &'static [&'static str] = &["res.listeners"];

        fn apply(&mut self, _action: &str, _params: &Params) -> Result<(), DriverError> {
            Ok(())
        }

        fn apply_with_model(
            &mut self,
            _action: &str,
            _params: &Params,
            model: &ModelView,
        ) -> Result<(), DriverError> {
            self.read.push(model.read(self.path)?.clone());
            Ok(())
        }

        fn observe(&self) -> Obs {
            Obs {
                state: String::new(),
                listeners: 0,
                response: String::new(),
            }
        }
    }

    #[test]
    fn should_refuse_the_read_when_driver_reads_an_undeclared_model_path() {
        // given: a driver that declares `res.listeners` and reads `res.serveTasks`
        let trace = sample();
        let mut driver = Peeker {
            path: &["res", "serveTasks"],
            read: Vec::new(),
        };

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("refused");

        // then
        assert_eq!(divergence.step, 0);
        assert_eq!(
            divergence.cause,
            Cause::Driver(DriverError::Refused(
                "the driver read the model path res.serveTasks, which its MODEL_READS does not \
                 declare"
                    .to_owned()
            ))
        );
    }

    #[test]
    fn should_hand_the_declared_path_only_when_driver_reads_the_model() {
        // given
        let state = &sample().states[1];

        // when
        let view = ModelView::new(state, Peeker::MODEL_READS);

        // then: the declared value, and nothing else of `res` or of any other variable
        let listeners = state.var("res").and_then(|r| r.field("listeners"));
        assert_eq!(view.read(&["res", "listeners"]).ok(), listeners);
        assert_eq!(
            view,
            ModelView::new(
                &State {
                    index: 9,
                    vars: BTreeMap::from([(
                        "res".to_owned(),
                        Value::Record(BTreeMap::from([(
                            "listeners".to_owned(),
                            listeners.expect("res.listeners").clone()
                        )]))
                    )]),
                },
                Peeker::MODEL_READS
            )
        );
        for undeclared in [&["res", "serveTasks"][..], &["state"], &["res"]] {
            assert!(
                matches!(view.read(undeclared), Err(DriverError::Refused(why)) if why.contains("does not declare")),
                "{undeclared:?}"
            );
        }
    }

    #[test]
    fn should_keep_each_entry_of_a_map_when_pattern_names_every_key() {
        // given: two browsers with a page and a counter each, and another variable
        let browser = |doc: &str| {
            Value::Record(BTreeMap::from([
                ("doc".to_owned(), Value::Str(doc.to_owned())),
                ("reloaded".to_owned(), Value::Int(3)),
            ]))
        };
        let state = State {
            index: 0,
            vars: BTreeMap::from([
                (
                    "browsers".to_owned(),
                    Value::Map(BTreeMap::from([
                        (Value::Str("b1".into()), browser("Shown")),
                        (Value::Str("b2".into()), browser("Navigating")),
                    ])),
                ),
                ("up".to_owned(), Value::Bool(true)),
            ]),
        };

        // when
        let view = ModelView::new(&state, &["browsers.*.doc", "browsers.b1.doc"]);

        // then
        assert_eq!(
            view.read(&["browsers", "b2", "doc"]),
            Ok(&Value::Str("Navigating".into()))
        );
        assert_eq!(
            view.read(&["browsers", "b1", "doc"]),
            Ok(&Value::Str("Shown".into()))
        );
        assert!(view.read(&["browsers", "b1", "reloaded"]).is_err());
        assert!(view.read(&["up"]).is_err());
        assert_eq!(
            view.read(&["browsers", "b3", "doc"]),
            Err(DriverError::Refused(
                "the model has no browsers.b3.doc".to_owned()
            ))
        );
    }

    #[test]
    fn should_reject_a_longer_or_shorter_path_when_checking_the_patterns() {
        // given
        let pattern = "browsers.*.doc";

        // when
        let results = (
            covers(pattern, &["browsers", "b1", "doc"]),
            covers(pattern, &["browsers", "b1", "doc", "inner"]),
            covers(pattern, &["browsers", "b1"]),
            covers(pattern, &["browsers", "b1", "open"]),
        );

        // then
        assert_eq!(results, (true, true, false, false));
    }

    #[test]
    fn should_refuse_a_trace_of_another_length_when_its_manifest_gives_one() {
        // given: the five-step sample, said to have four steps
        let trace = sample().with_max_steps(4);
        let mut driver = Toy::new(Fault::None);

        // when
        let divergence = replay(&trace, &mut driver, project).expect_err("another length");

        // then
        assert_eq!(
            divergence.cause,
            Cause::Trace(
                "the trace has 6 states, not the 5 its manifest's max_steps 4 gives".to_owned()
            )
        );
        let mut driver = Toy::new(Fault::None);
        assert_eq!(
            replay(&sample().with_max_steps(5), &mut driver, project),
            Ok(())
        );
    }
}
