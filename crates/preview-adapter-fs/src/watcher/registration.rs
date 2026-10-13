//! Registering the eligible directories with the native watcher (plan.md section 8.1 steps 3
//! to 6, section 8.4).
//!
//! - **One watch per eligible directory.** [`NotifyWatcher::start`] walks the site with
//!   [`eligible_dirs`] and calls `watch(dir, NonRecursive)` for each directory it returns, the
//!   root included, parents before children. A directory the walk prunes is never named to the
//!   operating system.
//! - **Watch, then list.** The walk lists a directory before its watch exists, so a child
//!   created in between would be neither listed nor notified. Each directory newly watched is
//!   therefore listed once more after its watch is in place, and every eligible child that the
//!   walk did not see is registered the same way, until nothing new appears: a child created
//!   before the second listing is in it, one created after it is notified. One call registers
//!   at most [`MAX_RELISTED`] such children, so a tree that keeps growing as fast as it is
//!   listed cannot hold the call: past the cap the watcher records that a rescan is needed,
//!   and the source turns that into a full reconcile, which picks up the rest.
//! - **Absolute paths only here.** The path handed to `notify` is `display_root` joined with
//!   the directory's validated components; it is built in [`NotifyWatcher::absolute`] and goes
//!   nowhere else. Errors name the relative directory.
//! - **Partial failure.** A directory that cannot be walked or watched becomes a
//!   [`RegistrationError`] and the rest is still registered. Once the system's watch limit is
//!   reached no further watch is attempted (each would fail the same way) until one of this
//!   watcher's watches is removed or a full reconcile probes again; the remaining directories
//!   are reported with [`WatchFault::WatchLimitReached`].
//! - **No link registration.** `notify` registers by path and follows a symbolic link, so after
//!   each watch the directory is reopened through the root capability (`O_NOFOLLOW` per
//!   component). If it is no longer a real directory there (a link swapped in after the walk),
//!   the watch is removed and the directory reported. `inotify_add_watch` follows the link, so
//!   for a link to a directory already watched it hands back that directory's watch, and the
//!   withdrawal removes it; after any withdrawal every recorded directory is therefore watched
//!   again in place (on Linux). A swap in the instant between the watch and that check, then
//!   back, is not detected; ADR 0047 records the residual window.
//! - **The root's parent.** On Linux the root's parent directory is watched too, non-recursively
//!   and outside the [`WatchSet`]: the server holds the root's descriptor, which keeps the
//!   root's inode alive, so `inotify` reports neither `IN_DELETE_SELF` nor `IN_IGNORED` on the
//!   root's own watch when the root is removed. Only the parent reports it, as the removal of
//!   an entry whose path is the root: the bridge turns that into a change on the root, whose
//!   reconcile finds the root removed (ADR 0073). Every other entry of the parent is outside
//!   the root, and the bridge drops it unread.
//! - **macOS: one recursive stream, no per-directory watches.** `notify`'s FSEvents backend
//!   rebuilds its whole stream on every `watch` and `unwatch`, from "now", so a change made
//!   while a registration restarts it is lost, and registering a copied-in tree of hundreds
//!   of directories one by one lost the events that would have registered the rest (and an
//!   `unwatch` of a link removes its target's entry, which is keyed by canonical path). The
//!   kernel stream is recursive by nature anyway. So on macOS the first registration starts
//!   one recursive watch of the resolved root and every other `watch_one` only confirms
//!   through the capability that a real directory stands there; nothing is restarted or
//!   removed afterwards. The [`WatchSet`] stays the record of eligible directories, so
//!   reconciliation is unchanged. The stream reports ignored subtrees too; the bridge drops
//!   their changes (ADR 0075).
//! - **Record.** Successful registrations are kept in a [`WatchSet`], the answer to "is this
//!   directory watched" for reconciliation.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};

