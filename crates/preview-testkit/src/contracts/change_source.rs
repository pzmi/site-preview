//! The `ChangeSource` contract harness: every implementation of the port, the scripted fake
//! and the native watcher alike, is run through the same named cases (ADR 0038, the contract
//! on `preview_application::ChangeSource`).
//!
//! A source is driven through what happens to the site, never through its own queue: a test
//! supplies a [`ChangeSourceFactory`] that starts a source watching a site built from a
//! [`WatchTree`] and then writes files, writes a burst of files the consumer does not keep up
//! with, and stops the source. A real factory does those things to a temporary directory and
//! lets the operating system report them; a fake factory scripts the batches the operating
//! system would cause. The test then calls [`change_source_contract`] (panics with the report)
//! or [`check_change_source_contract`] (returns it, for a negative control).
//!
//! Every wait is bounded by [`ChangeSourceFactory::DEADLINE`] through
//! [`wait_until`](crate::poll::wait_until): a source over the real operating system delivers
//! asynchronously, and the deadline only limits how long a failing case hangs. A fake that
//! delivers synchronously sets it to zero, so its run never sleeps. Futures are polled with a
//! no-op waker, never on a runtime: the contract says `next` never blocks, so polling is
//! enough.
//!
//! Not covered here, because it cannot be provoked portably: a runtime failure of the native
//! watcher (`Failed(RuntimeFailure)`). The adapter's own tests feed one to its callback.

use core::num::NonZeroUsize;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Wake;
use std::time::Duration;

use preview_application::{ChangeBatch, ChangeSource, SourceFailure};
use preview_domain::{ChangeEvent, ChangeKind, Component, RelPath};

use super::{Case, ContractReport, run_cases};
use crate::poll::wait_until;

/// A site described declaratively, created before the source starts watching it.
///
/// Paths are `/`-separated and relative to the site root; a factory creates missing parent
/// directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchTree {
    dirs: Vec<String>,
    files: Vec<String>,
    unwatchable: Vec<String>,
}

impl WatchTree {
    /// An empty site: the root directory only.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a directory.
    #[must_use]
    pub fn dir(mut self, path: &str) -> Self {
        self.dirs.push(path.to_owned());
        self
    }

    /// Adds a regular file.
    #[must_use]
    pub fn file(mut self, path: &str) -> Self {
        self.files.push(path.to_owned());
        self
    }

    /// Adds an eligible directory the source cannot watch. A real factory makes it deeper than
    /// the watcher descends; a fake reports the registration failure it would cause.
    #[must_use]
    pub fn unwatchable(mut self, path: &str) -> Self {
        self.unwatchable.push(path.to_owned());
        self
    }

    /// The directories, in the order they were added.
    #[must_use]
    pub fn dirs(&self) -> &[String] {
        &self.dirs
    }

    /// The regular files, in the order they were added.
    #[must_use]
    pub fn files(&self) -> &[String] {
        &self.files
    }

    /// The directories that cannot be watched, in the order they were added.
    #[must_use]
    pub fn unwatchable_dirs(&self) -> &[String] {
        &self.unwatchable
    }
}

/// Starts the implementation under test over a site and acts on that site.
///
/// `Site` owns whatever keeps the site alive and drives it: a temporary directory, or the
/// handle that feeds a scripted source.
pub trait ChangeSourceFactory {
    /// The implementation under test.
    type Source: ChangeSource<Event = ChangeEvent>;
    /// The site the source watches, and the means to change it.
    type Site;

    /// How long a case waits for a batch to arrive. Zero for a source that delivers as soon as
    /// the site changes: the harness then never sleeps.
    const DEADLINE: Duration;

    /// Builds a fresh site holding exactly `tree` and starts watching it with room for
    /// `capacity` pending notifications.
    fn start(tree: &WatchTree, capacity: NonZeroUsize) -> (Self::Source, Self::Site);

    /// Creates the regular file at `path`, or overwrites it.
    fn write(site: &mut Self::Site, path: &str);

    /// Writes every file of `paths` in a burst the consumer does not read during; a source
    /// with less room than the burst needs loses some of it.
    fn burst(site: &mut Self::Site, paths: &[String]) {
        for path in paths {
            Self::write(site, path);
        }
    }

    /// Stops the source the way its owner does: shutting it down, or dropping its producer.
    fn stop(site: &mut Self::Site, source: &mut Self::Source);
}

