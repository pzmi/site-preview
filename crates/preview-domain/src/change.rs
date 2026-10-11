//! Classification of one raw filesystem change (plan.md §8.2; ADR 0012).
//!
//! The adapter translates an OS notification into a [`ChangeEvent`]; this module decides
//! whether it may advance the revision. Ineligible paths are judged by the same predicate
//! that serving uses ([`crate::eligibility`]), so a `.git` or `node_modules` update can never
//! publish a revision.

use crate::eligibility::{self, ComponentKind, DenyReason, Eligibility};
use crate::rel_path::RelPath;

/// What the operating system said happened, as far as the adapter can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// An entry appeared.
    Create,
    /// File data changed.
    Modify,
    /// An entry disappeared.
    Remove,
    /// An entry was renamed (reported for each side the OS reports).
    Rename,
    /// The adapter knows the entry is a directory that appeared, vanished or was renamed.
    DirectoryChanged,
    /// Read or open without a write.
    AccessOnly,
    /// Permissions, timestamps or other attributes only.
    MetadataOnly,
    /// The OS gave a coarse or unclear event; the adapter will not guess.
    Ambiguous,
}

/// One raw change. `path` is the changed entry itself, root-relative; the empty path is the
/// site root directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangeEvent {
    /// What happened.
    pub kind: ChangeKind,
    /// Where, relative to the site root.
    pub path: RelPath,
}

/// Why an event was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IgnoreReason {
    /// The path is not eligible (hidden, excluded directory, editor temporary name or the
    /// reserved root name); editor swap and temp names arrive as
    /// `Ineligible(DenyReason::EditorArtifact)`.
    Ineligible(DenyReason),
    /// Access-only event.
    AccessOnly,
    /// Metadata-only event.
    MetadataOnly,
}

/// The verdict on one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Classification {
    /// The change may affect served content. `dir_changed` is true when the directory
    /// structure may have changed, so the watch set must be reconciled.
    Relevant {
        /// A directory was created, removed or renamed.
        dir_changed: bool,
    },
    /// The change is irrelevant and must not advance the revision.
    Ignored(IgnoreReason),
    /// The event is too coarse to attribute: rescan instead of inventing precision.
    Rescan,
}

