//! The watcher driver (FORMAL-06, ADR 0058, ADR 0070): replays `formal/watcher.qnt`'s `step`
//! actions against the real [`ChangeCoordinator`] (with its domain
//! [`Debouncer`](preview_domain::Debouncer), `classify`, `ChangeSet` and `WatcherHealth`)
//! publishing to a real [`ReloadState`], on a [`FakeClock`], through the pipeline's own
//! synchronous decision step, composition's [`WatchDecisions`]: the same code
//! `WatchPipeline` runs for each batch and each due deadline (hand it over, reconcile when the
//! coordinator asks, a full reconcile after a rescan revision unless one ran since the previous
//! one, and the reconcile's outcome as readiness or a failure). The pipeline's async shell (the
//! timer, the source's `next`, `block_in_place`, the warnings) is not driven here.
//!
//! The model's environment, the operating system and the bridge's channel (`fs` and `src`),
//! is not under test here: the driver reads it from the trace, step by step
//! ([`WatcherDriver::new`]), and never reads the model's debouncer, health, revision or `last`.
//!
//! | Model action | Stimulus |
//! |:--|:--|
//! | `init`, `initUnwatchable`, `initCapped` | a new coordinator and reload state at 0 ms; `watcher_ready(dirs)` when every directory is watched at start-up |
//! | `receive(k, capped)` | the batch the bridge's `next` delivers from the previous state's `src` (`Overflow`, `Failed`, or up to `k` changes with real names) to [`WatchDecisions::receive`] |
//! | `settle(capped)` | [`WatchDecisions::settle`] |
//! | `tick` | the [`FakeClock`] moves [`TICK_MS`] |
//! | `shutdown` | the driver stops applying anything but time |
//! | OS actions, `duplicate`, `reorder`, `kernelQueue`, `runtimeFailure`, `freeWatch` | nothing: they change only the environment, which the next `receive` reads |
//!
//! A reconcile the decisions run reports what the watcher adapter would (ADR 0050, `reconciled`
//! in composition's bootstrap), from the previous state's `fs` and `src` and the step's
//! `capped`: with the root gone a root removal (ADR 0067); `d` gone or watched, or registered
//! now, is readiness; `d` newly unwatchable is a problem; `d` still unwatchable, or a
//! registration past the relisting cap, is neither. A readiness the coordinator refuses is a
//! [`DriverError::Refused`]: the model never refuses one (the pipeline carries on past it, as
//! only revision exhaustion, outside the model, causes it).
//!
//! The observation ([`WatcherObs`]) is read from the real code after every step: the
//! revision from the [`ReloadState`], `dirty` and the deadline from
//! [`ChangeCoordinator::deadline`], the health, what the received batch recorded and whether
//! it asked for a reconcile, the health events the step returned, and the revision published
//! in the step as the [`ReloadState`] carries it (full or not, its paths) with the rescan flag
//! `settle` returned.

use std::collections::BTreeSet;

use preview_application::{ChangeBatch, ChangeCoordinator, ReloadState, SourceFailure};
pub use preview_composition::PipelineCoordinator;
use preview_composition::{ReconcileScope, Reconciled, StepReport, WatchDecisions, WatchReport};
use preview_domain::{
    ChangeEvent, ChangeKind, Component, Deadline, DebounceWindow, DegradeReason, DomainEvent,
    InstanceId, RelPath, WatcherHealth, WatcherHealthRefusal,
};
use preview_testkit::FakeClock;

use crate::itf::{State, Trace, Value};
use crate::replay::{Driver, DriverError, Params, ProjectionError};

/// One model tick in milliseconds: the model's window is 2 ticks, the debounce window 150 ms.
pub const TICK_MS: u64 = 75;

