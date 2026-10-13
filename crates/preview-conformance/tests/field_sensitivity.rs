//! Per-field sensitivity (FORMAL-10, ADR 0070): every field a driver projects is compared.
//!
//! Each test takes a committed regression trace that replays without divergence, changes one
//! model variable at one state, and asserts that the replay against the real code stops at
//! exactly that state with a [`Cause::Mismatch`], the model's side showing the change in that
//! one field and the real side the unchanged value. A driver or projection that dropped the
//! field (a constant `observe`, a projection that ignores a variable) would replay the edited
//! trace without divergence, so the test fails with it.
//!
//! The negative controls (`negative_controls.rs`) show the other side for the fields they
//! fault: a faulty implementation is caught there. The lifecycle's `stopped` is projected from
//! the step's action, not from a variable, so it has no model-side edit here; the
//! `StreamOutlivesShutdown` control pins it from the implementation's side.

use std::collections::BTreeSet;

use preview_conformance::drivers::lifecycle::{self, LifecycleDriver, LifecycleObs, Response};
use preview_conformance::drivers::reload::{self, Kind, ReloadDriver, ReloadObs, Stamp};
use preview_conformance::drivers::watcher::{self, Health, HealthEvent, WatcherDriver, WatcherObs};
use preview_conformance::generate::regression;
use preview_conformance::{Cause, Driver, ProjectionError, State, Trace, Value, replay};
use preview_domain::LifecycleState;
use tempfile::TempDir;

/// The value at `path` in `state` (a variable, then record fields and string map keys).
fn at<'s>(state: &'s mut State, path: &[&str]) -> &'s mut Value {
    let (var, rest) = path.split_first().expect("a path");
    let mut value = state
        .vars
        .get_mut(*var)
        .unwrap_or_else(|| panic!("no variable {var}"));
    for step in rest {
        value = match value {
            Value::Record(fields) => fields.get_mut(*step),
            Value::Map(entries) => entries.get_mut(&Value::Str((*step).to_owned())),
            Value::List(items) => step.parse::<usize>().ok().and_then(|i| items.get_mut(i)),
            _ => None,
        }
        .unwrap_or_else(|| panic!("no {step} in {path:?}"));
    }
    value
}

/// A variant without a payload.
fn tag(name: &str) -> Value {
    Value::Variant {
        tag: name.to_owned(),
        value: Box::new(Value::Tuple(Vec::new())),
    }
}

/// Edits state `step` of `trace` with `edit`, replays it against `driver` and asserts the
/// catch: that state, a mismatch, the edited projection on the model's side and the
/// unedited one on the real side. Returns both projections.
fn caught<D, P>(
    trace: &Trace,
    step: usize,
    edit: impl FnOnce(&mut State),
    driver: &mut D,
    project: P,
) -> (D::Obs, D::Obs)
where
    D: Driver,
    P: Fn(&State) -> Result<D::Obs, ProjectionError> + Copy,
{
    let original = project(&trace.states[step]).expect("the model state projects");
    let mut edited = trace.clone();
    edit(&mut edited.states[step]);
    let changed = project(&edited.states[step]).expect("the edited state projects");
    assert_ne!(original, changed, "the edit changes the projection");
    let divergence = replay(&edited, driver, project).expect_err("the edited field is compared");
    assert_eq!(
        (divergence.step, &divergence.cause),
        (step, &Cause::Mismatch),
        "{divergence}"
    );
    assert_eq!(divergence.expected, Some(format!("{changed:#?}")));
    assert_eq!(divergence.actual, Some(format!("{original:#?}")));
    (original, changed)
}

fn trace(name: &str) -> Trace {
    regression(name).unwrap_or_else(|err| panic!("{err}"))
}

// ------------------------------------------------------------------------------------------
// Lifecycle: `lifecycle-startServeStop` (start, request, open a stream, stop, request).
// ------------------------------------------------------------------------------------------

fn site() -> TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><title>Conformance</title><h1>Hello</h1>",
    )
    .expect("an index page");
    dir
}

/// Replays `lifecycle-startServeStop` with state `step` edited by `edit`.
fn lifecycle_caught(step: usize, edit: impl FnOnce(&mut State)) -> (LifecycleObs, LifecycleObs) {
    let root = site();
    let mut driver = LifecycleDriver::new(root.path());
    caught(
        &trace("lifecycle-startServeStop"),
        step,
        edit,
        &mut driver,
        lifecycle::project,
    )
}

