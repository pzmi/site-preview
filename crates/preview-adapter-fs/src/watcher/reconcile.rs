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
//! - **Links and ignored names.** Every walk is [`eligible_dirs`] through the root capability,
//!   and every registration goes through `NotifyWatcher::register`, which reopens the directory
//!   with `O_NOFOLLOW` after watching it: a link swapped in after the listing is withdrawn and
//!   never followed.
//! - **Bounded retries.** A directory that could not be walked or watched stays in the failed
//!   record and is tried once more on each later reconcile (no loop, no timer); the set is
//!   complete only when that record is empty. A failure is reported once, when it first appears
//!   or changes reason. A directory that vanished before it could be watched is not a failure:
//!   its removal is a change of its own.

use std::collections::BTreeSet;

use preview_domain::{RelPath, WatchSet};

use super::registration::{NotifyWatcher, RegistrationError, WatchFault};
use crate::capability::RootDir;
use crate::traversal::{MAX_DEPTH, Traversal, eligible_children, eligible_dirs};

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
    /// Brings the watch set in line with the tree for `hints`, retrying every directory that
    /// failed before. Pinned by the `os_watcher` tests, among them
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
            complete: self.failed.is_empty(),
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
            work.rescans.insert(RelPath::root());
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
        // One more try for each directory that failed before; a nesting too deep stays so.
        let retry: Vec<RelPath> = self
            .failed
            .iter()
            .filter(|(_, fault)| **fault != WatchFault::TooDeep)
            .map(|(dir, _)| dir.clone())
            .collect();
        for dir in retry {
            self.failed.remove(&dir);
            work.rescans.insert(dir);
        }
        work
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
        for dir in kept.cloned().collect::<Vec<_>>() {
            if let Err(fault) = self.refresh(root, &dir)
                && fault != WatchFault::NotFound
            {
                errors.push(RegistrationError { dir, fault });
            }
        }
        let added = Traversal {
            dirs: diff.add,
            errors: traversal.errors,
        };
        self.register_quietly(root, added, errors);
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

    /// Registers `traversal`, keeping its failures except a directory that vanished first.
    fn register_quietly(
        &mut self,
        root: &RootDir,
        traversal: Traversal,
        errors: &mut Vec<RegistrationError>,
    ) {
        let attempted: Vec<RelPath> = traversal.dirs.iter().cloned().collect();
        let failures = self.register(root, traversal);
        for dir in attempted {
            if self.watched.contains(&dir) {
                self.failed.remove(&dir);
            }
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