/// The coordinator interface the driver exercises: [`ChangeCoordinator`]'s commands the
/// pipeline's decisions issue ([`PipelineCoordinator`], composition's own seam), plus the
/// start-up report and the queries the driver observes. It is the seam a negative control
/// replaces with a deliberately faulty implementation (FORMAL-08); the driver uses nothing
/// else of the coordinator.
pub trait Coordinator: PipelineCoordinator {
    /// [`ChangeCoordinator::watcher_ready`].
    ///
    /// # Errors
    ///
    /// As [`ChangeCoordinator::watcher_ready`].
    fn watcher_ready(&mut self, dirs: usize) -> Result<Vec<DomainEvent>, WatcherHealthRefusal>;
    /// [`ChangeCoordinator::health`].
    fn health(&self) -> WatcherHealth;
    /// [`ChangeCoordinator::deadline`].
    fn deadline(&self) -> Option<Deadline>;
}

impl Coordinator for ChangeCoordinator {
    fn watcher_ready(&mut self, dirs: usize) -> Result<Vec<DomainEvent>, WatcherHealthRefusal> {
        Self::watcher_ready(self, dirs)
    }

    fn health(&self) -> WatcherHealth {
        Self::health(self)
    }

    fn deadline(&self) -> Option<Deadline> {
        Self::deadline(self)
    }
}

/// The watcher's health as the model names it (the directory count is left out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// `HStarting`: start-up has not reported readiness.
    Starting,
    /// `HWatching`: live reload works.
    Watching,
    /// `HDegraded(reason)`.
    Degraded(DegradeReason),
}

/// A health event a step returned (`HealthEvent` in the model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthEvent {
    /// [`DomainEvent::WatcherDegraded`].
    Degraded(DegradeReason),
    /// [`DomainEvent::WatcherRecovered`].
    Recovered,
}

/// The revision a step published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publish {
    /// The published change set is a full reload.
    pub full: bool,
    /// `settle` asked for a rescan.
    pub rescan: bool,
    /// The published change set's URL paths.
    pub paths: BTreeSet<String>,
}

/// The canonical watcher observation: what both the model and the real coordinator project to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatcherObs {
    /// The latest published revision.
    pub revision: u64,
    /// Changes are pending (the debouncer is dirty).
    pub dirty: bool,
    /// When they settle, in milliseconds since the start, while dirty.
    pub deadline_ms: Option<u64>,
    /// The watcher's health.
    pub health: Health,
    /// How many changes of the step's batch reached the debouncer.
    pub recorded: usize,
    /// The step's batch asked for a reconcile.
    pub needs_reconcile: bool,
    /// The health events the step returned, in order.
    pub events: Vec<HealthEvent>,
    /// The revision the step published, if any.
    pub published: Option<Publish>,
}

/// An abstract entry of the model's site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Index,
    Css,
    Css2,
    Sub,
    DirD,
    Temp,
    Git,
    Root,
}

impl Entry {
    fn decode(value: &Value) -> Result<Self, ProjectionError> {
        Ok(match value.tag() {
            Some("Index") => Self::Index,
            Some("Css") => Self::Css,
            Some("Css2") => Self::Css2,
            Some("Sub") => Self::Sub,
            Some("DirD") => Self::DirD,
            Some("Temp") => Self::Temp,
            Some("Git") => Self::Git,
            Some("Root") => Self::Root,
            _ => return Err(ProjectionError(format!("{value} is not a path"))),
        })
    }

    /// The real root-relative names: `index.html`, `a.css`, `c.css`, `d/b.js`, `d`,
    /// `.index.html.swp`, `.git/HEAD` and the root itself.
    const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Index => &["index.html"],
            Self::Css => &["a.css"],
            Self::Css2 => &["c.css"],
            Self::Sub => &["d", "b.js"],
            Self::DirD => &["d"],
            Self::Temp => &[".index.html.swp"],
            Self::Git => &[".git", "HEAD"],
            Self::Root => &[],
        }
    }

    fn rel_path(self) -> RelPath {
        RelPath::new(
            self.names()
                .iter()
                .filter_map(|name| Component::new(name).ok())
                .collect(),
        )
    }

    /// The URL path a published change set names it by.
    fn url_path(self) -> String {
        format!("/{}", self.names().join("/"))
    }
}

