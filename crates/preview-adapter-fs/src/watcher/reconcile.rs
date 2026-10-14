//! Keeping the watch set equal to the eligible directories after the tree changes (plan.md
//! section 8.1, "New directories"; section 8.4, the retry sentence; ADR 0050).
//!
//! - **Hints, not guesses.** The source turns each delivered change that the domain says may
//!   alter the directory structure ([`preview_domain::Classification::requests_reconcile`])
//!   into a [`ReconcileHint`]; a loss or a hint list that grew too long is [`ReconcileHint::Full`].
//!   An ignored path (`.git`, `node_modules`, an editor temporary name) never becomes a hint, so
//!   its subtree is never walked or watched.
//! - **An entry that was created, removed or renamed voids its old watches.** `inotify` watches
//!   an inode, not a name, and `notify` drops the watch of a deleted or moved-away directory by
//!   itself. So for [`ReconcileHint::Entry`] every watch recorded at or under the path is
//!   removed first, then the path is walked again and whatever eligible directory stands there
//!   now is registered with all its eligible descendants: a directory created or moved in, a
//!   directory deleted and created again under the same name, a tree renamed over another.
//!   All removals of a call happen before any registration, because `inotify` hands back the
//!   existing watch for an inode already watched under another name, which a later removal of
//!   the old name would take away.
//! - **Removed entries rescan the nearest existing parent.** When the path no longer holds a
//!   directory, the nearest ancestor that still exists is listed one level deep: an eligible
//!   child that is not watched is registered with its subtree, and a watched child that is gone
//!   is dropped with its subtree, so a change the OS folded into the removal is not lost.
//! - **Coarse changes are diffed.** [`ReconcileHint::Subtree`] (an event the OS would not
//!   attribute) and [`ReconcileHint::Full`] walk the subtree and apply the domain's
//!   [`WatchSet::reconcile`] diff. A directory still present is watched again in place
//!   (`NotifyWatcher::refresh`): for the same inode that changes nothing, and a directory
//!   replaced under the same name while the events saying so were lost gets its own watch,
//!   with no moment unwatched. (Comparing inode numbers cannot tell: a file system reuses a
//!   freed inode at once, within one timestamp tick.) A directory the walk could not list
//!   keeps its watched descendants, since nothing says they are gone.
//!   A directory kept is listed again after its refresh, as a new one is after its watch
//!   (`NotifyWatcher::register_relisting`): the walk listed a replaced directory before it had
//!   a watch of its own.
//! - **Links and ignored names.** Every walk is [`eligible_dirs`] through the root capability,
//!   and every registration goes through `NotifyWatcher::register`, which reopens the directory
//!   with `O_NOFOLLOW` after watching it: a link swapped in after the listing is withdrawn and
//!   never followed.
//! - **Bounded retries.** A directory that could not be walked or watched stays in the failed
//!   record. A changes-only reconcile (one per batch that may have changed the directory
//!   structure, so one per atomic save) retries at most [`RETRY_BUDGET`] of them, round robin
//!   (no loop, no timer); a full reconcile retries every one. While the system's watch limit
//!   is reached nothing is retried until one of this watcher's watches is removed, or a full
//!   reconcile probes again. The set is complete only when that record is empty. A failure is
//!   reported once, when it first appears or changes reason. A directory that vanished before
//!   it could be watched is not a failure: its removal is a change of its own.
//! - **The root itself.** Every reconcile first checks that the root path still names the
//!   directory the root capability holds ([`root_removed`]). When it does not (the root was
//!   deleted, moved away, or replaced by another directory at its path), nothing is walked or
//!   registered, and the report says `root_removed`: a watch on the path would follow a
//!   directory that is not served, and none can follow the served one (ADR 0067). A `stat`
//!   that fails for any reason but a missing path (an I/O error or a stale handle of a network
//!   file system) is not a removal: it may pass, and the next reconcile checks again.

use std::collections::BTreeSet;
use std::path::Path;

use preview_domain::{RelPath, WatchSet};