use notify::{Config, EventHandler, RecommendedWatcher, RecursiveMode, Watcher};
use preview_application::{ContentRefusal, ForbiddenReason};
use preview_domain::{RelPath, WatchSet};

use super::display_rel;
use crate::capability::RootDir;
use crate::traversal::{
    MAX_DEPTH, Traversal, TraversalError, TraversalFault, eligible_children, eligible_dirs,
};

/// How many rounds of re-watching after a withdrawal one call runs at most; a round only
/// repeats when a link was swapped in again meanwhile, and the next call carries on.
const MAX_REPAIR_ROUNDS: usize = 4;

/// How many directories the second listings of one registration call queue at most beyond
/// what its walk found; the rest is left to a full reconcile (ADR 0050).
pub(crate) const MAX_RELISTED: usize = 4096;

/// Why a directory is not watched. A category only: the OS error text and the absolute path
/// never leave the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchFault {
    /// The system denied listing or watching the directory.
    PermissionDenied,
    /// The directory vanished, or is no longer a real directory (a link or file now stands
    /// at its path).
    NotFound,
    /// The system's limit on watches is reached (Linux `fs.inotify.max_user_watches`).
    WatchLimitReached,
    /// The native watcher could not be created: the limit on watcher instances (Linux
    /// `fs.inotify.max_user_instances`) or on open files is reached.
    InstanceLimitReached,
    /// The directory lies deeper than [`crate::MAX_DEPTH`].
    TooDeep,
    /// Any other operating-system failure, by kind.
    Io(io::ErrorKind),
    /// The native watcher failed without an operating-system error kind.
    Unavailable,
}

impl fmt::Display for WatchFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::NotFound => f.write_str("not found or no longer a directory"),
            Self::WatchLimitReached => f.write_str(
                "the system's file watch limit is reached (on Linux raise fs.inotify.max_user_watches)",
            ),
            Self::InstanceLimitReached => f.write_str(
                "the system's limit on file watcher instances or open files is reached (on Linux raise fs.inotify.max_user_instances)",
            ),
            Self::TooDeep => write!(f, "nested more than {} levels deep", crate::MAX_DEPTH),
            Self::Io(kind) => write!(f, "{kind}"),
            Self::Unavailable => f.write_str("the native file watcher is unavailable"),
        }
    }
}

impl From<TraversalFault> for WatchFault {
    fn from(fault: TraversalFault) -> Self {
        match fault {
            TraversalFault::PermissionDenied => Self::PermissionDenied,
            TraversalFault::TooDeep => Self::TooDeep,
            TraversalFault::Io(io::ErrorKind::NotFound) => Self::NotFound,
            TraversalFault::Io(kind) => Self::Io(kind),
        }
    }
}

impl From<&notify::Error> for WatchFault {
    fn from(error: &notify::Error) -> Self {
        match &error.kind {
            notify::ErrorKind::Io(e) => match e.kind() {
                io::ErrorKind::PermissionDenied => Self::PermissionDenied,
                io::ErrorKind::NotFound => Self::NotFound,
                kind => Self::Io(kind),
            },
            notify::ErrorKind::PathNotFound | notify::ErrorKind::WatchNotFound => Self::NotFound,
            notify::ErrorKind::MaxFilesWatch => Self::WatchLimitReached,
            notify::ErrorKind::Generic(_) | notify::ErrorKind::InvalidConfig(_) => {
                Self::Unavailable
            }
        }
    }
}

impl WatchFault {
    /// The fault of the native watcher failing to start. `inotify_init1` fails with `EMFILE`
    /// when the per-user instance limit or the process's descriptor limit is reached, and with
    /// `ENFILE` at the system's file limit; both are named rather than left as an
    /// uncategorized error. Pinned by `should_name_instance_limit_when_watcher_cannot_start`.
    fn starting(error: &notify::Error) -> Self {
        if let notify::ErrorKind::Io(e) = &error.kind
            && let Some(errno) = rustix::io::Errno::from_io_error(e)
            && (errno == rustix::io::Errno::MFILE || errno == rustix::io::Errno::NFILE)
        {
            return Self::InstanceLimitReached;
        }
        Self::from(error)
    }
}

