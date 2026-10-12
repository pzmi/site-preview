//! The reload driver (FORMAL-07, ADR 0057): replays `formal/reload.qnt`'s `step` actions
//! against the real [`ReloadState`] and the real event-stream handler, in process: the
//! adapter's router answers `GET /__site_preview/events` through `tower::ServiceExt::oneshot`
//! on a current-thread runtime with a paused clock (ADR 0017, D16), and each model browser's
//! connection is the real SSE response body.
//!
//! | Model action | Stimulus |
//! |:--|:--|
//! | `init` | a new [`ReloadState`] for a new instance; no connection |
//! | `publish(kinds, reason)`, `publishHot(kinds)` | [`ReloadState::publish_next`] with a [`ChangeSet`] accumulated from one `Modify` per kind (`a.css`, `a.js`, `a.html`, `a.png`, `a.txt`) and, for `reason`, an `Ambiguous` event on the root (a rescan) |
//! | `stopServer` | fire the [`Shutdown`], then poll every open body once: an ended body is a closed connection |
//! | `startServer` | a new [`ReloadState`], [`Shutdown`] and [`StreamLimit`] for a new instance id |
//! | `connect(b)` | `GET /__site_preview/events` through the router; the response must be `200`, and its first event is polled at once |
//! | `send(b)` | poll `b`'s body once, without waiting: the event it yields, if any |
//! | `disconnect(b)`, `userReload(b)` | drop `b`'s body |
//! | `receive(b, applied)` | the client's (D16): drop `b`'s body when the model's client reloads (its page is `Navigating` after the step), else nothing |
//! | `navigate(b)` | the client's: nothing reaches the server (a navigating page has no stream) |
//!
//! A body is polled only at the model's `connect`, `send` and `stopServer`, never between
//! them, so publishes in between coalesce in the real `Feed` exactly where the model's `send`
//! coalesces them. The clock is paused and never advanced (no step awaits a timer), so no
//! keepalive comment is due; one that arrived would be skipped.
//!
//! The observation ([`ReloadObs`]) is read from the real code after every step:
//!
//! - `up`: whether the driver's server runs (between a `startServer` and a `stopServer`);
//!   in process there is no listener to probe, so this one is the driver's own record.
//! - `committed`: [`ReloadState::current`]; `latest`: the kinds and [`ChangeSet::is_full`] of
//!   the value a new subscription reads ([`preview_application::StateSubscription::initial`]).
//! - `streams`: per open connection, every event its body yielded, parsed from the wire
//!   (`event:`, `id:` and the JSON `data:`): name, instance, revision, the set of the
//!   `changes` kinds and `full`. A connection whose body ended or was dropped is absent.
//!
//! Instances are compared by their order of creation: the real ids are opaque strings, the
//! model numbers them 1, 2, ... (an id the driver never created observes as instance 0).
//! Left out, as the browser's (D16) or ghost state: `doc`, `baseline`, `delivered`,
//! `hotApplied`, `reloaded`, `trigger`, `history`, `taken` and `before`. `lastSent` is the
//! last event of `stream` (the model's `monotonicRevision`), so it is not observed apart.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use axum::Router;
use axum::body::{Body, HttpBody};
use axum::http::{Request, StatusCode};
use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Injection, Shutdown, StreamLimit,
    build_router,
};
use preview_application::ReloadState;
use preview_domain::{
    AssetKind, BoundAddr, ChangeEvent, ChangeKind, ChangeSet, Component, HostPolicy, RelPath,
    SiteState,
};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tokio::runtime::{Builder, Runtime};
use tower::ServiceExt;

use crate::itf::{State, Value};
use crate::replay::{Driver, DriverError, Params, ProjectionError};

/// The event stream's path (plan.md section 7.2).
const EVENTS: &str = "/__site_preview/events";

/// `AssetKind`, as the model's `Kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// `css`.
    Css,
    /// `js`.
    Js,
    /// `html`.
    Html,
    /// `asset`.
    Asset,
    /// `other`.
    Other,
}