#[test]
fn should_diverge_at_the_edited_step_when_lifecycle_state_differs() {
    // given: serving with the watcher, edited to serving without it
    let edit = |s: &mut State| {
        *at(s, &["state"]) = Value::Variant {
            tag: "Serving".to_owned(),
            value: Box::new(Value::Bool(false)),
        };
    };

    // when
    let (original, changed) = lifecycle_caught(1, edit);

    // then
    let state = LifecycleState::Serving { watcher: false };
    assert_eq!(changed, LifecycleObs { state, ..original });
}

#[test]
fn should_diverge_at_the_edited_step_when_listener_ownership_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["res", "listeners"]) = Value::Int(0);

    // when
    let (original, changed) = lifecycle_caught(1, edit);

    // then
    let listener_owned = false;
    assert_eq!(
        changed,
        LifecycleObs {
            listener_owned,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_tasks_alive_differs() {
    // given
    let edit = |s: &mut State| {
        *at(s, &["res", "serveTasks"]) = Value::Int(0);
        *at(s, &["res", "debounceTasks"]) = Value::Int(0);
    };

    // when
    let (original, changed) = lifecycle_caught(1, edit);

    // then
    let tasks_alive = false;
    assert_eq!(
        changed,
        LifecycleObs {
            tasks_alive,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_open_streams_differ() {
    // given: one stream open, edited to two
    let edit = |s: &mut State| *at(s, &["res", "streams"]) = Value::Int(2);

    // when
    let (original, changed) = lifecycle_caught(3, edit);

    // then
    let streams_open = 2;
    assert_eq!(
        changed,
        LifecycleObs {
            streams_open,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_response_differs() {
    // given: the request answered, edited to unanswered
    let edit = |s: &mut State| *at(s, &["response"]) = tag("Unanswered");

    // when
    let (original, changed) = lifecycle_caught(2, edit);

    // then
    let response = Response::Unanswered;
    assert_eq!(
        changed,
        LifecycleObs {
            response,
            ..original
        }
    );
}

// ------------------------------------------------------------------------------------------
// Reload: `reload-cssOnlyHot` (navigate, connect, receive, publish css, send, receive).
// ------------------------------------------------------------------------------------------

/// Replays `reload-cssOnlyHot` with state `step` edited by `edit`.
fn reload_caught(step: usize, edit: impl FnOnce(&mut State)) -> (ReloadObs, ReloadObs) {
    let mut driver = ReloadDriver::new();
    caught(
        &trace("reload-cssOnlyHot"),
        step,
        edit,
        &mut driver,
        reload::project,
    )
}

#[test]
fn should_diverge_at_the_edited_step_when_server_up_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["up"]) = Value::Bool(false);

    // when
    let (original, changed) = reload_caught(1, edit);

    // then
    assert_eq!(
        changed,
        ReloadObs {
            up: false,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_committed_state_differs() {
    // given: (1, 1) edited to (2, 2)
    let edit = |s: &mut State| {
        *at(s, &["committed", "instance"]) = Value::Int(2);
        *at(s, &["committed", "revision"]) = Value::Int(2);
    };

    // when
    let (original, changed) = reload_caught(4, edit);

    // then
    let committed = Stamp {
        instance: 2,
        revision: 2,
    };
    assert_eq!(
        changed,
        ReloadObs {
            committed,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_latest_kinds_differ() {
    // given: a css change set, edited to a js one
    let edit = |s: &mut State| *at(s, &["latest", "kinds"]) = Value::Set([tag("Js")].into());

    // when
    let (original, mut changed) = reload_caught(4, edit);

    // then
    assert_eq!(changed.latest.kinds, BTreeSet::from([Kind::Js]));
    changed.latest.kinds = original.latest.kinds.clone();
    assert_eq!(changed, original);
}

#[test]
fn should_diverge_at_the_edited_step_when_latest_fullness_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["latest", "full"]) = Value::Bool(true);

    // when
    let (original, mut changed) = reload_caught(4, edit);

    // then
    assert!(changed.latest.full);
    changed.latest.full = false;
    assert_eq!(changed, original);
}

#[test]
fn should_diverge_at_the_edited_step_when_a_stream_is_closed_in_the_model() {
    // given: b1's stream open after connect, edited to closed
    let edit = |s: &mut State| *at(s, &["browsers", "b1", "open"]) = Value::Bool(false);

    // when
    let (original, changed) = reload_caught(2, edit);

    // then
    let mut streams = original.streams.clone();
    streams.remove("b1").expect("b1 is open");
    assert_eq!(
        changed,
        ReloadObs {
            streams,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_a_sent_event_differs() {
    // given: the hot css reload b1 received, edited to a full one
    let edit =
        |s: &mut State| *at(s, &["browsers", "b1", "stream", "1", "full"]) = Value::Bool(true);

    // when
    let (original, mut changed) = reload_caught(5, edit);

    // then
    let event = changed
        .streams
        .get_mut("b1")
        .and_then(|events| events.get_mut(1))
        .expect("b1 received the reload");
    assert!(event.full);
    event.full = false;
    assert_eq!(changed, original);
}

// ------------------------------------------------------------------------------------------
// Watcher: `watcher-burstOneReload` (modifies, a duplicate, receives, ticks, a settle).
// ------------------------------------------------------------------------------------------

/// Replays `watcher-burstOneReload` with state `step` edited by `edit`.
fn watcher_caught(step: usize, edit: impl FnOnce(&mut State)) -> (WatcherObs, WatcherObs) {
    let trace = trace("watcher-burstOneReload");
    let mut driver = WatcherDriver::new(&trace).expect("the environment decodes");
    caught(&trace, step, edit, &mut driver, watcher::project)
}

#[test]
fn should_diverge_at_the_edited_step_when_revision_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["revision"]) = Value::Int(2);

    // when
    let (original, changed) = watcher_caught(10, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            revision: 2,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_dirty_differs() {
    // given: a dirty debouncer edited to a clean one, whose deadline is not projected
    let edit = |s: &mut State| *at(s, &["deb", "dirty"]) = Value::Bool(false);

    // when
    let (original, changed) = watcher_caught(4, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            dirty: false,
            deadline_ms: None,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_deadline_differs() {
    // given: due at tick 2, edited to tick 3
    let edit = |s: &mut State| *at(s, &["deb", "deadline"]) = Value::Int(3);

    // when
    let (original, changed) = watcher_caught(4, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            deadline_ms: Some(3 * watcher::TICK_MS),
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_health_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["health"]) = tag("HStarting");

    // when
    let (original, changed) = watcher_caught(0, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            health: Health::Starting,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_recorded_count_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["last", "recorded"]) = Value::Int(2);

    // when
    let (original, changed) = watcher_caught(4, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            recorded: 2,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_reconcile_request_differs() {
    // given
    let edit = |s: &mut State| *at(s, &["last", "needsReconcile"]) = Value::Bool(true);

    // when
    let (original, changed) = watcher_caught(4, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            needs_reconcile: true,
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_health_events_differ() {
    // given: no event, edited to a recovery
    let edit = |s: &mut State| *at(s, &["last", "events"]) = Value::List(vec![tag("EvRecovered")]);

    // when
    let (original, changed) = watcher_caught(4, edit);

    // then
    assert_eq!(
        changed,
        WatcherObs {
            events: vec![HealthEvent::Recovered],
            ..original
        }
    );
}

#[test]
fn should_diverge_at_the_edited_step_when_published_revision_differs() {
    for (field, value) in [
        ("fired", Value::Bool(false)),
        ("firedFull", Value::Bool(false)),
        ("firedRescan", Value::Bool(true)),
        ("firedPaths", Value::Set([tag("Css")].into())),
    ] {
        // given: the burst's full revision of /a.css and /index.html, edited in one field
        let edit = |s: &mut State| *at(s, &["last", field]) = value.clone();

        // when
        let (original, changed) = watcher_caught(10, edit);

        // then
        let published = original.published.clone().expect("the model publishes");
        let expected = match field {
            "fired" => None,
            "firedFull" => Some(watcher::Publish {
                full: false,
                ..published
            }),
            "firedRescan" => Some(watcher::Publish {
                rescan: true,
                ..published
            }),
            _ => Some(watcher::Publish {
                paths: ["/a.css".to_owned()].into(),
                ..published
            }),
        };
        assert_eq!(
            changed,
            WatcherObs {
                published: expected,
                ..original
            },
            "{field}"
        );
    }
}