/// A model change kind as the adapter's [`ChangeKind`].
fn change_kind(value: &Value) -> Result<ChangeKind, ProjectionError> {
    Ok(match value.tag() {
        Some("KCreate") => ChangeKind::Create,
        Some("KModify") => ChangeKind::Modify,
        Some("KRemove") => ChangeKind::Remove,
        Some("KRename") => ChangeKind::Rename,
        Some("KDirChanged") => ChangeKind::DirectoryChanged,
        Some("KAmbiguous") => ChangeKind::Ambiguous,
        _ => return Err(ProjectionError(format!("{value} is not a change kind"))),
    })
}

fn reason(value: &Value) -> Result<DegradeReason, ProjectionError> {
    Ok(match value.tag() {
        Some("RegistrationFailed") => DegradeReason::RegistrationFailed,
        Some("RuntimeFailure") => DegradeReason::RuntimeFailure,
        Some("RootRemoved") => DegradeReason::RootRemoved,
        _ => return Err(ProjectionError(format!("{value} is not a failure reason"))),
    })
}

const fn source_failure(reason: DegradeReason) -> SourceFailure {
    match reason {
        DegradeReason::RuntimeFailure => SourceFailure::RuntimeFailure,
        DegradeReason::RootRemoved => SourceFailure::RootRemoved,
        // The model's failures are a registration or a runtime failure only.
        DegradeReason::RegistrationFailed | DegradeReason::RevisionExhausted => {
            SourceFailure::RegistrationFailed
        }
    }
}

/// One message in the bridge's channel.
#[derive(Clone, Debug)]
enum Msg {
    Event(ChangeEvent),
    Failed(DegradeReason),
}

/// The model's environment at one state: the bridge's channel and flags, the adapter's
/// watches, and the site on disk, as far as the pipeline's stimuli need them.
#[derive(Clone, Debug)]
struct Env {
    queue: Vec<Msg>,
    overflow: bool,
    failure: bool,
    watched: BTreeSet<Entry>,
    outstanding: BTreeSet<Entry>,
    d_present: bool,
    d_unwatchable: bool,
    root_present: bool,
}