impl Kind {
    /// The model's tag.
    fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "Css" => Self::Css,
            "Js" => Self::Js,
            "Html" => Self::Html,
            "Asset" => Self::Asset,
            "Other" => Self::Other,
            _ => return None,
        })
    }

    /// The wire code (`kind_code` in the SSE handler).
    fn from_wire(code: &str) -> Option<Self> {
        Some(match code {
            "css" => Self::Css,
            "js" => Self::Js,
            "html" => Self::Html,
            "asset" => Self::Asset,
            "other" => Self::Other,
            _ => return None,
        })
    }

    const fn of(kind: AssetKind) -> Self {
        match kind {
            AssetKind::Css => Self::Css,
            AssetKind::Js => Self::Js,
            AssetKind::Html => Self::Html,
            AssetKind::Asset => Self::Asset,
            AssetKind::Other => Self::Other,
        }
    }

    /// A file name of this kind, for a synthetic change event.
    const fn file(self) -> &'static str {
        match self {
            Self::Css => "a.css",
            Self::Js => "a.js",
            Self::Html => "a.html",
            Self::Asset => "a.png",
            Self::Other => "a.txt",
        }
    }
}

/// A `SiteState`: the instance by creation order (1, 2, ...) and the revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// The instance's position in creation order, from 1; 0 for an id never created.
    pub instance: u64,
    /// The revision.
    pub revision: u64,
}

/// A published change set: its kinds and whether it is full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The kinds of its paths.
    pub kinds: BTreeSet<Kind>,
    /// [`ChangeSet::is_full`].
    pub full: bool,
}

/// The name of an SSE event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventName {
    /// `event: state`, the baseline.
    State,
    /// `event: reload`.
    Reload,
    /// A frame the driver could not read as either, kept whole for the report.
    Malformed(String),
}

/// One event a connection received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Its name.
    pub name: EventName,
    /// The state it announces.
    pub at: Stamp,
    /// The kinds of its `changes` (none for a `state` event).
    pub kinds: BTreeSet<Kind>,
    /// Its `full` (false for a `state` event, which has none).
    pub full: bool,
}

/// The canonical reload observation: what both the model and the real code project to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReloadObs {
    /// A server runs.
    pub up: bool,
    /// The newest committed state.
    pub committed: Stamp,
    /// The change set published with it.
    pub latest: Change,
    /// Per open connection, by browser, the events it received in order.
    pub streams: BTreeMap<String, Vec<Event>>,
}

/// Builds the router for one server instance: the real [`build_router`], or a test's faulty
/// stand-in behind the same seam (FORMAL-08).
pub type MakeRouter = fn(AppState) -> Router;

/// One browser's open connection.
struct Connection {
    body: Body,
    received: Vec<Event>,
}

/// What polling a body once gave.
enum Polled {
    /// An event frame, as text.
    Event(String),
    /// Nothing is ready.
    Pending,
    /// The stream ended (or failed).
    Ended,
}

/// Drives the real [`ReloadState`] and SSE handler from the reload model's `step` actions.
pub struct ReloadDriver {
    runtime: Runtime,
    make_router: MakeRouter,
    ids: FixedInstanceIds,
    /// The real instance ids, in creation order.
    instances: Vec<String>,
    reload: ReloadState,
    shutdown: Shutdown,
    streams: StreamLimit,
    up: bool,
    connections: BTreeMap<String, Connection>,
}

impl fmt::Debug for ReloadDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReloadDriver")
            .field("instances", &self.instances)
            .field("up", &self.up)
            .field("connections", &self.connections.keys())
            .finish_non_exhaustive()
    }
}

impl ReloadDriver {
    /// A driver over the real router.
    ///
    /// # Panics
    ///
    /// When the current-thread runtime cannot be built.
    #[must_use]
    pub fn new() -> Self {
        Self::with_router(build_router)
    }

    /// A driver over the router `make_router` builds for each instance.
    ///
    /// # Panics
    ///
    /// When the current-thread runtime cannot be built.
    #[must_use]
    pub fn with_router(make_router: MakeRouter) -> Self {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("a current-thread runtime with a paused clock");
        let mut ids = fresh_ids();
        let first = ids.next_id();
        Self {
            runtime,
            make_router,
            ids,
            instances: vec![first.as_str().to_owned()],
            reload: ReloadState::new(first),
            shutdown: Shutdown::new(),
            streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
            up: true,
            connections: BTreeMap::new(),
        }
    }

    /// The model's `init`: one running instance at revision 0, no connection.
    fn reset(&mut self) {
        self.connections.clear();
        self.ids = fresh_ids();
        self.instances.clear();
        self.start();
    }