/// A directory that is not watched, and why (plan.md section 8.1 step 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegistrationError {
    /// The directory, relative to the site root (empty for the root itself).
    pub dir: RelPath,
    /// Why.
    pub fault: WatchFault,
}

impl fmt::Display for RegistrationError {
    /// `cannot watch directory `<relative dir>`: <reason>`; pinned by
    /// `should_report_directory_and_reason_when_registration_fails`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot watch directory `{}`: {}",
            display_rel(&self.dir),
            self.fault
        )
    }
}

impl core::error::Error for RegistrationError {}

impl From<TraversalError> for RegistrationError {
    fn from(error: TraversalError) -> Self {
        Self {
            dir: error.dir,
            fault: error.fault.into(),
        }
    }
}

/// A test's hook into the window between listing a directory and watching it.
#[cfg(test)]
pub(crate) type WatchHook = Box<dyn FnMut(&RelPath) + Send>;

/// The native watcher and the directories registered with it. Dropping it removes every watch.
pub(crate) struct NotifyWatcher {
    inner: RecommendedWatcher,
    pub(super) display_root: PathBuf,
    pub(super) watched: WatchSet,
    /// Eligible directories that are not watched, and why: the walk or the watch failed.
    /// Reconciliation retries them; the set is complete only while this is empty.
    pub(crate) failed: BTreeMap<RelPath, WatchFault>,
    /// Set when a watch failed for the system's watch limit: no watch is attempted while it
    /// holds. Cleared when one of this watcher's watches is removed, or by a full reconcile.
    pub(crate) limit_reached: bool,
    /// Where the round-robin retry of [`Self::failed`] stopped last (ADR 0050).
    pub(super) retry_cursor: Option<RelPath>,
    /// Set when a watch was withdrawn: its removal may have taken the watch of another
    /// recorded directory with it (a link to it), so the recorded set is watched again.
    withdrawn: bool,
    /// Set once the one recursive watch of the root exists (macOS only, see the module
    /// documentation).
    recursive_root: bool,
    /// How many directories the second listings of one call may queue ([`MAX_RELISTED`];
    /// tests lower it).
    pub(crate) relist_cap: usize,
    /// Set when a call stopped queueing at [`Self::relist_cap`]: eligible directories may be
    /// unwatched, and only a full reconcile is sure to find them. Taken by the source.
    pub(crate) rescan_needed: bool,
    /// Called with each directory just before it is watched: the window between a listing and
    /// a watch, which tests use to create a directory in it.
    #[cfg(test)]
    pub(crate) before_watch: Option<WatchHook>,
}

impl fmt::Debug for NotifyWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotifyWatcher")
            .field("watched", &self.watched.len())
            .finish_non_exhaustive()
    }
}

impl NotifyWatcher {
    /// Creates the native watcher, delivering its notifications to `handler`, and registers
    /// every eligible directory of the site.
    ///
    /// `display_root` is the resolved root the capability `root` was opened from
    /// ([`crate::SiteRoot::display_path`]). The returned errors list each directory that could
    /// not be walked or watched; the others are watched regardless.
    ///
    /// # Errors
    ///
    /// The [`WatchFault`] of the native watcher itself failing to start (on Linux, the
    /// `inotify` instance limit: [`WatchFault::InstanceLimitReached`]); nothing is watched
    /// then.
    pub fn start<H: EventHandler>(
        root: &RootDir,
        display_root: &Path,
        handler: H,
    ) -> Result<(Self, Vec<RegistrationError>), WatchFault> {
        let mut watcher = Self::new(display_root, handler)?;
        watcher.watch_parent();
        let errors = match eligible_dirs(root, &RelPath::root()) {
            Ok(traversal) => watcher.register(root, traversal),
            Err(error) => vec![error.into()],
        };
        watcher.failed = errors.iter().map(|e| (e.dir.clone(), e.fault)).collect();
        Ok((watcher, errors))
    }