use super::registration::{NotifyWatcher, RegistrationError, WatchFault};
use crate::capability::RootDir;
use crate::traversal::{MAX_DEPTH, Traversal, eligible_children, eligible_dirs};

/// How many failed directories a changes-only reconcile retries at most: each retry is a walk
/// of the directory's subtree, and such a reconcile follows every atomic save.
pub(crate) const RETRY_BUDGET: usize = 16;

/// What part of the tree to bring back in line with the watch set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ReconcileHint {
    /// The entry at this path was created, removed or renamed (on either side of a rename):
    /// its old watches are void and whatever stands there now is registered.
    Entry(RelPath),
    /// Something at or under this path changed in a way the OS did not attribute: the subtree
    /// is walked and diffed.
    Subtree(RelPath),
    /// Every eligible directory: changes were lost, or there were too many hints.
    Full,
}

/// What a reconcile did (plan.md section 8.1 step 6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    /// The number of directories watched afterwards.
    pub watching: usize,
    /// Directories that newly could not be walked or watched, or failed for a new reason.
    /// A failure already reported is not repeated while it persists.
    pub errors: Vec<RegistrationError>,
    /// Whether every eligible directory is watched afterwards: no directory is outstanding
    /// from this or an earlier registration.
    pub complete: bool,
    /// The root path no longer names the served directory (deleted, moved away or replaced):
    /// nothing was reconciled, and live reload cannot work again until a restart (ADR 0067).
    pub root_removed: bool,
}

/// Whether `display_root` no longer names the directory `root` holds: the directory was
/// deleted (no link to it is left), or the path now names another object, or nothing. Pinned
/// by `should_report_root_removed_when_root_is_deleted` and
/// `should_report_root_removed_when_root_is_replaced_at_its_path`; the reading of each
/// outcome is [`removed`]'s.
///
/// Identity is the device and inode pair, and a deleted directory is told by its link count:
/// a file system may give a new directory the freed inode number at once, so the pair alone
/// would take a root deleted and recreated for the original. A held descriptor that cannot be
/// read says nothing either way, so it is not a removal.
pub(crate) fn root_removed(root: &RootDir, display_root: &Path) -> bool {
    let Ok(held) = rustix::fs::fstat(root.as_fd()) else {
        return false;
    };
    let named = match rustix::fs::stat(display_root) {
        Ok(named) if (named.st_dev, named.st_ino) == (held.st_dev, held.st_ino) => Named::Same,
        Ok(_) => Named::Other,
        Err(errno) => Named::Failed(errno),
    };
    removed(held.st_nlink == 0, named)
}

/// What a `stat` of the root path found, against the directory the root capability holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    /// The served directory.
    Same,
    /// Another object: the root was moved away or replaced at its path.
    Other,
    /// The `stat` failed.
    Failed(rustix::io::Errno),
}

/// Whether the root counts as removed, given whether the held directory has no link left and
/// what its path names (ADR 0067, amended by OPS-04's review). Only a definite answer degrades
/// for good: the held directory unlinked, another object at the path, or a failure that says
/// the path names no directory (`ENOENT`; `ENOTDIR`, a component now a file; `ELOOP`, the
/// path now a link loop). Any other failure (`EIO`, `ESTALE`, `ETIMEDOUT`, `ENOTCONN` from a
/// network or FUSE file system, `EACCES` on a parent, `ENOMEM`, `EINTR`) may pass, so it is
/// not a removal: the reconcile goes on through the held descriptor, and the next reconcile
/// checks again. `ENAMETOOLONG` is classified explicitly as transient too: the path was
/// accepted when the server started, so a limit error now is a file-system or mount fault, not
/// proof that no directory is there (it is not in the removal list on purpose). A removal hidden by such a failure is found by the next reconcile any change
/// asks for, as when the parent cannot be watched (ADR 0073). Pinned by
/// `should_count_only_a_missing_path_as_removed_when_stat_fails`,
/// `should_count_removed_when_held_directory_has_no_link_whatever_the_path_says` and
/// `should_count_removed_only_when_path_names_another_object`.
fn removed(unlinked: bool, named: Named) -> bool {
    use rustix::io::Errno;
    unlinked
        || match named {
            Named::Same => false,
            Named::Other => true,
            Named::Failed(errno) => match errno {
                Errno::NOENT | Errno::NOTDIR | Errno::LOOP => true,
                // Explicitly not a removal (see above); every other errno is the same.
                Errno::NAMETOOLONG => false,
                _ => false,
            },
        }
}

