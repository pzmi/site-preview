//! A narrow decoder for the ITF traces Quint writes (`quint run --out-itf ... --mbt`).
//!
//! It covers what the models use and what the ITF format defines for them
//! (<https://apalache-mc.org/docs/adr/015adr-trace.html>): booleans, strings, integers (plain
//! JSON numbers or `{"#bigint": "..."}`), lists (JSON arrays), tuples (`{"#tup": [...]}`, so
//! also the unit value `{"#tup": []}`), sets (`{"#set": [...]}`), maps
//! (`{"#map": [[key, value], ...]}`), records (any other object) and variants (an object of
//! exactly `tag` and `value`, which is how Quint writes a sum type). Anything else, such as
//! `#unserializable`, a fraction or an integer outside `i128`, is a [`DecodeError`]: the
//! decoder refuses rather than guesses (ADR 0053).
//!
//! The decoder is local rather than the `itf` crate because the replay path must not depend
//! on the harness library under evaluation, and the format subset is small (ADR 0053).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde_json::Value as Json;

/// The variable Quint's `--mbt` flag adds for the action a step took.
pub const ACTION_TAKEN: &str = "mbt::actionTaken";
/// The variable Quint's `--mbt` flag adds for the `nondet` values a step picked.
pub const NONDET_PICKS: &str = "mbt::nondetPicks";

/// A decoded ITF value.
///
/// Sets and maps are ordered by value, so two traces that list the same set in another order
/// decode to equal values.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// An integer: a JSON number or `{"#bigint": "..."}`.
    Int(i128),
    /// A string.
    Str(String),
    /// A list (a JSON array).
    List(Vec<Value>),
    /// A tuple, `{"#tup": [...]}`; the unit value is the empty tuple.
    Tuple(Vec<Value>),
    /// A set, `{"#set": [...]}`.
    Set(BTreeSet<Value>),
    /// A map, `{"#map": [[key, value], ...]}`.
    Map(BTreeMap<Value, Value>),
    /// A record: field name to value.
    Record(BTreeMap<String, Value>),
    /// A variant of a sum type: `{"tag": ..., "value": ...}`.
    Variant {
        /// The constructor's name.
        tag: String,
        /// Its payload; the unit tuple for a constructor without one.
        value: Box<Value>,
    },
}

impl Value {
    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The integer, if this is one.
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The elements of a list, if this is one.
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    /// A record's field, if this is a record that has it.
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Record(fields) => fields.get(name),
            _ => None,
        }
    }

    /// The tag and payload, if this is a variant.
    pub fn as_variant(&self) -> Option<(&str, &Value)> {
        match self {
            Self::Variant { tag, value } => Some((tag, value)),
            _ => None,
        }
    }

    /// The tag alone, if this is a variant.
    pub fn tag(&self) -> Option<&str> {
        self.as_variant().map(|(tag, _)| tag)
    }

    /// Whether this is the unit value `{"#tup": []}`.
    pub fn is_unit(&self) -> bool {
        matches!(self, Self::Tuple(items) if items.is_empty())
    }

    /// Reads a Quint `Option`: `Some(Some(v))` for `Some(v)`, `Some(None)` for `None`, and
    /// `None` when this is not an option at all.
    pub fn as_option(&self) -> Option<Option<&Value>> {
        match self.as_variant()? {
            ("Some", value) => Some(Some(value)),
            ("None", value) if value.is_unit() => Some(None),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn seq<'a>(
            f: &mut fmt::Formatter<'_>,
            open: &str,
            items: impl Iterator<Item = &'a Value>,
            close: &str,
        ) -> fmt::Result {
            f.write_str(open)?;
            for (i, item) in items.enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{item}")?;
            }
            f.write_str(close)
        }
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(n) => write!(f, "{n}"),
            Self::Str(s) => write!(f, "{s:?}"),
            Self::List(items) => seq(f, "[", items.iter(), "]"),
            Self::Tuple(items) => seq(f, "(", items.iter(), ")"),
            Self::Set(items) => seq(f, "Set(", items.iter(), ")"),
            Self::Map(entries) => {
                f.write_str("Map(")?;
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{key} -> {value}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("{ ")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{name}: {value}")?;
                }
                f.write_str(" }")
            }
            Self::Variant { tag, value } if value.is_unit() => f.write_str(tag),
            Self::Variant { tag, value } => write!(f, "{tag}({value})"),
        }
    }
}