    /// The native watcher with nothing registered.
    ///
    /// On macOS `notify` 8.2 panics registering a path that is not UTF-8, so such a root is
    /// refused as [`WatchFault::Unavailable`] up front (ADR 0047).
    pub(crate) fn new<H: EventHandler>(
        display_root: &Path,
        handler: H,
    ) -> Result<Self, WatchFault> {
        if cfg!(target_os = "macos") && display_root.to_str().is_none() {
            return Err(WatchFault::Unavailable);
        }
        let inner = RecommendedWatcher::new(handler, Config::default())
            .map_err(|error| WatchFault::starting(&error))?;
        Ok(Self {
            inner,
            display_root: display_root.to_path_buf(),
            watched: WatchSet::new(),
            failed: BTreeMap::new(),
            limit_reached: false,
            retry_cursor: None,
            withdrawn: false,
            recursive_root: false,
            relist_cap: MAX_RELISTED,
            rescan_needed: false,
            #[cfg(test)]
            before_watch: None,
        })
    }

    /// Watches the root's parent directory on Linux (see the module documentation and ADR
    /// 0073), so the root's removal is notified although the root's descriptor is held. Best
    /// effort: when the parent cannot be watched (no permission to read it, the watch limit),
    /// or the root is `/`, a root removal is found only by a reconcile some other change asks
    /// for, as before. Watching it again is harmless: `inotify` keeps one watch per inode.
    /// Pinned by `should_notify_root_removal_when_emptied_root_is_removed` and
    /// `should_keep_watching_root_parent_when_a_link_to_it_is_withdrawn`.
    pub(crate) fn watch_parent(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(parent) = self.display_root.parent() {
            let _ = self.inner.watch(parent, RecursiveMode::NonRecursive);
        }
    }

    /// Registers the directories of `traversal` not yet watched and reports its walk errors
    /// beside the registration failures, in that order. Pinned by
    /// `should_watch_directory_created_between_listing_and_watch_when_registering`.
    pub(crate) fn register(
        &mut self,
        root: &RootDir,
        traversal: Traversal,
    ) -> Vec<RegistrationError> {
        self.register_relisting(root, traversal, Vec::new()).0
    }

    /// [`register`](Self::register), listing `relist` again first: directories already
    /// recorded whose watch was renewed after the walk listed them.
    ///
    /// Each directory is listed once more after its watch is in place, and an eligible child
    /// the walk did not see is registered the same way, until nothing new appears or
    /// [`Self::relist_cap`] such children were queued; then [`Self::rescan_needed`] is set.
    /// Returns the failures and the directories newly watched. Pinned by
    /// `should_hand_the_rest_to_a_full_reconcile_when_relisting_passes_the_cap`.
    pub(crate) fn register_relisting(
        &mut self,
        root: &RootDir,
        traversal: Traversal,
        relist: Vec<RelPath>,
    ) -> (Vec<RegistrationError>, Vec<RelPath>) {
        let mut walk = Walk {
            known: traversal.errors.iter().map(|e| e.dir.clone()).collect(),
            reported: traversal.errors.iter().map(|e| e.dir.clone()).collect(),
            errors: traversal
                .errors
                .into_iter()
                .map(RegistrationError::from)
                .collect(),
            queue: VecDeque::new(),
            room: self.relist_cap,
            truncated: false,
        };
        walk.known.extend(traversal.dirs.iter().cloned());
        walk.queue.extend(traversal.dirs);
        for dir in relist {
            walk.adopt_children(root, &dir, &self.watched);
        }
        let mut failures = Vec::new();
        let mut added = Vec::new();
        while let Some(dir) = walk.queue.pop_front() {
            if self.watched.contains(&dir) {
                continue;
            }
            if self.limit_reached {
                failures.push(RegistrationError {
                    dir,
                    fault: WatchFault::WatchLimitReached,
                });
                continue;
            }
            #[cfg(test)]
            if let Some(hook) = self.before_watch.as_mut() {
                hook(&dir);
            }
            match self.watch_one(root, &dir) {
                Ok(()) => {
                    self.watched.apply(&preview_domain::WatchDiff {
                        add: BTreeSet::from([dir.clone()]),
                        remove: BTreeSet::new(),
                    });
                    walk.adopt_children(root, &dir, &self.watched);
                    added.push(dir);
                }
                Err(fault) => {
                    self.limit_reached |= fault == WatchFault::WatchLimitReached;
                    failures.push(RegistrationError { dir, fault });
                }
            }
        }
        self.rescan_needed |= walk.truncated;
        self.repair(root, &mut failures);
        walk.errors.extend(failures);
        (walk.errors, added)
    }