/// The planned work of one call.
#[derive(Default)]
struct Work {
    /// Watches to remove before anything is registered.
    voided: BTreeSet<RelPath>,
    /// Subtrees to walk and diff.
    rescans: BTreeSet<RelPath>,
    /// Directories to list one level deep after a removal.
    parents: BTreeSet<RelPath>,
}

impl NotifyWatcher {
    /// Brings the watch set in line with the tree for `hints`, retrying directories that
    /// failed before within the bounds of the module documentation. Pinned by the
    /// `os_watcher` tests, among them
    /// `should_retry_at_most_the_budget_when_many_directories_failed`,
    /// `should_retry_nothing_until_a_watch_is_freed_when_watch_limit_is_reached`,
    /// `should_rewatch_directory_when_deleted_and_recreated`,
    /// `should_drop_stale_watch_when_directory_removed`,
    /// `should_rescan_nearest_existing_parent_when_nested_directories_are_removed` and
    /// `should_report_relative_directory_once_when_new_directory_cannot_be_walked`.
    pub(crate) fn reconcile(
        &mut self,
        root: &RootDir,
        hints: &BTreeSet<ReconcileHint>,
    ) -> ReconcileReport {
        let before = self.failed.clone();
        let work = self.plan(root, hints);
        let mut errors = Vec::new();
        self.unwatch(work.voided);
        for start in outermost(&work.rescans) {
            self.rescan(root, start, &mut errors);
        }
        for parent in &work.parents {
            if !work.rescans.iter().any(|start| parent.starts_with(start)) {
                self.rescan_children(root, parent, &mut errors);
            }
        }
        self.repair(root, &mut errors);
        for error in &errors {
            self.failed.insert(error.dir.clone(), error.fault);
        }
        let mut seen = BTreeSet::new();
        errors.retain(|error| {
            seen.insert(error.dir.clone()) && before.get(&error.dir) != Some(&error.fault)
        });
        ReconcileReport {
            watching: self.watched.len(),
            errors,
            complete: self.failed.is_empty() && !self.rescan_needed,
            root_removed: false,
        }
    }

    /// Turns the hints and the failed record into the call's work.
    fn plan(&mut self, root: &RootDir, hints: &BTreeSet<ReconcileHint>) -> Work {
        let mut work = Work::default();
        let full = hints.iter().any(|hint| match hint {
            ReconcileHint::Full => true,
            ReconcileHint::Entry(path) | ReconcileHint::Subtree(path) => path.is_root(),
        });
        if full {
            // The walk covers every failed directory; it also probes the watch limit again,
            // which watches freed elsewhere on the system may have lifted.
            work.rescans.insert(RelPath::root());
            self.limit_reached = false;
        }
        for hint in hints {
            match hint {
                ReconcileHint::Entry(path) if !path.is_root() => {
                    work.voided.extend(under(&self.watched, path));
                    work.rescans.insert(path.clone());
                    if root.open_dir(path).is_err() {
                        work.parents.insert(self.nearest_existing(root, path));
                    }
                }
                ReconcileHint::Subtree(path) => {
                    work.rescans.insert(path.clone());
                }
                _ => {}
            }
        }
        if !full {
            for dir in self.retries() {
                self.failed.remove(&dir);
                work.rescans.insert(dir);
            }
        }
        work
    }