/// Why a trace could not be decoded. `path` locates the offending JSON value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// Where in the document, as `states[2].res.listeners`.
    pub path: String,
    /// What was wrong there.
    pub reason: String,
}

impl DecodeError {
    fn at(path: &str, reason: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "invalid ITF trace: {}", self.reason)
        } else {
            write!(f, "invalid ITF trace at {}: {}", self.path, self.reason)
        }
    }
}

impl std::error::Error for DecodeError {}

/// One state of a trace: its index and every variable's value, including the `mbt::` ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// The state's position in the trace, from `#meta.index` (else its array position).
    pub index: usize,
    /// Each variable's value.
    pub vars: BTreeMap<String, Value>,
}

impl State {
    /// A variable's value.
    pub fn var(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    /// The action that produced this state (`mbt::actionTaken`; `init` for the first state).
    pub fn action_taken(&self) -> Option<&str> {
        self.var(ACTION_TAKEN).and_then(Value::as_str)
    }

    /// The `nondet` values the action picked: `mbt::nondetPicks` with each `Some(v)` unwrapped
    /// and each `None` (a parameter this action did not pick) left out. `None` when the trace
    /// was written without `--mbt`, or the variable is not a record of options.
    pub fn nondet_picks(&self) -> Option<BTreeMap<String, Value>> {
        let Value::Record(fields) = self.var(NONDET_PICKS)? else {
            return None;
        };
        let mut picks = BTreeMap::new();
        for (name, value) in fields {
            if let Some(picked) = value.as_option()? {
                picks.insert(name.clone(), picked.clone());
            }
        }
        Some(picks)
    }

    /// The state without the `mbt::` variables: what the model itself holds.
    pub fn model_vars(&self) -> BTreeMap<&str, &Value> {
        self.vars
            .iter()
            .filter(|(name, _)| !name.starts_with("mbt::"))
            .map(|(name, value)| (name.as_str(), value))
            .collect()
    }
}

/// A decoded trace plus where it came from, which every divergence report names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    /// The trace's identity in reports: the file name for a loaded trace.
    pub id: String,
    /// The model it was generated from (`#meta.source`), when recorded.
    pub source: Option<String>,
    /// The seed it was generated with. ITF does not record it, so the caller sets it (from
    /// the corpus manifest or the generating command) with [`Trace::with_seed`].
    pub seed: Option<String>,
    /// The variable names in `vars`, in the trace's order.
    pub vars: Vec<String>,
    /// The states, first to last.
    pub states: Vec<State>,
}

impl Trace {
    /// Decodes an ITF document, naming it `id` in reports.
    pub fn from_json_str(text: &str, id: impl Into<String>) -> Result<Self, DecodeError> {
        let json: Json = serde_json::from_str(text)
            .map_err(|err| DecodeError::at("", format!("not JSON: {err}")))?;
        Self::from_json(&json, id.into())
    }

    /// Reads and decodes an ITF file; its file name is the trace's id.
    pub fn load(path: &Path) -> Result<Self, DecodeError> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| DecodeError::at("", format!("cannot read {}: {err}", path.display())))?;
        let id = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::from_json_str(&text, id)
    }

    /// Records the seed the trace was generated with.
    #[must_use]
    pub fn with_seed(mut self, seed: impl Into<String>) -> Self {
        self.seed = Some(seed.into());
        self
    }

    fn from_json(json: &Json, id: String) -> Result<Self, DecodeError> {
        let Json::Object(top) = json else {
            return Err(DecodeError::at("", "the document is not an object"));
        };
        let source = match top.get("#meta").and_then(|meta| meta.get("source")) {
            None => None,
            Some(Json::String(s)) => Some(s.clone()),
            Some(_) => return Err(DecodeError::at("#meta.source", "not a string")),
        };
        let vars = match top.get("vars") {
            None => Vec::new(),
            Some(Json::Array(names)) => names
                .iter()
                .enumerate()
                .map(|(i, name)| match name {
                    Json::String(s) => Ok(s.clone()),
                    _ => Err(DecodeError::at(&format!("vars[{i}]"), "not a string")),
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(DecodeError::at("vars", "not an array")),
        };
        let Some(Json::Array(raw_states)) = top.get("states") else {
            return Err(DecodeError::at("states", "missing, or not an array"));
        };
        let states = raw_states
            .iter()
            .enumerate()
            .map(|(i, raw)| decode_state(raw, i))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id,
            source,
            seed: None,
            vars,
            states,
        })
    }
}