    /// After a withdrawal, watches every recorded directory again in place, so a watch the
    /// withdrawal took from another directory (`inotify` hands back the existing watch for a
    /// link to a watched directory) is restored; the root's parent too, which a link to it
    /// would take the same way. Bounded by [`MAX_REPAIR_ROUNDS`]; what is left waits for the
    /// next call. Pinned by `should_keep_watching_link_target_when_link_is_withdrawn` and
    /// `should_keep_watching_root_parent_when_a_link_to_it_is_withdrawn`.
    pub(crate) fn repair(&mut self, root: &RootDir, errors: &mut Vec<RegistrationError>) {
        for _ in 0..MAX_REPAIR_ROUNDS {
            if !core::mem::take(&mut self.withdrawn) {
                return;
            }
            self.watch_parent();
            let recorded: Vec<RelPath> = self.watched.iter().cloned().collect();
            for dir in recorded {
                if self.watched.contains(&dir)
                    && let Err(fault) = self.refresh(root, &dir)
                    && fault != WatchFault::NotFound
                {
                    errors.push(RegistrationError { dir, fault });
                }
            }
        }
    }

    /// Watches one directory, then confirms through the capability that a real directory
    /// still stands at its path; otherwise the watch is withdrawn.
    fn watch_one(&mut self, root: &RootDir, dir: &RelPath) -> Result<(), WatchFault> {
        let path = self.absolute(dir);
        self.native_watch(&path)?;
        let Err(refusal) = root.open_dir(dir) else {
            return Ok(());
        };
        // The watch may already be gone with its inode. If a link stood there, the watch was
        // its target's, which may be recorded under its own name: `repair` restores it.
        if !cfg!(target_os = "macos") {
            let _ = self.inner.unwatch(&path);
            self.withdrawn = true;
        }
        Err(match refusal {
            ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied) => {
                WatchFault::PermissionDenied
            }
            ContentRefusal::Io(kind) => WatchFault::Io(kind),
            ContentRefusal::NotFound | ContentRefusal::Forbidden(_) => WatchFault::NotFound,
        })
    }

    /// Puts `path` under native watch: a non-recursive watch of the directory, except on macOS
    /// where the one recursive watch of the root covers every directory and is started once.
    fn native_watch(&mut self, path: &Path) -> Result<(), WatchFault> {
        self.native_watch_with(path, cfg!(target_os = "macos"))
    }

    /// [`Self::native_watch`] with the strategy given, so Linux runs the macOS one in a test
    /// (`should_watch_the_whole_tree_once_when_the_root_is_watched_recursively`).
    pub(crate) fn native_watch_with(
        &mut self,
        path: &Path,
        recursive_root: bool,
    ) -> Result<(), WatchFault> {
        let (target, mode) = if recursive_root {
            if self.recursive_root {
                return Ok(());
            }
            (self.display_root.clone(), RecursiveMode::Recursive)
        } else {
            (path.to_path_buf(), RecursiveMode::NonRecursive)
        };
        self.inner
            .watch(&target, mode)
            .map_err(|error| WatchFault::from(&error))?;
        self.recursive_root |= recursive_root;
        Ok(())
    }

    /// Removes the watches of `dirs` and forgets them. A watch the native watcher already
    /// dropped with its directory (`notify` removes the watch of a deleted or moved-away
    /// directory itself) is not an error: the record is what has to change.
    ///
    /// A removed watch frees one of the system's watches, so the watch limit is tried again.
    pub(crate) fn unwatch(&mut self, dirs: BTreeSet<RelPath>) {
        for dir in &dirs {
            if !cfg!(target_os = "macos") {
                let path = self.absolute(dir);
                let _ = self.inner.unwatch(&path);
            }
            self.limit_reached &= !self.watched.contains(dir);
        }
        self.watched.apply(&preview_domain::WatchDiff {
            add: BTreeSet::new(),
            remove: dirs,
        });
    }

    /// Watches `dir` again where it is already recorded, so the watch follows whatever
    /// directory stands under the name now: `inotify` watches an inode, and adding a watch for
    /// the same inode changes nothing, while a directory replaced under the same name gets a
    /// watch of its own. No watch is removed first, so no notification is missed meanwhile.
    /// On failure the directory is forgotten.
    ///
    /// FSEvents (macOS) watches paths, not inodes, so a replaced directory is still covered
    /// there, and every `watch` call restarts its stream: nothing is done on macOS.
    pub(crate) fn refresh(&mut self, root: &RootDir, dir: &RelPath) -> Result<(), WatchFault> {
        if cfg!(target_os = "macos") {
            return Ok(());
        }
        let refreshed = self.watch_one(root, dir);
        if refreshed.is_err() {
            self.unwatch(BTreeSet::from([dir.clone()]));
        }
        refreshed
    }

    /// The absolute path of `dir`: the only place one is built.
    fn absolute(&self, dir: &RelPath) -> PathBuf {
        let mut path = self.display_root.clone();
        path.extend(dir.components().iter().map(|name| name.as_str()));
        path
    }

    /// The directories currently watched, root-relative.
    #[must_use]
    pub fn watched(&self) -> &WatchSet {
        &self.watched
    }
}