impl Env {
    fn decode(state: &State) -> Result<Self, ProjectionError> {
        let var = |name: &str| {
            state
                .var(name)
                .ok_or_else(|| ProjectionError::missing(name))
        };
        let field = |var: &Value, name: &str, what: &str| {
            var.field(name)
                .cloned()
                .ok_or_else(|| ProjectionError::missing(&format!("{what}.{name}")))
        };
        let flag = |var: &Value, name: &str, what: &str| {
            field(var, name, what)?
                .as_bool()
                .ok_or_else(|| ProjectionError::missing(&format!("{what}.{name}")))
        };
        let src = var("src")?;
        let fs = var("fs")?;
        let queue = field(src, "queue", "src")?
            .as_list()
            .ok_or_else(|| ProjectionError::missing("src.queue"))?
            .iter()
            .map(Self::message)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            queue,
            overflow: flag(src, "overflow", "src")?,
            failure: flag(src, "failure", "src")?,
            watched: entries(&field(src, "watched", "src")?)?,
            outstanding: entries(&field(src, "outstanding", "src")?)?,
            d_present: flag(fs, "dPresent", "fs")?,
            d_unwatchable: flag(fs, "dUnwatchable", "fs")?,
            root_present: flag(fs, "rootPresent", "fs")?,
        })
    }

    fn message(value: &Value) -> Result<Msg, ProjectionError> {
        match value.as_variant() {
            Some(("MEvent", notif)) => Ok(Msg::Event(notification(notif)?)),
            Some(("MFailed", why)) => Ok(Msg::Failed(reason(why)?)),
            _ => Err(ProjectionError(format!("{value} is not a channel message"))),
        }
    }

    /// The bridge's `next` (`nextBatch` in the model): the start-up registration failure (the
    /// head of the queue), then the overflow flag, then the failure flag, then a queued
    /// failure alone or up to `k` consecutive queued changes as one batch.
    fn next_batch(&self, k: usize) -> Option<ChangeBatch<ChangeEvent>> {
        if let Some(Msg::Failed(DegradeReason::RegistrationFailed)) = self.queue.first() {
            return Some(ChangeBatch::Failed(SourceFailure::RegistrationFailed));
        }
        if self.overflow {
            return Some(ChangeBatch::Overflow);
        }
        if self.failure {
            return Some(ChangeBatch::Failed(SourceFailure::RuntimeFailure));
        }
        match self.queue.first()? {
            Msg::Failed(why) => Some(ChangeBatch::Failed(source_failure(*why))),
            Msg::Event(_) => Some(ChangeBatch::Events(
                self.queue
                    .iter()
                    .map_while(|m| match m {
                        Msg::Event(e) => Some(e.clone()),
                        Msg::Failed(_) => None,
                    })
                    .take(k)
                    .collect(),
            )),
        }
    }

    /// What the adapter's reconcile reports from this state (`reconcile` in the model), as
    /// composition's [`Reconciled`]: the root gone is a root removal (ADR 0067); `d` gone or
    /// watched, or registered now, is readiness with every watched directory; `d` newly
    /// unwatchable is a problem; `d` still unwatchable, or a registration past the relisting
    /// cap, is neither.
    fn reconcile(&self, capped: bool) -> Reconciled {
        let d = Entry::DirD;
        let without_d = self.watched.iter().filter(|e| **e != d).count();
        let ready = |dirs| Reconciled {
            ready: Some(dirs),
            ..Reconciled::default()
        };
        if !self.root_present {
            Reconciled {
                root_removed: true,
                ..Reconciled::default()
            }
        } else if !self.d_present {
            ready(without_d)
        } else if self.watched.contains(&d) {
            ready(self.watched.len())
        } else if self.d_unwatchable {
            if self.outstanding.contains(&d) {
                Reconciled::default()
            } else {
                Reconciled {
                    problems: vec!["d".to_owned()],
                    ..Reconciled::default()
                }
            }
        } else if capped {
            Reconciled::default()
        } else {
            ready(without_d + 1)
        }
    }
}

fn entries(value: &Value) -> Result<BTreeSet<Entry>, ProjectionError> {
    let Value::Set(items) = value else {
        return Err(ProjectionError(format!("{value} is not a set of paths")));
    };
    items.iter().map(Entry::decode).collect()
}

fn notification(value: &Value) -> Result<ChangeEvent, ProjectionError> {
    let part = |name: &str| {
        value
            .field(name)
            .ok_or_else(|| ProjectionError::missing(&format!("notification.{name}")))
    };
    Ok(ChangeEvent {
        kind: change_kind(part("kind")?)?,
        path: Entry::decode(part("path")?)?.rel_path(),
    })
}

/// What the step did, beyond the coordinator's own state.
#[derive(Debug, Default)]
struct StepOutput {
    recorded: usize,
    needs_reconcile: bool,
    events: Vec<HealthEvent>,
    revision_before: u64,
    rescan: Option<bool>,
}

/// Drives a [`Coordinator`] (the real [`ChangeCoordinator`] unless a negative control swaps
/// it) from the watcher model's actions, through composition's [`WatchDecisions`].
pub struct WatcherDriver<C: Coordinator = ChangeCoordinator> {
    /// The environment at each state of the trace, by index.
    script: Vec<Env>,
    /// How many steps were applied: the index of the next state.
    applied: usize,
    make: fn(DebounceWindow, ReloadState) -> C,
    clock: FakeClock,
    reload: ReloadState,
    /// The pipeline's decisions over the coordinator.
    decisions: WatchDecisions<C>,
    cancelled: bool,
    step: StepOutput,
}