    /// A new process: a new instance id, channel, shutdown switch and stream limit.
    fn start(&mut self) {
        let id = self.ids.next_id();
        self.instances.push(id.as_str().to_owned());
        self.reload = ReloadState::new(id);
        self.shutdown = Shutdown::new();
        self.streams = StreamLimit::new(DEFAULT_MAX_STREAMS);
        self.up = true;
    }

    /// Fires the shutdown switch, then polls every open body once: a body that ends is a
    /// closed connection; one that yields an event or nothing stays open, as observed.
    fn stop(&mut self) {
        self.shutdown.trigger();
        let names: Vec<String> = self.connections.keys().cloned().collect();
        for name in names {
            self.poll_into(&name);
        }
        self.up = false;
    }

    fn publish(&self, kinds: &BTreeSet<Kind>, reason: bool) -> Result<(), DriverError> {
        let mut changes = ChangeSet::new();
        for kind in kinds {
            let name = Component::new(kind.file())
                .map_err(|e| DriverError::Refused(format!("{}: {e:?}", kind.file())))?;
            changes.accumulate(&ChangeEvent {
                kind: ChangeKind::Modify,
                path: RelPath::new(vec![name]),
            });
        }
        if reason {
            // Unattributable: a rescan, which marks the set full and records no path.
            changes.accumulate(&ChangeEvent {
                kind: ChangeKind::Ambiguous,
                path: RelPath::root(),
            });
        }
        self.reload
            .publish_next(changes)
            .map(|_| ())
            .map_err(|e| DriverError::Refused(format!("publish_next: {e}")))
    }

    fn app_state(&self) -> AppState {
        AppState {
            content: Arc::new(InMemorySite::new()),
            reload: self.reload.clone(),
            host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
            shutdown: self.shutdown.signal(),
            keep_alive: DEFAULT_KEEP_ALIVE,
            streams: self.streams.clone(),
            injection: Injection::default(),
        }
    }

    fn connect(&mut self, browser: &str) -> Result<(), DriverError> {
        // The model connects only a closed browser; a leftover body is dropped first.
        self.connections.remove(browser);
        let request = Request::builder()
            .uri(EVENTS)
            .header("host", "127.0.0.1")
            .body(Body::empty())
            .map_err(|e| DriverError::Refused(format!("request: {e}")))?;
        let router = (self.make_router)(self.app_state());
        let response = self
            .runtime
            .block_on(router.oneshot(request))
            .unwrap_or_else(|never| match never {});
        if response.status() != StatusCode::OK {
            return Err(DriverError::Refused(format!(
                "{EVENTS} answered {}",
                response.status()
            )));
        }
        self.connections.insert(
            browser.to_owned(),
            Connection {
                body: response.into_body(),
                received: Vec::new(),
            },
        );
        self.poll_into(browser);
        Ok(())
    }

    /// Polls `browser`'s body once and records what it gave: an event is appended, an ended
    /// body closes the connection, nothing ready changes nothing.
    fn poll_into(&mut self, browser: &str) {
        let Some(connection) = self.connections.get_mut(browser) else {
            return;
        };
        match poll_once(&self.runtime, &mut connection.body) {
            Polled::Event(text) => {
                let event = parse(&text, &self.instances);
                connection.received.push(event);
            }
            Polled::Pending => {}
            Polled::Ended => {
                self.connections.remove(browser);
            }
        }
    }

    fn send(&mut self, browser: &str) -> Result<(), DriverError> {
        if !self.connections.contains_key(browser) {
            return Err(DriverError::Refused(format!(
                "{browser} has no open stream"
            )));
        }
        self.poll_into(browser);
        Ok(())
    }

    /// The instance's position in creation order, from 1; 0 for an unknown id.
    fn stamp(&self, state: &SiteState) -> Stamp {
        Stamp {
            instance: ordinal(&self.instances, state.instance_id().as_str()),
            revision: state.revision().get(),
        }
    }
}

impl Default for ReloadDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for ReloadDriver {
    type Obs = ReloadObs;