    /// The failed directories to try once more: none while the watch limit is reached, never a
    /// nesting too deep, and at most [`RETRY_BUDGET`], continuing after the last one tried.
    fn retries(&mut self) -> Vec<RelPath> {
        if self.limit_reached {
            return Vec::new();
        }
        let candidates: Vec<&RelPath> = self
            .failed
            .iter()
            .filter(|(_, fault)| **fault != WatchFault::TooDeep)
            .map(|(dir, _)| dir)
            .collect();
        let split = self
            .retry_cursor
            .as_ref()
            .map_or(0, |cursor| candidates.partition_point(|dir| *dir <= cursor));
        let chosen: Vec<RelPath> = candidates[split..]
            .iter()
            .chain(&candidates[..split])
            .take(RETRY_BUDGET)
            .map(|dir| (*dir).clone())
            .collect();
        if let Some(last) = chosen.last() {
            self.retry_cursor = Some(last.clone());
        }
        chosen
    }

    /// The nearest strict ancestor of `path` that still exists as a directory. The watch set
    /// names the candidates (a directory that was never watched cannot be rescanned through
    /// its watch); the root is the last resort.
    fn nearest_existing(&self, root: &RootDir, path: &RelPath) -> RelPath {
        let mut current = path.clone();
        while let Some(candidate) = self.watched.nearest_existing_ancestor(&current) {
            if root.open_dir(&candidate).is_ok() {
                return candidate;
            }
            current = candidate;
        }
        RelPath::root()
    }

    /// Walks `start` and moves the watches at and under it to what the walk found.
    fn rescan(&mut self, root: &RootDir, start: &RelPath, errors: &mut Vec<RegistrationError>) {
        self.forget_failures_under(start);
        if start.components().len() > MAX_DEPTH {
            errors.push(RegistrationError {
                dir: start.clone(),
                fault: WatchFault::TooDeep,
            });
            return;
        }
        let traversal = match eligible_dirs(root, start) {
            Ok(traversal) => traversal,
            Err(error) => {
                let error = RegistrationError::from(error);
                if error.fault == WatchFault::NotFound {
                    // Gone, or no longer a directory: nothing stands there to watch.
                    let gone = under(&self.watched, start);
                    self.unwatch(gone);
                } else {
                    errors.push(error);
                }
                return;
            }
        };
        let unlisted: Vec<RelPath> = traversal.errors.iter().map(|e| e.dir.clone()).collect();
        let current: WatchSet = under(&self.watched, start).into_iter().collect();
        let diff = current.reconcile(&traversal.dirs);
        let stale: BTreeSet<RelPath> = diff
            .remove
            .into_iter()
            .filter(|dir| !unlisted.iter().any(|blind| dir.starts_with(blind)))
            .collect();
        self.unwatch(stale);
        let kept = traversal.dirs.iter().filter(|dir| current.contains(dir));
        let mut refreshed = Vec::new();
        for dir in kept.cloned().collect::<Vec<_>>() {
            match self.refresh(root, &dir) {
                Ok(()) => refreshed.push(dir),
                Err(WatchFault::NotFound) => {}
                Err(fault) => errors.push(RegistrationError { dir, fault }),
            }
        }
        // FSEvents keeps a path's watch, so the listing of a kept directory is current there.
        if cfg!(target_os = "macos") {
            refreshed.clear();
        }
        let added = Traversal {
            dirs: diff.add,
            errors: traversal.errors,
        };
        self.register_quietly(root, added, refreshed, errors);
    }

    /// Lists `parent` one level deep: registers each eligible child that is not watched with
    /// its subtree, and drops each watched child that is gone with its subtree.
    fn rescan_children(
        &mut self,
        root: &RootDir,
        parent: &RelPath,
        errors: &mut Vec<RegistrationError>,
    ) {
        let Ok(listing) = eligible_children(root, parent) else {
            // The parent vanished too: its own removal is a change that will be reconciled.
            return;
        };
        let depth = parent.components().len() + 1;
        let gone: Vec<RelPath> = self
            .watched
            .iter()
            .filter(|dir| {
                dir.components().len() == depth
                    && dir.starts_with(parent)
                    && !listing.dirs.contains(*dir)
            })
            .cloned()
            .collect();
        for child in gone {
            let subtree = under(&self.watched, &child);
            self.unwatch(subtree);
        }
        for child in &listing.dirs {
            if !self.watched.contains(child) {
                self.rescan(root, child, errors);
            }
        }
        errors.extend(listing.errors.into_iter().map(RegistrationError::from));
    }