impl WatcherDriver {
    /// A driver over the real [`ChangeCoordinator`] for `trace`, whose environment it reads.
    ///
    /// # Errors
    ///
    /// [`ProjectionError`] when a state's `fs` or `src` does not have the model's shape.
    pub fn new(trace: &Trace) -> Result<Self, ProjectionError> {
        Self::with_coordinator(trace, ChangeCoordinator::new)
    }
}

impl<C: Coordinator> WatcherDriver<C> {
    /// A driver whose coordinator `make` builds, for each `init`, from the debounce window and
    /// the reload state the driver observes.
    ///
    /// # Errors
    ///
    /// As [`WatcherDriver::new`].
    pub fn with_coordinator(
        trace: &Trace,
        make: fn(DebounceWindow, ReloadState) -> C,
    ) -> Result<Self, ProjectionError> {
        let script = trace
            .states
            .iter()
            .map(Env::decode)
            .collect::<Result<_, _>>()?;
        let reload = new_reload_state();
        Ok(Self {
            script,
            applied: 0,
            make,
            clock: FakeClock::new(),
            decisions: WatchDecisions::new(make(window(), reload.clone())),
            reload,
            cancelled: false,
            step: StepOutput::default(),
        })
    }

    fn revision(&self) -> u64 {
        self.reload.current().revision().get()
    }

    /// The environment the step at `index` starts from: the previous state's, or the initial
    /// state's for `init`.
    fn env(&self, index: usize) -> Result<Env, DriverError> {
        self.script
            .get(index.saturating_sub(1))
            .cloned()
            .ok_or_else(|| DriverError::Refused(format!("the trace has no state {index}")))
    }

    fn note(&mut self, events: Vec<DomainEvent>) {
        self.step
            .events
            .extend(events.into_iter().filter_map(health_event));
    }