/// The names of the harness's cases, in run order.
pub const CHANGE_SOURCE_CASES: &[&str] = &[
    "write_delivered",
    "order",
    "ignored_subtree",
    "overflow_before_later_changes",
    "failure_not_terminal",
    "closed_sticky",
    "dropped_next_loses_nothing",
];

/// Runs every case against `F` and panics with the full report if any failed.
///
/// # Panics
///
/// When the implementation breaks a case of the contract.
pub fn change_source_contract<F: ChangeSourceFactory>() {
    check_change_source_contract::<F>().assert_passed();
}

/// Runs every case against `F` and returns the report; a panic inside a case (in the factory
/// or the implementation) is that case's failure.
#[must_use]
pub fn check_change_source_contract<F: ChangeSourceFactory>() -> ContractReport {
    let cases: [(&'static str, Case); 7] = [
        ("write_delivered", write_delivered::<F>),
        ("order", order::<F>),
        ("ignored_subtree", ignored_subtree::<F>),
        (
            "overflow_before_later_changes",
            overflow_before_later_changes::<F>,
        ),
        ("failure_not_terminal", failure_not_terminal::<F>),
        ("closed_sticky", closed_sticky::<F>),
        (
            "dropped_next_loses_nothing",
            dropped_next_loses_nothing::<F>,
        ),
    ];
    run_cases(&cases)
}

type Batch = ChangeBatch<ChangeEvent>;

/// Room for every notification a case causes, so only the overflow case can lose any.
const ROOMY: NonZeroUsize = NonZeroUsize::new(1024).expect("non-zero");
/// The smallest room there is: a burst of more than one notification overflows it.
const TIGHT: NonZeroUsize = NonZeroUsize::new(1).expect("non-zero");
/// Files in the overflow case's burst; each causes at least one notification.
const BURST: usize = 256;
/// Batches a single drain takes at most, so a source that never runs dry cannot hang a case.
const DRAIN_LIMIT: usize = 4096;
/// Batches the report quotes before eliding the rest.
const QUOTED: usize = 12;

fn rel(path: &str) -> RelPath {
    RelPath::new(
        path.split('/')
            .filter(|part| !part.is_empty())
            .map(|part| Component::new(part).expect("a harness path is valid"))
            .collect(),
    )
}

fn shown(path: &RelPath) -> String {
    path.components()
        .iter()
        .map(Component::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether `change` reports that the file at `path` was written.
fn writes(change: &ChangeEvent, path: &RelPath) -> bool {
    change.path == *path && matches!(change.kind, ChangeKind::Create | ChangeKind::Modify)
}

/// The position of the first batch reporting a write of `path`.
fn delivered_at(seen: &[Batch], path: &str) -> Option<usize> {
    let path = rel(path);
    seen.iter().position(|batch| match batch {
        ChangeBatch::Events(changes) => changes.iter().any(|change| writes(change, &path)),
        _ => false,
    })
}

/// The position, counted in changes across batches, of the first write of `path`.
fn change_at(seen: &[Batch], path: &str) -> Option<usize> {
    let path = rel(path);
    seen.iter()
        .filter_map(|batch| match batch {
            ChangeBatch::Events(changes) => Some(changes),
            _ => None,
        })
        .flatten()
        .position(|change| writes(change, &path))
}

fn describe(batch: &Batch) -> String {
    match batch {
        ChangeBatch::Events(changes) => {
            let listed: Vec<String> = changes
                .iter()
                .map(|change| format!("{:?} {}", change.kind, shown(&change.path)))
                .collect();
            format!("Events[{}]", listed.join(", "))
        }
        other => format!("{other:?}"),
    }
}

/// The batches seen, for a failure line.
fn transcript(seen: &[Batch]) -> String {
    let mut text: Vec<String> = seen.iter().take(QUOTED).map(describe).collect();
    if seen.len() > QUOTED {
        text.push(format!("... ({} more)", seen.len() - QUOTED));
    }
    format!("[{}]", text.join(", "))
}

/// Polls one `next` once and drops it.
fn poll_once<S: ChangeSource>(source: &mut S, waker: &Waker) -> Poll<ChangeBatch<S::Event>> {
    let future = pin!(source.next());
    future.poll(&mut Context::from_waker(waker))
}

/// Records whether it was woken.
struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Polls one `next` as an executor would: again for as long as it wakes itself while being
/// polled, then drops it if it is still pending.
fn poll_woken<S: ChangeSource>(source: &mut S) -> Poll<ChangeBatch<S::Event>> {
    let flag = Arc::new(Flag(AtomicBool::new(false)));
    let waker = Waker::from(Arc::clone(&flag));
    let mut future = pin!(source.next());
    for _ in 0..DRAIN_LIMIT {
        let poll = future.as_mut().poll(&mut Context::from_waker(&waker));
        if poll.is_ready() || !flag.0.swap(false, Ordering::SeqCst) {
            return poll;
        }
    }
    Poll::Pending
}

/// Takes every batch available now, stopping at `Closed` (which repeats for ever).
fn drain<S: ChangeSource<Event = ChangeEvent>>(source: &mut S, seen: &mut Vec<Batch>) {
    for _ in 0..DRAIN_LIMIT {
        match poll_woken(source) {
            Poll::Ready(batch) => {
                let closed = batch == ChangeBatch::Closed;
                seen.push(batch);
                if closed {
                    return;
                }
            }
            Poll::Pending => return,
        }
    }
}

/// Collects batches until `done` holds of them, within the factory's deadline.
fn collect_until<F: ChangeSourceFactory>(
    source: &mut F::Source,
    seen: &mut Vec<Batch>,
    awaited: &str,
    done: impl Fn(&[Batch]) -> bool,
) -> Result<(), String> {
    wait_until(F::DEADLINE, awaited, || {
        drain(source, seen);
        done(seen) || seen.last() == Some(&ChangeBatch::Closed)
    })
    .map_err(|timeout| format!("{timeout}; batches seen: {}", transcript(seen)))?;
    if done(seen) {
        Ok(())
    } else {
        Err(format!(
            "closed before {awaited}; batches seen: {}",
            transcript(seen)
        ))
    }
}

/// The failed checks of one case, one per line.
#[derive(Default)]
struct Checks(Vec<String>);

impl Checks {
    fn check(&mut self, ok: bool, line: impl FnOnce() -> String) {
        if !ok {
            self.0.push(line());
        }
    }

    fn wellformed(&mut self, seen: &[Batch]) {
        let empty = seen
            .iter()
            .any(|batch| matches!(batch, ChangeBatch::Events(changes) if changes.is_empty()));
        self.check(!empty, || {
            format!("an empty Events batch was delivered: {}", transcript(seen))
        });
    }

    fn only_events(&mut self, seen: &[Batch]) {
        let other = seen
            .iter()
            .find(|batch| !matches!(batch, ChangeBatch::Events(_)));
        self.check(other.is_none(), || {
            format!(
                "a quiet, fully watched site reported {}: {}",
                other.map(describe).unwrap_or_default(),
                transcript(seen)
            )
        });
    }

    fn finish(self) -> Result<(), String> {
        if self.0.is_empty() {
            return Ok(());
        }
        Err(self
            .0
            .iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn indented(error: String) -> String {
    format!("  {error}")
}

/// A file written in a watched directory is eventually delivered as a change to its
/// root-relative path, in a non-empty `Events` batch.
fn write_delivered<F: ChangeSourceFactory>() -> Result<(), String> {
    let (mut source, mut site) = F::start(&WatchTree::new().file("index.html").dir("docs"), ROOMY);
    F::write(&mut site, "docs/page.html");
    let mut seen = Vec::new();
    collect_until::<F>(
        &mut source,
        &mut seen,
        "`docs/page.html` delivered",
        |seen| delivered_at(seen, "docs/page.html").is_some(),
    )
    .map_err(indented)?;
    let mut checks = Checks::default();
    checks.wellformed(&seen);
    checks.only_events(&seen);
    checks.finish()
}

/// Changes arrive in the order the site saw them.
fn order<F: ChangeSourceFactory>() -> Result<(), String> {
    let (mut source, mut site) = F::start(&WatchTree::new().file("index.html"), ROOMY);
    F::write(&mut site, "first.html");
    F::write(&mut site, "second.html");
    let mut seen = Vec::new();
    collect_until::<F>(&mut source, &mut seen, "both writes delivered", |seen| {
        delivered_at(seen, "first.html").is_some() && delivered_at(seen, "second.html").is_some()
    })
    .map_err(indented)?;
    let mut checks = Checks::default();
    checks.wellformed(&seen);
    let first = change_at(&seen, "first.html");
    let second = change_at(&seen, "second.html");
    checks.check(first < second, || {
        format!(
            "`second.html` was delivered before `first.html`, written first: {}",
            transcript(&seen)
        )
    });
    checks.finish()
}

/// Nothing inside an ignored subtree (an excluded or hidden directory, wherever it sits) is
/// delivered. The marker is written last: once it has arrived, a change in an ignored
/// directory written before it would have arrived too.
fn ignored_subtree<F: ChangeSourceFactory>() -> Result<(), String> {
    let ignored = ["node_modules", ".git", "docs/vendor", "docs/.cache"];
    let tree = WatchTree::new()
        .file("index.html")
        .dir("node_modules/pkg")
        .dir(".git/objects")
        .dir("docs/vendor")
        .dir("docs/.cache");
    let (mut source, mut site) = F::start(&tree, ROOMY);
    for path in [
        "node_modules/pkg/index.js",
        ".git/objects/ab",
        "docs/vendor/lib.js",
        "docs/.cache/entry",
    ] {
        F::write(&mut site, path);
    }
    F::write(&mut site, "docs/marker.html");
    let mut seen = Vec::new();
    collect_until::<F>(
        &mut source,
        &mut seen,
        "`docs/marker.html` delivered",
        |seen| delivered_at(seen, "docs/marker.html").is_some(),
    )
    .map_err(indented)?;
    let mut checks = Checks::default();
    checks.wellformed(&seen);
    let leaked: Vec<String> = seen
        .iter()
        .filter_map(|batch| match batch {
            ChangeBatch::Events(changes) => Some(changes),
            _ => None,
        })
        .flatten()
        .filter(|change| ignored.iter().any(|dir| change.path.starts_with(&rel(dir))))
        .map(|change| shown(&change.path))
        .collect();
    checks.check(leaked.is_empty(), || {
        format!("changes inside an ignored subtree were delivered: {leaked:?}")
    });
    checks.finish()
}

/// A source with room for one notification is handed a burst it cannot hold, then a later
/// change. Either nothing was lost (every burst file arrives before the later change, and no
/// `Overflow` is owed) or `Overflow` arrives before the later change does; a source that loses
/// changes without reporting it delivers the later change after a gap, or never.
fn overflow_before_later_changes<F: ChangeSourceFactory>() -> Result<(), String> {
    let (mut source, mut site) = F::start(&WatchTree::new().file("index.html").dir("burst"), TIGHT);
    let burst: Vec<String> = (0..BURST).map(|n| format!("burst/f{n}.html")).collect();
    F::burst(&mut site, &burst);
    F::write(&mut site, "late.html");
    let mut seen = Vec::new();
    collect_until::<F>(
        &mut source,
        &mut seen,
        "`Overflow` or `late.html` delivered",
        |seen| seen.contains(&ChangeBatch::Overflow) || delivered_at(seen, "late.html").is_some(),
    )
    .map_err(|error| format!("  changes were lost and no `Overflow` reported them ({error})"))?;
    let mut checks = Checks::default();
    checks.wellformed(&seen);
    let overflow = seen
        .iter()
        .position(|batch| *batch == ChangeBatch::Overflow);
    let late = delivered_at(&seen, "late.html");
    let reported_first = match (overflow, late) {
        (Some(overflow), Some(late)) => overflow < late,
        (Some(_), None) => true,
        (None, _) => false,
    };
    if !reported_first {
        let missing: Vec<&String> = burst
            .iter()
            .filter(|path| delivered_at(&seen, path).is_none())
            .collect();
        checks.check(missing.is_empty(), || {
            format!(
                "`late.html` was delivered without an `Overflow` before it, yet {} of {BURST} \
                 burst files never arrived (first: {:?}): {}",
                missing.len(),
                missing.first(),
                transcript(&seen)
            )
        });
    }
    checks.finish()
}

/// A directory that cannot be watched is reported as `Failed(RegistrationFailed)`, and the
/// source keeps delivering changes elsewhere after it.
fn failure_not_terminal<F: ChangeSourceFactory>() -> Result<(), String> {
    let tree = WatchTree::new().file("index.html").unwatchable("deep");
    let (mut source, mut site) = F::start(&tree, ROOMY);
    F::write(&mut site, "after.html");
    let failed = ChangeBatch::Failed(SourceFailure::RegistrationFailed);
    let mut seen = Vec::new();
    collect_until::<F>(
        &mut source,
        &mut seen,
        "the failure and `after.html` delivered",
        |seen| seen.contains(&failed) && delivered_at(seen, "after.html").is_some(),
    )
    .map_err(indented)?;
    let mut checks = Checks::default();
    checks.wellformed(&seen);
    let failed_at = seen.iter().position(|batch| *batch == failed);
    checks.check(failed_at < delivered_at(&seen, "after.html"), || {
        format!(
            "the registration failure came after a later change: {}",
            transcript(&seen)
        )
    });
    checks.finish()
}

/// Once stopped, the source answers at once: what was already queued may still come, then
/// `Closed`, and `Closed` for every later call; a change made after the stop never arrives.
fn closed_sticky<F: ChangeSourceFactory>() -> Result<(), String> {
    let (mut source, mut site) = F::start(&WatchTree::new().file("index.html"), ROOMY);
    F::write(&mut site, "before.html");
    let mut seen = Vec::new();
    collect_until::<F>(&mut source, &mut seen, "`before.html` delivered", |seen| {
        delivered_at(seen, "before.html").is_some()
    })
    .map_err(indented)?;
    F::stop(&mut site, &mut source);
    F::write(&mut site, "after-stop.html");
    let mut checks = Checks::default();
    let mut after = Vec::new();
    for _ in 0..DRAIN_LIMIT {
        match poll_once(&mut source, Waker::noop()) {
            Poll::Ready(ChangeBatch::Closed) => break,
            Poll::Ready(batch) => after.push(batch),
            Poll::Pending => {
                checks.check(false, || {
                    format!(
                        "`next` was pending after the stop instead of returning `Closed`; \
                         batches after the stop: {}",
                        transcript(&after)
                    )
                });
                return checks.finish();
            }
        }
    }
    let again: Vec<Poll<Batch>> = (0..3)
        .map(|_| poll_once(&mut source, Waker::noop()))
        .collect();
    checks.check(
        again
            .iter()
            .all(|poll| *poll == Poll::Ready(ChangeBatch::Closed)),
        || format!("`Closed` did not repeat at once: {again:?}"),
    );
    checks.check(delivered_at(&after, "after-stop.html").is_none(), || {
        format!(
            "a change made after the stop was delivered: {}",
            transcript(&after)
        )
    });
    checks.wellformed(&after);
    checks.finish()
}

/// A pending `next` arranges a wake-up when a change arrives, and dropping it (woken but not
/// polled again), or dropping a fresh one after a single poll, loses nothing: the change still
/// arrives.
fn dropped_next_loses_nothing<F: ChangeSourceFactory>() -> Result<(), String> {
    let (mut source, mut site) = F::start(&WatchTree::new().file("index.html"), ROOMY);
    let mut checks = Checks::default();
    let mut seen = Vec::new();
    let flag = Arc::new(Flag(AtomicBool::new(false)));
    let waker = Waker::from(Arc::clone(&flag));
    {
        let mut pending = pin!(source.next());
        match pending.as_mut().poll(&mut Context::from_waker(&waker)) {
            Poll::Ready(batch) => seen.push(batch),
            Poll::Pending => {
                F::write(&mut site, "a.html");
                let woken = wait_until(F::DEADLINE, "the pending `next` woken", || {
                    flag.0.load(Ordering::SeqCst)
                });
                checks.check(woken.is_ok(), || {
                    "a pending `next` was never woken after a change arrived".to_owned()
                });
            }
        }
    }
    if seen.is_empty() {
        // Polled once and dropped even if it woke itself: a consumer racing `next` against a
        // deadline drops the loser wherever it stands.
        if let Poll::Ready(batch) = poll_once(&mut source, Waker::noop()) {
            seen.push(batch);
        }
    } else {
        F::write(&mut site, "a.html");
    }
    if let Err(error) = collect_until::<F>(&mut source, &mut seen, "`a.html` delivered", |seen| {
        delivered_at(seen, "a.html").is_some()
    }) {
        checks.check(false, || {
            format!("a change was lost when `next` futures were dropped: {error}")
        });
    }
    checks.wellformed(&seen);
    checks.finish()
}