    fn apply(&mut self, action: &str, params: &Params) -> Result<(), DriverError> {
        match action {
            "init" => self.reset(),
            "publish" | "publishHot" => {
                let reason = match params.get("reason") {
                    None => false,
                    Some(_) => params.bool("reason")?,
                };
                self.publish(&kinds(params)?, reason)?;
            }
            "stopServer" => self.stop(),
            "startServer" => self.start(),
            "connect" => self.connect(params.str("b")?)?,
            "send" => self.send(params.str("b")?)?,
            "disconnect" | "userReload" => {
                self.connections.remove(params.str("b")?);
            }
            "receive" => {
                params.str("b")?;
                params.bool("applied")?;
            }
            "navigate" => {
                params.str("b")?;
            }
            other => return Err(DriverError::UnknownAction(other.to_owned())),
        }
        Ok(())
    }

    fn apply_with_model(
        &mut self,
        action: &str,
        params: &Params,
        model: &State,
    ) -> Result<(), DriverError> {
        self.apply(action, params)?;
        if action == "receive" {
            // The client's decision (D16): a reload navigates away and closes the stream.
            let browser = params.str("b")?;
            if client_navigated(model, browser)? {
                self.connections.remove(browser);
            }
        }
        Ok(())
    }

    fn observe(&self) -> ReloadObs {
        let published = self.reload.subscribe().initial().clone();
        ReloadObs {
            up: self.up,
            committed: self.stamp(&published.state),
            latest: Change {
                kinds: published
                    .changes
                    .changes()
                    .iter()
                    .map(|change| Kind::of(change.kind()))
                    .collect(),
                full: published.changes.is_full(),
            },
            streams: self
                .connections
                .iter()
                .map(|(name, connection)| (name.clone(), connection.received.clone()))
                .collect(),
        }
    }
}

/// The instance ids of one replay, the same in every run.
fn fresh_ids() -> FixedInstanceIds {
    FixedInstanceIds::new(7, 1)
}

fn ordinal(instances: &[String], id: &str) -> u64 {
    instances
        .iter()
        .position(|known| known == id)
        .and_then(|i| u64::try_from(i + 1).ok())
        .unwrap_or(0)
}

/// The picked `kinds`, a set of the model's `Kind`.
fn kinds(params: &Params) -> Result<BTreeSet<Kind>, DriverError> {
    let value = params.value("kinds")?;
    let bad = || DriverError::bad_param("kinds", "a set of kinds", value);
    let Value::Set(set) = value else {
        return Err(bad());
    };
    set.iter()
        .map(|kind| kind.tag().and_then(Kind::from_tag).ok_or_else(bad))
        .collect()
}

/// Whether the model's client for `browser` is navigating after the step: it reloaded.
fn client_navigated(model: &State, browser: &str) -> Result<bool, DriverError> {
    let doc = model
        .var("browsers")
        .and_then(|browsers| map_get(browsers, browser))
        .and_then(|b| b.field("doc"))
        .and_then(Value::tag)
        .ok_or_else(|| DriverError::Refused(format!("the model has no browsers[{browser}].doc")))?;
    Ok(doc == "Navigating")
}

fn map_get<'v>(map: &'v Value, key: &str) -> Option<&'v Value> {
    match map {
        Value::Map(entries) => entries.get(&Value::Str(key.to_owned())),
        _ => None,
    }
}

/// Polls `body` once inside `runtime`, without waiting, skipping keepalive comments.
fn poll_once(runtime: &Runtime, body: &mut Body) -> Polled {
    runtime.block_on(std::future::poll_fn(|cx| {
        loop {
            let polled = match Pin::new(&mut *body).poll_frame(cx) {
                Poll::Pending => Polled::Pending,
                Poll::Ready(None | Some(Err(_))) => Polled::Ended,
                Poll::Ready(Some(Ok(frame))) => {
                    let Ok(data) = frame.into_data() else {
                        continue;
                    };
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if text
                        .lines()
                        .all(|line| line.is_empty() || line.starts_with(':'))
                    {
                        continue;
                    }
                    Polled::Event(text)
                }
            };
            return Poll::Ready(polled);
        }
    }))
}