fn decode_state(raw: &Json, position: usize) -> Result<State, DecodeError> {
    let path = format!("states[{position}]");
    let Json::Object(fields) = raw else {
        return Err(DecodeError::at(&path, "not an object"));
    };
    let index = match fields.get("#meta").and_then(|meta| meta.get("index")) {
        None => position,
        Some(n) => n
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| DecodeError::at(&format!("{path}.#meta.index"), "not an index"))?,
    };
    let mut vars = BTreeMap::new();
    for (name, value) in fields {
        if name == "#meta" {
            continue;
        }
        vars.insert(
            name.clone(),
            decode_value(value, &format!("{path}.{name}"))?,
        );
    }
    Ok(State { index, vars })
}

/// Decodes one ITF value; `path` names it in errors.
pub fn decode_value(json: &Json, path: &str) -> Result<Value, DecodeError> {
    match json {
        Json::Null => Err(DecodeError::at(path, "null is not an ITF value")),
        Json::Bool(b) => Ok(Value::Bool(*b)),
        Json::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .map(Value::Int)
            .ok_or_else(|| DecodeError::at(path, format!("{n} is not an integer"))),
        Json::String(s) => Ok(Value::Str(s.clone())),
        Json::Array(items) => decode_items(items, path).map(Value::List),
        Json::Object(fields) => decode_object(fields, path),
    }
}

fn decode_items(items: &[Json], path: &str) -> Result<Vec<Value>, DecodeError> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| decode_value(item, &format!("{path}[{i}]")))
        .collect()
}

fn decode_object(fields: &serde_json::Map<String, Json>, path: &str) -> Result<Value, DecodeError> {
    if let Some((key, _)) = fields.iter().find(|(key, _)| key.starts_with('#')) {
        if fields.len() != 1 {
            return Err(DecodeError::at(
                path,
                format!("{key} is not alone in its object"),
            ));
        }
        return decode_special(key, &fields[key.as_str()], &format!("{path}.{key}"));
    }
    if fields.len() == 2
        && let (Some(Json::String(tag)), Some(value)) = (fields.get("tag"), fields.get("value"))
    {
        return Ok(Value::Variant {
            tag: tag.clone(),
            value: Box::new(decode_value(value, &format!("{path}.value"))?),
        });
    }
    let mut record = BTreeMap::new();
    for (name, value) in fields {
        record.insert(
            name.clone(),
            decode_value(value, &format!("{path}.{name}"))?,
        );
    }
    Ok(Value::Record(record))
}