    /// Records what a step of the decisions reported: its health events, in order; a
    /// problem is only a warning, which the model does not hold; a refused readiness is a
    /// divergence, since the model never refuses one.
    fn take(&mut self, step: StepReport) -> Result<(), DriverError> {
        for report in step.reports {
            match report {
                WatchReport::Event(event) => self.step.events.extend(health_event(event)),
                WatchReport::Problem(_) => {}
                WatchReport::ReadinessRefused(refusal) => {
                    return Err(DriverError::Refused(format!(
                        "the coordinator refused a reconcile's readiness: {refusal}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn init(&mut self, env: &Env) -> Result<(), DriverError> {
        self.clock = FakeClock::new();
        self.reload = new_reload_state();
        self.decisions = WatchDecisions::new((self.make)(window(), self.reload.clone()));
        self.cancelled = false;
        // Start-up reports readiness only when every eligible directory is watched; otherwise
        // the registration failure waits in the source (the model's first message), or the
        // registration passed the relisting cap and waits for the `Overflow`'s full reconcile.
        if env.outstanding.is_empty() && (!env.d_present || env.watched.contains(&Entry::DirD)) {
            let events = self
                .decisions
                .coordinator_mut()
                .watcher_ready(env.watched.len())
                .map_err(|e| DriverError::Refused(e.to_string()))?;
            self.note(events);
        }
        Ok(())
    }

    fn receive(&mut self, env: &Env, params: &Params) -> Result<(), DriverError> {
        let k = usize::try_from(params.int("k")?)
            .map_err(|_| DriverError::Refused("the batch size k is out of range".into()))?;
        let capped = params.bool("capped")?;
        let Some(batch) = env.next_batch(k) else {
            return Err(DriverError::Refused("the source has nothing ready".into()));
        };
        let step = self
            .decisions
            .receive(batch, &self.clock, |_: ReconcileScope| {
                env.reconcile(capped)
            })
            .ok_or_else(|| DriverError::Refused("the coordinator closed".into()))?;
        self.step.recorded = step.recorded;
        self.step.needs_reconcile = step.needs_reconcile;
        self.take(step)
    }

    fn settle(&mut self, env: &Env, params: &Params) -> Result<(), DriverError> {
        let capped = params.bool("capped")?;
        let step = self
            .decisions
            .settle(&self.clock, |_: ReconcileScope| env.reconcile(capped));
        self.step.rescan = step.rescan;
        self.take(step)
    }
}

/// A domain event as the health event the model names, if it is one.
fn health_event(event: DomainEvent) -> Option<HealthEvent> {
    match event {
        DomainEvent::WatcherDegraded { reason } => Some(HealthEvent::Degraded(reason)),
        DomainEvent::WatcherRecovered => Some(HealthEvent::Recovered),
        DomainEvent::RevisionPublished(_) => None,
    }
}

impl<C: Coordinator> Driver for WatcherDriver<C> {
    type Obs = WatcherObs;

    fn apply(&mut self, action: &str, params: &Params) -> Result<(), DriverError> {
        let index = self.applied;
        self.applied += 1;
        self.step = StepOutput {
            revision_before: self.revision(),
            ..StepOutput::default()
        };
        let env = self.env(index)?;
        match action {
            "init" | "initUnwatchable" | "initCapped" => {
                self.init(&env)?;
                self.step.revision_before = self.revision();
            }
            "tick" => self.clock.advance(TICK_MS),
            // The pipeline task is gone: nothing receives or settles any more.
            "receive" | "settle" | "shutdown" if self.cancelled => {
                return Err(DriverError::Refused(format!("{action} after shutdown")));
            }
            "receive" => self.receive(&env, params)?,
            "settle" => self.settle(&env, params)?,
            "shutdown" => self.cancelled = true,
            "modify" | "createFile" | "deleteFile" | "renameFile" | "saveAtomically"
            | "tempWrite" | "gitChange" | "dirDelete" | "dirCreate" | "freeWatch"
            | "rootDelete" | "rootRecreate" | "kernelQueue" | "runtimeFailure" | "duplicate"
            | "reorder" => {}
            other => return Err(DriverError::UnknownAction(other.to_owned())),
        }
        Ok(())
    }

    fn observe(&self) -> WatcherObs {
        let coordinator = self.decisions.coordinator();
        let deadline = coordinator.deadline();
        let revision = self.revision();
        let published = (revision != self.step.revision_before).then(|| {
            let current = self.reload.subscribe();
            let changes = &current.initial().changes;
            Publish {
                full: changes.is_full(),
                rescan: self.step.rescan.unwrap_or(false),
                paths: changes
                    .changes()
                    .iter()
                    .map(|c| c.url_path().to_owned())
                    .collect(),
            }
        });
        WatcherObs {
            revision,
            dirty: deadline.is_some(),
            deadline_ms: deadline.map(|d| d.at().as_millis()),
            health: match coordinator.health() {
                WatcherHealth::Starting => Health::Starting,
                WatcherHealth::Watching { .. } => Health::Watching,
                WatcherHealth::Degraded { reason } => Health::Degraded(reason),
            },
            recorded: self.step.recorded,
            needs_reconcile: self.step.needs_reconcile,
            events: self.step.events.clone(),
            published,
        }
    }
}

/// The debounce window the model's `WINDOW` stands for.
fn window() -> DebounceWindow {
    DebounceWindow::DEFAULT
}

fn new_reload_state() -> ReloadState {
    ReloadState::new(InstanceId::from_parts(1, 1))
}

/// The record field `name` of `value`, which is `what` in the model.
fn field<'v>(value: &'v Value, name: &str, what: &str) -> Result<&'v Value, ProjectionError> {
    value
        .field(name)
        .ok_or_else(|| ProjectionError::missing(&format!("{what}.{name}")))
}

/// Projects a state of `formal/watcher.qnt` (or of a module importing it, such as
/// `watcher_traces.qnt`) into the driver's observation.
///
/// # Errors
///
/// [`ProjectionError`] when a variable is absent or has a shape the model does not give it.
pub fn project(state: &State) -> Result<WatcherObs, ProjectionError> {
    let var = |name: &str| {
        state
            .var(name)
            .ok_or_else(|| ProjectionError::missing(name))
    };
    let int = |value: &Value, what: &str| {
        value
            .as_int()
            .and_then(|i| u64::try_from(i).ok())
            .ok_or_else(|| ProjectionError::missing(what))
    };
    let flag = |value: &Value, what: &str| {
        value
            .as_bool()
            .ok_or_else(|| ProjectionError::missing(what))
    };
    let deb = var("deb")?;
    let last = var("last")?;
    let dirty = flag(field(deb, "dirty", "deb")?, "deb.dirty")?;
    let deadline = int(field(deb, "deadline", "deb")?, "deb.deadline")?;
    let health = match var("health")?.as_variant() {
        Some(("HStarting", _)) => Health::Starting,
        Some(("HWatching", _)) => Health::Watching,
        Some(("HDegraded", why)) => Health::Degraded(reason(why)?),
        _ => return Err(ProjectionError::missing("health")),
    };
    let events = field(last, "events", "last")?
        .as_list()
        .ok_or_else(|| ProjectionError::missing("last.events"))?
        .iter()
        .map(|e| match e.as_variant() {
            Some(("EvDegraded", why)) => Ok(HealthEvent::Degraded(reason(why)?)),
            Some(("EvRecovered", _)) => Ok(HealthEvent::Recovered),
            _ => Err(ProjectionError(format!("{e} is not a health event"))),
        })
        .collect::<Result<_, _>>()?;
    let published = if flag(field(last, "fired", "last")?, "last.fired")? {
        Some(Publish {
            full: flag(field(last, "firedFull", "last")?, "last.firedFull")?,
            rescan: flag(field(last, "firedRescan", "last")?, "last.firedRescan")?,
            paths: entries(field(last, "firedPaths", "last")?)?
                .into_iter()
                .map(Entry::url_path)
                .collect(),
        })
    } else {
        None
    };
    Ok(WatcherObs {
        revision: int(var("revision")?, "revision")?,
        dirty,
        // Only a dirty debouncer has a deadline (the model's clean one holds 0).
        deadline_ms: dirty.then_some(deadline.saturating_mul(TICK_MS)),
        health,
        recorded: usize::try_from(int(field(last, "recorded", "last")?, "last.recorded")?)
            .map_err(|_| ProjectionError::missing("last.recorded"))?,
        needs_reconcile: flag(
            field(last, "needsReconcile", "last")?,
            "last.needsReconcile",
        )?,
        events,
        published,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model's window in ticks (`WINDOW` in `watcher.qnt`).
    const WINDOW_TICKS: u64 = 2;

    #[test]
    fn should_span_debounce_window_when_model_window_elapses() {
        // given
        let model_window = WINDOW_TICKS * TICK_MS;

        // when
        let window = window().as_millis();

        // then
        assert_eq!(model_window, window);
    }

    #[test]
    fn should_name_real_paths_when_entry_is_translated() {
        // given
        let entries = [
            Entry::Index,
            Entry::Sub,
            Entry::Temp,
            Entry::Git,
            Entry::Root,
        ];

        // when
        let urls: Vec<String> = entries.iter().map(|e| e.url_path()).collect();
        let roots: Vec<bool> = entries.iter().map(|e| e.rel_path().is_root()).collect();

        // then
        assert_eq!(
            urls,
            [
                "/index.html",
                "/d/b.js",
                "/.index.html.swp",
                "/.git/HEAD",
                "/"
            ]
        );
        assert_eq!(roots, [false, false, false, false, true]);
    }
}