/// Reads one SSE frame: `event:`, `id:` and a JSON `data:` with `instance_id`, `revision` and,
/// for a `reload`, `changes` (each with a `kind`) and `full`. Anything else is
/// [`EventName::Malformed`], so the comparison fails with the frame in the report.
fn parse(text: &str, instances: &[String]) -> Event {
    let malformed = || Event {
        name: EventName::Malformed(text.to_owned()),
        at: Stamp {
            instance: 0,
            revision: 0,
        },
        kinds: BTreeSet::new(),
        full: false,
    };
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
    };
    let name = match field("event") {
        Some("state") => EventName::State,
        Some("reload") => EventName::Reload,
        _ => return malformed(),
    };
    let Some(data) = field("data").and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
    else {
        return malformed();
    };
    let (Some(instance), Some(revision)) = (
        data.get("instance_id").and_then(serde_json::Value::as_str),
        data.get("revision").and_then(serde_json::Value::as_u64),
    ) else {
        return malformed();
    };
    if field("id") != Some(revision.to_string().as_str()) {
        return malformed();
    }
    let mut kinds = BTreeSet::new();
    if let Some(changes) = data.get("changes") {
        let Some(changes) = changes.as_array() else {
            return malformed();
        };
        for change in changes {
            let Some(kind) = change
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .and_then(Kind::from_wire)
            else {
                return malformed();
            };
            kinds.insert(kind);
        }
    }
    let full = match data.get("full") {
        None => false,
        Some(full) => match full.as_bool() {
            Some(full) => full,
            None => return malformed(),
        },
    };
    Event {
        name,
        at: Stamp {
            instance: ordinal(instances, instance),
            revision,
        },
        kinds,
        full,
    }
}

/// Projects a state of `formal/reload.qnt` (or of a module importing it, such as
/// `reload_traces.qnt`) into the driver's observation.
///
/// # Errors
///
/// [`ProjectionError`] when a variable is absent or has a shape the model does not give it.
pub fn project(state: &State) -> Result<ReloadObs, ProjectionError> {
    let var = |name: &str| {
        state
            .var(name)
            .ok_or_else(|| ProjectionError::missing(name))
    };
    let latest = var("latest")?;
    let Value::Map(browsers) = var("browsers")? else {
        return Err(ProjectionError::missing("browsers"));
    };
    let mut streams = BTreeMap::new();
    for (name, browser) in browsers {
        let name = name
            .as_str()
            .ok_or_else(|| ProjectionError::missing("a browser's name"))?;
        let open = browser
            .field("open")
            .and_then(Value::as_bool)
            .ok_or_else(|| ProjectionError::missing("browsers.open"))?;
        if !open {
            continue;
        }
        let events = browser
            .field("stream")
            .and_then(Value::as_list)
            .ok_or_else(|| ProjectionError::missing("browsers.stream"))?
            .iter()
            .map(model_event)
            .collect::<Result<Vec<_>, _>>()?;
        streams.insert(name.to_owned(), events);
    }
    Ok(ReloadObs {
        up: var("up")?
            .as_bool()
            .ok_or_else(|| ProjectionError::missing("up"))?,
        committed: model_stamp(var("committed")?, "committed")?,
        latest: Change {
            kinds: model_kinds(latest.field("kinds"), "latest.kinds")?,
            full: latest
                .field("full")
                .and_then(Value::as_bool)
                .ok_or_else(|| ProjectionError::missing("latest.full"))?,
        },
        streams,
    })
}

fn model_stamp(value: &Value, what: &str) -> Result<Stamp, ProjectionError> {
    let int = |field: &str| {
        value
            .field(field)
            .and_then(Value::as_int)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| ProjectionError::missing(&format!("{what}.{field}")))
    };
    Ok(Stamp {
        instance: int("instance")?,
        revision: int("revision")?,
    })
}

fn model_kinds(value: Option<&Value>, what: &str) -> Result<BTreeSet<Kind>, ProjectionError> {
    let Some(Value::Set(set)) = value else {
        return Err(ProjectionError::missing(what));
    };
    set.iter()
        .map(|kind| {
            kind.tag()
                .and_then(Kind::from_tag)
                .ok_or_else(|| ProjectionError::missing(what))
        })
        .collect()
}

fn model_event(value: &Value) -> Result<Event, ProjectionError> {
    let name = match value.field("name").and_then(Value::tag) {
        Some("StateEvent") => EventName::State,
        Some("ReloadEvent") => EventName::Reload,
        _ => return Err(ProjectionError::missing("event.name")),
    };
    Ok(Event {
        name,
        at: model_stamp(
            value
                .field("at")
                .ok_or_else(|| ProjectionError::missing("event.at"))?,
            "event.at",
        )?,
        kinds: model_kinds(value.field("kinds"), "event.kinds")?,
        full: value
            .field("full")
            .and_then(Value::as_bool)
            .ok_or_else(|| ProjectionError::missing("event.full"))?,
    })
}