/// Classifies one event.
///
/// Order: an ineligible path is ignored first (so ambiguous `.git` churn never forces a
/// rescan), then access-only and metadata-only events, then ambiguous ones (`Rescan`).
/// The entry's own kind is unknown to the predicate except for `DirectoryChanged`, which is
/// checked as a directory; every other kind is checked as a file, so a removed directory the
/// adapter could not identify (`vendor/` reported as `Remove`) errs towards a spurious
/// reload, never a missed one. Events on the root itself are relevant only as
/// `DirectoryChanged`; anything else there is `Rescan`.
///
/// Pinned by the `should_*` tests in this module.
#[must_use]
pub fn classify(event: &ChangeEvent) -> Classification {
    let last = if event.kind == ChangeKind::DirectoryChanged {
        ComponentKind::Dir
    } else {
        ComponentKind::File
    };
    if let Eligibility::Denied(reason) = eligibility::components(event.path.components(), last) {
        return Classification::Ignored(IgnoreReason::Ineligible(reason));
    }
    match event.kind {
        ChangeKind::AccessOnly => Classification::Ignored(IgnoreReason::AccessOnly),
        ChangeKind::MetadataOnly => Classification::Ignored(IgnoreReason::MetadataOnly),
        ChangeKind::Ambiguous => Classification::Rescan,
        ChangeKind::DirectoryChanged => Classification::Relevant { dir_changed: true },
        ChangeKind::Create | ChangeKind::Modify | ChangeKind::Remove | ChangeKind::Rename => {
            if event.path.is_root() {
                Classification::Rescan
            } else {
                Classification::Relevant { dir_changed: false }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Component;
    use proptest::prelude::*;

    fn event(kind: ChangeKind, parts: &[&str]) -> ChangeEvent {
        ChangeEvent {
            kind,
            path: RelPath::new(
                parts
                    .iter()
                    .map(|p| Component::new(p).expect("valid test component"))
                    .collect(),
            ),
        }
    }

    #[test]
    fn should_ignore_when_change_is_inside_dot_git() {
        // given
        let events = [
            event(ChangeKind::Modify, &[".git", "index"]),
            event(ChangeKind::Create, &["docs", ".git", "HEAD.lock"]),
            event(ChangeKind::DirectoryChanged, &[".git"]),
            event(ChangeKind::Ambiguous, &[".git", "objects"]),
        ];

        // when
        let results: Vec<_> = events.iter().map(classify).collect();

        // then
        for result in results {
            assert_eq!(
                result,
                Classification::Ignored(IgnoreReason::Ineligible(DenyReason::Hidden))
            );
        }
    }

    #[test]
    fn should_ignore_when_file_is_editor_swap() {
        // given
        let events = [
            event(ChangeKind::Create, &[".index.html.swp"]),
            event(ChangeKind::Modify, &["css", "site.css~"]),
            event(ChangeKind::Create, &["4913"]),
            event(ChangeKind::Rename, &["#index.html#"]),
        ];

        // when
        let results: Vec<_> = events.iter().map(classify).collect();

        // then
        for result in results {
            assert!(matches!(
                result,
                Classification::Ignored(IgnoreReason::Ineligible(_))
            ));
        }
    }

    #[test]
    fn should_ignore_when_change_is_inside_excluded_directory() {
        // given
        let inside = event(ChangeKind::Modify, &["node_modules", "x", "index.js"]);
        let vendor_file = event(ChangeKind::Modify, &["vendor"]);

        // when
        let results = [classify(&inside), classify(&vendor_file)];

        // then
        assert_eq!(
            results,
            [
                Classification::Ignored(IgnoreReason::Ineligible(DenyReason::ExcludedDirectory)),
                Classification::Relevant { dir_changed: false },
            ]
        );
    }

    #[test]
    fn should_be_relevant_when_html_is_modified() {
        // given
        let modified = event(ChangeKind::Modify, &["index.html"]);
        let created = event(ChangeKind::Create, &["css", "site.css"]);
        let removed = event(ChangeKind::Remove, &["a.js"]);
        let renamed = event(ChangeKind::Rename, &["a", "b.html"]);

        // when
        let results = [modified, created, removed, renamed].map(|e| classify(&e));

        // then
        for result in results {
            assert_eq!(result, Classification::Relevant { dir_changed: false });
        }
    }

    #[test]
    fn should_rescan_when_event_is_ambiguous() {
        // given
        let ambiguous = event(ChangeKind::Ambiguous, &["css", "site.css"]);
        let on_root = event(ChangeKind::Ambiguous, &[]);

        // when
        let results = [classify(&ambiguous), classify(&on_root)];

        // then
        assert_eq!(results, [Classification::Rescan; 2]);
    }

    #[test]
    fn should_flag_dir_change_when_directory_created() {
        // given
        let created = event(ChangeKind::DirectoryChanged, &["assets", "img"]);
        let root = event(ChangeKind::DirectoryChanged, &[]);

        // when
        let results = [classify(&created), classify(&root)];

        // then
        assert_eq!(results, [Classification::Relevant { dir_changed: true }; 2]);
    }

    #[test]
    fn should_ignore_when_event_is_access_or_metadata_only() {
        // given
        let access = event(ChangeKind::AccessOnly, &["index.html"]);
        let metadata = event(ChangeKind::MetadataOnly, &["index.html"]);

        // when
        let results = [classify(&access), classify(&metadata)];

        // then
        assert_eq!(
            results,
            [
                Classification::Ignored(IgnoreReason::AccessOnly),
                Classification::Ignored(IgnoreReason::MetadataOnly),
            ]
        );
    }

    #[test]
    fn should_rescan_when_non_directory_event_is_on_root() {
        // given
        let on_root = event(ChangeKind::Modify, &[]);

        // when
        let result = classify(&on_root);

        // then
        assert_eq!(result, Classification::Rescan);
    }

    #[test]
    fn should_ignore_when_directory_is_excluded_even_if_reported_as_directory() {
        // given
        let vendor_dir = event(ChangeKind::DirectoryChanged, &["lib", "vendor"]);

        // when
        let result = classify(&vendor_dir);

        // then
        assert_eq!(
            result,
            Classification::Ignored(IgnoreReason::Ineligible(DenyReason::ExcludedDirectory))
        );
    }

    fn kind_strategy() -> impl Strategy<Value = ChangeKind> {
        prop_oneof![
            Just(ChangeKind::Create),
            Just(ChangeKind::Modify),
            Just(ChangeKind::Remove),
            Just(ChangeKind::Rename),
            Just(ChangeKind::DirectoryChanged),
            Just(ChangeKind::AccessOnly),
            Just(ChangeKind::MetadataOnly),
            Just(ChangeKind::Ambiguous),
        ]
    }

    proptest! {
        #[test]
        fn should_never_publish_when_any_component_is_hidden(
            kind in kind_strategy(),
            before in proptest::collection::vec("[a-z]{1,6}", 0..3),
            hidden in "\\.[a-z]{1,6}",
            after in proptest::collection::vec("[a-z]{1,6}", 0..3),
        ) {
            // given
            let parts: Vec<&str> = before
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(hidden.as_str()))
                .chain(after.iter().map(String::as_str))
                .collect();

            // when
            let result = classify(&event(kind, &parts));

            // then
            prop_assert!(matches!(result, Classification::Ignored(_)));
        }
    }
}