/// The listing side of one registration: what the walk and the second listings found.
struct Walk {
    /// Every directory seen so far, watched or not, so none is queued twice.
    known: BTreeSet<RelPath>,
    /// Directories already reported as not walkable, so none is reported twice.
    reported: BTreeSet<RelPath>,
    errors: Vec<RegistrationError>,
    queue: VecDeque<RelPath>,
    /// How many more children the second listings may queue.
    room: usize,
    /// Set when a second listing found a child past the cap; it was not queued.
    truncated: bool,
}

impl Walk {
    /// Lists `dir`, whose watch is in place now, and queues each eligible child neither seen
    /// before nor already watched, while there is room; a child nested too deep is reported
    /// instead. A directory that vanished meanwhile is skipped: its removal is a change of its
    /// own. A watched child never counts against the room, so relisting a large site that is
    /// already watched never reaches the cap.
    fn adopt_children(&mut self, root: &RootDir, dir: &RelPath, watched: &WatchSet) {
        let listing = match eligible_children(root, dir) {
            Ok(listing) => listing,
            Err(error) if error.fault == TraversalFault::Io(io::ErrorKind::NotFound) => return,
            Err(error) => {
                self.report(error);
                return;
            }
        };
        for error in listing.errors {
            self.report(error);
        }
        for child in listing.dirs {
            if watched.contains(&child) || !self.known.insert(child.clone()) {
                continue;
            }
            if child.components().len() > MAX_DEPTH {
                self.report(TraversalError {
                    dir: child,
                    fault: TraversalFault::TooDeep,
                });
            } else if self.room == 0 {
                self.truncated = true;
            } else {
                self.room -= 1;
                self.queue.push_back(child);
            }
        }
    }