    /// Registers `traversal` (listing `relist` again first), keeping its failures except a
    /// directory that vanished first.
    fn register_quietly(
        &mut self,
        root: &RootDir,
        traversal: Traversal,
        relist: Vec<RelPath>,
        errors: &mut Vec<RegistrationError>,
    ) {
        let (failures, added) = self.register_relisting(root, traversal, relist);
        for dir in added {
            self.failed.remove(&dir);
        }
        errors.extend(
            failures
                .into_iter()
                .filter(|error| error.fault != WatchFault::NotFound),
        );
    }

    fn forget_failures_under(&mut self, start: &RelPath) {
        self.failed.retain(|dir, _| !dir.starts_with(start));
    }
}

/// The recorded directories at or under `path`.
fn under(watched: &WatchSet, path: &RelPath) -> BTreeSet<RelPath> {
    watched
        .iter()
        .filter(|dir| dir.starts_with(path))
        .cloned()
        .collect()
}

/// The paths of `starts` that no other path of it contains; a walk covers its descendants.
fn outermost(starts: &BTreeSet<RelPath>) -> Vec<&RelPath> {
    starts
        .iter()
        .filter(|path| {
            !starts
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .collect()
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
    fn should_count_only_a_missing_path_as_removed_when_stat_fails() {
        // given
        use rustix::io::Errno;
        let missing = [Errno::NOENT, Errno::NOTDIR, Errno::LOOP];
        let transient = [
            Errno::IO,
            Errno::STALE,
            Errno::TIMEDOUT,
            Errno::NOTCONN,
            Errno::ACCESS,
            Errno::NOMEM,
            Errno::INTR,
            Errno::NAMETOOLONG,
        ];

        // when
        let gone: Vec<bool> = missing
            .iter()
            .map(|errno| removed(false, Named::Failed(*errno)))
            .collect();
        let kept: Vec<bool> = transient
            .iter()
            .map(|errno| removed(false, Named::Failed(*errno)))
            .collect();

        // then
        assert_eq!(gone, [true; 3]);
        assert_eq!(kept, [false; 8]);
    }

    #[test]
    fn should_count_removed_when_held_directory_has_no_link_whatever_the_path_says() {
        // given
        let outcomes = [
            Named::Same,
            Named::Other,
            Named::Failed(rustix::io::Errno::IO),
        ];

        // when
        let removals: Vec<bool> = outcomes.iter().map(|named| removed(true, *named)).collect();

        // then
        assert_eq!(removals, [true; 3]);
    }

    #[test]
    fn should_count_removed_only_when_path_names_another_object() {
        // given
        let (same, other) = (Named::Same, Named::Other);

        // when
        let removals = (removed(false, same), removed(false, other));

        // then
        assert_eq!(removals, (false, true));
    }

    #[test]
    fn should_keep_only_outermost_paths_when_starts_nest() {
        // given
        let starts: BTreeSet<RelPath> = ["a", "a/b", "a/b/c", "ab", "z/y"]
            .into_iter()
            .map(rel)
            .collect();

        // when
        let kept = outermost(&starts);

        // then
        assert_eq!(kept, [&rel("a"), &rel("ab"), &rel("z/y")]);
    }

    #[test]
    fn should_select_path_and_descendants_when_listing_under() {
        // given
        let watched: WatchSet = ["", "a", "a/b", "ab", "b"].into_iter().map(rel).collect();

        // when
        let selected = under(&watched, &rel("a"));

        // then
        assert_eq!(selected, ["a", "a/b"].into_iter().map(rel).collect());
    }
}
