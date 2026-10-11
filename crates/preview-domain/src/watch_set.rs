//! The set of directories the watcher has registered, and its reconciliation against the set
//! that should be watched (plan.md §8.1 steps 5 and 6, "New directories"; ADR 0012).
//!
//! Pure: the adapter computes the desired set by traversal (using the shared eligibility
//! predicate), asks for the [`WatchDiff`], registers and unwatches accordingly, then applies
//! the diff to its record. The record is therefore the single answer to "is this directory
//! already watched", which avoids duplicate and stale registrations.

use std::collections::BTreeSet;

use crate::rel_path::RelPath;

/// What to change to move a [`WatchSet`] to a desired state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatchDiff {
    /// Directories to start watching.
    pub add: BTreeSet<RelPath>,
    /// Directories whose registration is stale: removed, renamed away or now ineligible.
    /// Includes every descendant of a removed directory that is no longer desired.
    pub remove: BTreeSet<RelPath>,
}

impl WatchDiff {
    /// Whether applying the diff changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

/// The directories currently registered with the watcher, root-relative.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatchSet(BTreeSet<RelPath>);

impl WatchSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `dir` is registered.
    #[must_use]
    pub fn contains(&self, dir: &RelPath) -> bool {
        self.0.contains(dir)
    }

    /// The number of registered directories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no directory is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The registered directories in order (a directory before its descendants).
    pub fn iter(&self) -> impl Iterator<Item = &RelPath> {
        self.0.iter()
    }

    /// The difference between this set and `desired`: what to add and what to remove so that
    /// the set equals `desired`. A directory present in both is untouched, so an unchanged
    /// subtree is never re-registered.
    ///
    /// Pinned by `should_add_only_new_directories_when_reconciling`,
    /// `should_remove_descendants_when_directory_removed` and
    /// `should_equal_desired_when_diff_is_applied`.
    #[must_use]
    pub fn reconcile(&self, desired: &BTreeSet<RelPath>) -> WatchDiff {
        WatchDiff {
            add: desired.difference(&self.0).cloned().collect(),
            remove: self.0.difference(desired).cloned().collect(),
        }
    }

    /// Applies a diff: removals first, then additions.
    pub fn apply(&mut self, diff: &WatchDiff) {
        for dir in &diff.remove {
            self.0.remove(dir);
        }
        self.0.extend(diff.add.iter().cloned());
    }

    /// The nearest strict ancestor of `path` that is registered, or `None` when not even the
    /// root is. Used after a removal: the parent is rescanned, and if it is gone too the walk
    /// continues upwards. Pinned by `should_find_ancestor_when_nearest_is_registered`.
    #[must_use]
    pub fn nearest_existing_ancestor(&self, path: &RelPath) -> Option<RelPath> {
        let mut current = path.parent();
        while let Some(candidate) = current {
            if self.0.contains(&candidate) {
                return Some(candidate);
            }
            current = candidate.parent();
        }
        None
    }
}

impl FromIterator<RelPath> for WatchSet {
    fn from_iter<I: IntoIterator<Item = RelPath>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Component;
    use proptest::prelude::*;

    fn rel(text: &str) -> RelPath {
        RelPath::new(
            text.split('/')
                .filter(|p| !p.is_empty())
                .map(|p| Component::new(p).expect("valid test component"))
                .collect(),
        )
    }

    fn set(dirs: &[&str]) -> BTreeSet<RelPath> {
        dirs.iter().map(|d| rel(d)).collect()
    }

    #[test]
    fn should_add_only_new_directories_when_reconciling() {
        // given
        let watched: WatchSet = set(&["", "css"]).into_iter().collect();
        let desired = set(&["", "css", "css/fonts", "img"]);

        // when
        let diff = watched.reconcile(&desired);

        // then
        assert_eq!(diff.add, set(&["css/fonts", "img"]));
        assert!(diff.remove.is_empty());
    }

    #[test]
    fn should_remove_descendants_when_directory_removed() {
        // given
        let watched: WatchSet = set(&["", "docs", "docs/a", "docs/a/b", "img"])
            .into_iter()
            .collect();
        let desired = set(&["", "img"]);

        // when
        let diff = watched.reconcile(&desired);

        // then
        assert_eq!(diff.remove, set(&["docs", "docs/a", "docs/a/b"]));
        assert!(diff.add.is_empty());
    }

    #[test]
    fn should_report_empty_diff_when_already_reconciled() {
        // given
        let watched: WatchSet = set(&["", "a"]).into_iter().collect();

        // when
        let diff = watched.reconcile(&set(&["", "a"]));

        // then
        assert!(diff.is_empty());
    }

    #[test]
    fn should_replace_subtree_when_directory_renamed() {
        // given
        let mut watched: WatchSet = set(&["", "old", "old/x"]).into_iter().collect();
        let desired = set(&["", "new", "new/x"]);

        // when
        let diff = watched.reconcile(&desired);
        watched.apply(&diff);

        // then
        assert_eq!(diff.add, set(&["new", "new/x"]));
        assert_eq!(diff.remove, set(&["old", "old/x"]));
        assert!(watched.contains(&rel("new/x")));
        assert!(!watched.contains(&rel("old")));
    }

    #[test]
    fn should_find_ancestor_when_nearest_is_registered() {
        // given
        let watched: WatchSet = set(&["", "a", "a/b"]).into_iter().collect();

        // when
        let parent = watched.nearest_existing_ancestor(&rel("a/b/c"));
        let skipped = watched.nearest_existing_ancestor(&rel("a/b/c/d/e"));
        let to_root = watched.nearest_existing_ancestor(&rel("z/y"));
        let of_root = watched.nearest_existing_ancestor(&RelPath::root());
        let of_registered = watched.nearest_existing_ancestor(&rel("a/b"));
        let none = WatchSet::new().nearest_existing_ancestor(&rel("a"));

        // then
        assert_eq!(parent, Some(rel("a/b")));
        assert_eq!(skipped, Some(rel("a/b")));
        assert_eq!(to_root, Some(RelPath::root()));
        assert_eq!(of_root, None);
        assert_eq!(of_registered, Some(rel("a")));
        assert_eq!(none, None);
    }

    fn dirs_strategy() -> impl Strategy<Value = BTreeSet<RelPath>> {
        proptest::collection::btree_set(
            proptest::collection::vec("[a-c]{1,2}", 0..4).prop_map(|parts| {
                RelPath::new(
                    parts
                        .iter()
                        .map(|p| Component::new(p).expect("valid"))
                        .collect(),
                )
            }),
            0..12,
        )
    }

    proptest! {
        #[test]
        fn should_equal_desired_when_diff_is_applied(
            current in dirs_strategy(),
            desired in dirs_strategy(),
        ) {
            // given
            let mut watched: WatchSet = current.into_iter().collect();

            // when
            let diff = watched.reconcile(&desired);
            watched.apply(&diff);

            // then
            prop_assert_eq!(watched.iter().cloned().collect::<BTreeSet<_>>(), desired.clone());
            prop_assert!(watched.reconcile(&desired).is_empty());
        }
    }
}