    fn report(&mut self, error: TraversalError) {
        if self.reported.insert(error.dir.clone()) {
            self.errors.push(error.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use preview_domain::Component;

    fn rel(text: &str) -> RelPath {
        RelPath::new(
            text.split('/')
                .filter(|part| !part.is_empty())
                .map(|part| Component::new(part).expect("valid component"))
                .collect(),
        )
    }

    #[test]
    fn should_name_relative_directory_and_reason_when_error_is_displayed() {
        // given
        let errors = [
            RegistrationError {
                dir: rel("docs/guide"),
                fault: WatchFault::PermissionDenied,
            },
            RegistrationError {
                dir: RelPath::root(),
                fault: WatchFault::NotFound,
            },
        ];

        // when
        let texts = errors.map(|e| e.to_string());

        // then
        assert_eq!(
            texts,
            [
                "cannot watch directory `docs/guide`: permission denied",
                "cannot watch directory `.`: not found or no longer a directory",
            ]
        );
    }

    #[test]
    fn should_map_notify_errors_to_categories_when_converted() {
        // given
        let errors = [
            notify::Error::new(notify::ErrorKind::MaxFilesWatch),
            notify::Error::path_not_found(),
            notify::Error::io(io::Error::from(io::ErrorKind::PermissionDenied)),
            notify::Error::io(io::Error::from(io::ErrorKind::NotFound)),
            notify::Error::io(io::Error::from(io::ErrorKind::OutOfMemory)),
            notify::Error::generic("text that must not leak"),
        ];

        // when
        let faults = errors.each_ref().map(WatchFault::from);

        // then
        assert_eq!(
            faults,
            [
                WatchFault::WatchLimitReached,
                WatchFault::NotFound,
                WatchFault::PermissionDenied,
                WatchFault::NotFound,
                WatchFault::Io(io::ErrorKind::OutOfMemory),
                WatchFault::Unavailable,
            ]
        );
    }

    #[test]
    fn should_name_instance_limit_when_watcher_cannot_start() {
        // given
        let errno = |e: rustix::io::Errno| io::Error::from_raw_os_error(e.raw_os_error());
        let errors = [
            notify::Error::io(errno(rustix::io::Errno::MFILE)),
            notify::Error::io(errno(rustix::io::Errno::NFILE)),
            notify::Error::io(errno(rustix::io::Errno::NOMEM)),
            notify::Error::new(notify::ErrorKind::MaxFilesWatch),
        ];

        // when
        let faults = errors.each_ref().map(WatchFault::starting);

        // then
        assert_eq!(
            faults,
            [
                WatchFault::InstanceLimitReached,
                WatchFault::InstanceLimitReached,
                WatchFault::Io(io::ErrorKind::OutOfMemory),
                WatchFault::WatchLimitReached,
            ]
        );
        assert!(
            WatchFault::InstanceLimitReached
                .to_string()
                .contains("fs.inotify.max_user_instances")
        );
        assert!(
            WatchFault::WatchLimitReached
                .to_string()
                .contains("fs.inotify.max_user_watches")
        );
    }

    #[test]
    fn should_map_traversal_faults_when_converted() {
        // given
        let faults = [
            TraversalFault::PermissionDenied,
            TraversalFault::TooDeep,
            TraversalFault::Io(io::ErrorKind::NotFound),
            TraversalFault::Io(io::ErrorKind::Interrupted),
        ];

        // when
        let mapped = faults.map(WatchFault::from);

        // then
        assert_eq!(
            mapped,
            [
                WatchFault::PermissionDenied,
                WatchFault::TooDeep,
                WatchFault::NotFound,
                WatchFault::Io(io::ErrorKind::Interrupted),
            ]
        );
    }
}