fn decode_special(key: &str, body: &Json, path: &str) -> Result<Value, DecodeError> {
    match (key, body) {
        ("#bigint", Json::String(digits)) => digits
            .parse::<i128>()
            .map(Value::Int)
            .map_err(|_| DecodeError::at(path, format!("{digits:?} is not an integer in i128"))),
        ("#tup", Json::Array(items)) => decode_items(items, path).map(Value::Tuple),
        ("#set", Json::Array(items)) => {
            let mut set = BTreeSet::new();
            for (i, item) in items.iter().enumerate() {
                let element_path = format!("{path}[{i}]");
                if !set.insert(decode_value(item, &element_path)?) {
                    return Err(DecodeError::at(&element_path, "duplicate set element"));
                }
            }
            Ok(Value::Set(set))
        }
        ("#map", Json::Array(entries)) => {
            let mut map = BTreeMap::new();
            for (i, entry) in entries.iter().enumerate() {
                let entry_path = format!("{path}[{i}]");
                let Json::Array(pair) = entry else {
                    return Err(DecodeError::at(&entry_path, "a map entry is not a pair"));
                };
                let [key, value] = pair.as_slice() else {
                    return Err(DecodeError::at(&entry_path, "a map entry is not a pair"));
                };
                let key = decode_value(key, &format!("{entry_path}[0]"))?;
                let value = decode_value(value, &format!("{entry_path}[1]"))?;
                if map.insert(key, value).is_some() {
                    return Err(DecodeError::at(&entry_path, "duplicate map key"));
                }
            }
            Ok(Value::Map(map))
        }
        ("#bigint" | "#tup" | "#set" | "#map", _) => {
            Err(DecodeError::at(path, "has the wrong JSON type"))
        }
        _ => Err(DecodeError::at(path, format!("{key} is not supported"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `quint run formal/lifecycle.qnt --backend typescript --step serverStep --max-samples 1
    /// --max-steps 5 --seed 11 --mbt --verbosity 0 --out-itf <this file>`, from the repo root.
    pub(crate) const SAMPLE: &str =
        include_str!("../../../formal/traces/regression/itf-decoder-sample.itf.json");

    fn sample() -> Trace {
        Trace::from_json_str(SAMPLE, "itf-decoder-sample.itf.json").expect("the sample decodes")
    }

    fn value(text: &str) -> Result<Value, DecodeError> {
        let json: Json = serde_json::from_str(text).expect("test JSON");
        decode_value(&json, "v")
    }

    #[test]
    fn should_decode_every_state_when_reading_the_committed_sample() {
        // given
        let text = SAMPLE;

        // when
        let trace = Trace::from_json_str(text, "itf-decoder-sample.itf.json");

        // then
        let trace = trace.expect("the sample decodes");
        assert_eq!(trace.source.as_deref(), Some("formal/lifecycle.qnt"));
        assert_eq!(trace.seed, None);
        assert_eq!(trace.states.len(), 6);
        assert!(trace.states.iter().enumerate().all(|(i, s)| s.index == i));
        assert!(trace.vars.iter().any(|v| v == ACTION_TAKEN));
        let actions: Vec<_> = trace.states.iter().map(|s| s.action_taken()).collect();
        assert_eq!(
            actions,
            [
                Some("init"),
                Some("startServer"),
                Some("request"),
                Some("request"),
                Some("stopServer"),
                Some("startServer"),
            ]
        );
    }

    #[test]
    fn should_decode_variants_bigints_records_and_lists_when_reading_the_sample() {
        // given
        let trace = sample();

        // when
        let serving = &trace.states[1];

        // then
        assert_eq!(
            serving.var("state"),
            Some(&Value::Variant {
                tag: "Serving".into(),
                value: Box::new(Value::Bool(false)),
            })
        );
        let res = serving.var("res").expect("res");
        assert_eq!(res.field("listeners"), Some(&Value::Int(1)));
        assert_eq!(res.field("watchers"), Some(&Value::Int(0)));
        let inputs = serving
            .var("inputs")
            .and_then(Value::as_list)
            .expect("a list");
        let tags: Vec<_> = inputs.iter().filter_map(Value::tag).collect();
        assert_eq!(tags, ["InStart", "InBound", "InWatcherFailed"]);
        assert_eq!(
            serving.var("response").and_then(Value::tag),
            Some("NoRequest")
        );
        assert_eq!(
            serving
                .var("before")
                .and_then(|b| b.field("state"))
                .map(ToString::to_string),
            Some("Stopped".to_owned())
        );
    }

    #[test]
    fn should_unwrap_picked_params_and_drop_unpicked_ones_when_reading_nondet_picks() {
        // given
        let trace = sample();

        // when
        let start = trace.states[1].nondet_picks().expect("picks");
        let failed_start = trace.states[5].nondet_picks().expect("picks");
        let request = trace.states[2].nondet_picks().expect("picks");

        // then
        assert_eq!(start.get("bindOk"), Some(&Value::Bool(true)));
        assert_eq!(start.get("watcherOk"), Some(&Value::Bool(false)));
        assert_eq!(failed_start.get("bindOk"), Some(&Value::Bool(false)));
        assert!(request.is_empty());
    }

    #[test]
    fn should_omit_mbt_variables_when_listing_model_vars() {
        // given
        let trace = sample();

        // when
        let vars = trace.states[0].model_vars();

        // then
        let names: Vec<_> = vars.keys().copied().collect();
        assert_eq!(
            names,
            ["before", "effects", "inputs", "res", "response", "state"]
        );
    }

    #[test]
    fn should_decode_sets_and_maps_in_value_order_when_given_any_order() {
        // given
        let set = r##"{"#set": [{"#bigint": "3"}, 1, 2]}"##;
        let map = r##"{"#map": [["b", {"#tup": [1, true]}], ["a", {"#set": []}]]}"##;

        // when
        let set = value(set);
        let map = value(map);

        // then
        assert_eq!(
            set,
            Ok(Value::Set([1, 2, 3].into_iter().map(Value::Int).collect()))
        );
        let map = map.expect("a map");
        assert_eq!(map.to_string(), r#"Map("a" -> Set(), "b" -> (1, true))"#);
    }

    #[test]
    fn should_decode_big_and_negative_integers_when_given_bigint() {
        // given
        let big = r##"{"#bigint": "-170141183460469231731687303715884105728"}"##;

        // when
        let decoded = value(big);

        // then
        assert_eq!(decoded, Ok(Value::Int(i128::MIN)));
    }

    #[test]
    fn should_read_a_quint_option_when_given_some_or_none() {
        // given
        let some = value(r#"{"tag": "Some", "value": 4}"#).expect("variant");
        let none = value(r##"{"tag": "None", "value": {"#tup": []}}"##).expect("variant");
        let other = value(r#"{"tag": "Other", "value": 4}"#).expect("variant");

        // when
        let read = (some.as_option(), none.as_option(), other.as_option());

        // then
        assert_eq!(read, (Some(Some(&Value::Int(4))), Some(None), None));
    }

    #[test]
    fn should_refuse_with_a_path_when_given_unsupported_or_malformed_values() {
        // given
        let cases = [
            (r##"{"#unserializable": "x"}"##, "v.#unserializable"),
            (r##"{"#bigint": "1e3"}"##, "v.#bigint"),
            (r##"{"#bigint": "1", "x": 2}"##, "v"),
            (r##"{"#set": [1, 1]}"##, "v.#set[1]"),
            (r##"{"#map": [[1, 2, 3]]}"##, "v.#map[0]"),
            (r##"{"#map": [[1, 2], [1, 3]]}"##, "v.#map[1]"),
            (r##"{"#tup": 1}"##, "v.#tup"),
            ("1.5", "v"),
            ("null", "v"),
            (r#"{"a": [true, null]}"#, "v.a[1]"),
        ];

        // when
        let errors: Vec<_> = cases.iter().map(|(text, _)| value(text)).collect();

        // then
        for ((text, path), error) in cases.iter().zip(errors) {
            let error = error.expect_err(text);
            assert_eq!(&error.path, path, "{text}: {error}");
        }
    }

    #[test]
    fn should_refuse_when_the_document_has_no_states() {
        // given
        let text = r#"{"vars": ["x"]}"#;

        // when
        let decoded = Trace::from_json_str(text, "t");

        // then
        assert_eq!(decoded.expect_err("no states").path, "states");
    }

    #[test]
    fn should_name_the_trace_by_its_file_when_loaded_from_disk() {
        // given
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../formal/traces/regression/itf-decoder-sample.itf.json"
        ));

        // when
        let trace = Trace::load(path).map(|t| t.with_seed("11"));

        // then
        let trace = trace.expect("loads");
        assert_eq!(trace.id, "itf-decoder-sample.itf.json");
        assert_eq!(trace.seed.as_deref(), Some("11"));
        assert_eq!(trace.states, sample().states);
    }
}
