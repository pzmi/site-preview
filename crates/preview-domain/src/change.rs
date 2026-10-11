//! Classification of one raw filesystem change (plan.md §8.2; ADR 0012).
//!
//! The adapter translates an OS notification into a [`ChangeEvent`]; this module decides
//! whether it may advance the revision. Ineligible paths are judged by the same predicate
//! that serving uses ([`crate::eligibility`]), so a `.git` or `node_modules` update can never
//! publish a revision.

use crate::eligibility::{self, ComponentKind, DenyReason, Eligibility};
use crate::rel_path::RelPath;

/// What the operating system said happened, as far as the adapter can tell.
///
/// Only `Modify` and `DirectoryChanged` say what kind of entry changed. `Create`, `Remove`
/// and `Rename` do not: a removed or renamed entry is gone before anyone can look, and an
/// OS may not report the type of a created one, so [`classify`] treats each as possibly a
/// file and possibly a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// An entry appeared; its type is unknown.
    Create,
    /// File data changed.
    Modify,
    /// An entry disappeared; its type is unknown.
    Remove,
    /// An entry was renamed (reported for each side the OS reports); its type is unknown.
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

/// What a relevant change touched, as far as the event and the entry's name tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relevance {
    /// File data changed (`Modify`): the path is recorded.
    File,
    /// An entry of unknown type (`Create`, `Remove`, `Rename`) whose name is eligible both as
    /// a file and as a directory: the path is recorded as a file, and the watch set is
    /// reconciled, because the entry may be a directory that appeared, vanished or moved.
    FileOrDirectory,
    /// A known directory changed (`DirectoryChanged`): a full reload without a path, and the
    /// watch set is reconciled.
    Directory,
    /// An entry of unknown type whose name is eligible only as a file (`vendor`,
    /// `node_modules`): if it was a file it was served, so the page reloads in full, but its
    /// path is never recorded, so the name of a possibly excluded directory never reaches a
    /// client (ADR 0039). An excluded directory is never watched, so nothing is reconciled.
    Undisclosed,
}

/// The verdict on one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Classification {
    /// The change may affect served content.
    Relevant(Relevance),
    /// The change is irrelevant and must not advance the revision.
    Ignored(IgnoreReason),
    /// The event is too coarse to attribute: rescan instead of inventing precision.
    Rescan,
}

impl Classification {
    /// Whether the directory structure may have changed, so the watch set must be reconciled
    /// before the revision settles (plan.md §8.1, §8.3): a directory change, an entry of
    /// unknown type with an eligible directory name, or an event too coarse to attribute.
    /// Pinned by `should_request_reconcile_when_entry_may_be_a_directory`.
    #[must_use]
    pub const fn requests_reconcile(self) -> bool {
        matches!(
            self,
            Self::Relevant(Relevance::Directory | Relevance::FileOrDirectory) | Self::Rescan
        )
    }
}

/// Classifies one event.
///
/// Order: an ineligible path is ignored first (so ambiguous `.git` churn never forces a
/// rescan), then access-only and metadata-only events, then ambiguous ones (`Rescan`).
/// `DirectoryChanged` is checked with its last component as a directory, `Modify` and the
/// ignorable or ambiguous kinds with it as a file. `Create`, `Remove` and `Rename` are of
/// unknown type and are checked both ways (ADR 0031, amended): denied as a file (which implies
/// denied as a directory) is ignored; denied only as a directory (`vendor` reported as
/// `Remove`) is [`Relevance::Undisclosed`]; eligible both ways is
/// [`Relevance::FileOrDirectory`]. Events on the root itself are relevant only as
/// `DirectoryChanged`; anything else there is `Rescan`.
///
/// Pinned by the `should_*` tests in this module.
#[must_use]
pub fn classify(event: &ChangeEvent) -> Classification {
    let names = event.path.components();
    let checked_as = if event.kind == ChangeKind::DirectoryChanged {
        ComponentKind::Dir
    } else {
        ComponentKind::File
    };
    if let Eligibility::Denied(reason) = eligibility::components(names, checked_as) {
        return Classification::Ignored(IgnoreReason::Ineligible(reason));
    }
    match event.kind {
        ChangeKind::AccessOnly => Classification::Ignored(IgnoreReason::AccessOnly),
        ChangeKind::MetadataOnly => Classification::Ignored(IgnoreReason::MetadataOnly),
        ChangeKind::Ambiguous => Classification::Rescan,
        ChangeKind::DirectoryChanged => Classification::Relevant(Relevance::Directory),
        _ if event.path.is_root() => Classification::Rescan,
        ChangeKind::Modify => Classification::Relevant(Relevance::File),
        ChangeKind::Create | ChangeKind::Remove | ChangeKind::Rename => {
            match eligibility::components(names, ComponentKind::Dir) {
                Eligibility::Eligible => Classification::Relevant(Relevance::FileOrDirectory),
                Eligibility::Denied(_) => Classification::Relevant(Relevance::Undisclosed),
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
                Classification::Relevant(Relevance::File),
            ]
        );
    }

    #[test]
    fn should_withhold_path_when_entry_of_unknown_type_is_named_like_excluded_directory() {
        // given
        let events = [
            event(ChangeKind::Remove, &["vendor"]),
            event(ChangeKind::Create, &["node_modules"]),
            event(ChangeKind::Rename, &["docs", "__pycache__"]),
            event(ChangeKind::Rename, &["lib", "VENDOR"]),
        ];

        // when
        let results = events.each_ref().map(classify);

        // then
        assert_eq!(
            results,
            [Classification::Relevant(Relevance::Undisclosed); 4]
        );
        assert!(results.iter().all(|c| !c.requests_reconcile()));
    }

    #[test]
    fn should_ignore_when_entry_of_unknown_type_is_denied_as_file() {
        // given
        let events = [
            event(ChangeKind::Remove, &[".venv"]),
            event(ChangeKind::Rename, &["docs", ".git"]),
            event(ChangeKind::Create, &["vendor", "lib.js"]),
        ];

        // when
        let results = events.each_ref().map(classify);

        // then
        assert_eq!(
            results,
            [
                Classification::Ignored(IgnoreReason::Ineligible(DenyReason::Hidden)),
                Classification::Ignored(IgnoreReason::Ineligible(DenyReason::Hidden)),
                Classification::Ignored(IgnoreReason::Ineligible(DenyReason::ExcludedDirectory)),
            ]
        );
    }

    #[test]
    fn should_request_reconcile_when_entry_may_be_a_directory() {
        // given
        let possibly_dirs = [
            event(ChangeKind::Create, &["blog"]),
            event(ChangeKind::Remove, &["assets", "img"]),
            event(ChangeKind::Rename, &["moved-in"]),
            event(ChangeKind::DirectoryChanged, &["docs"]),
            event(ChangeKind::Ambiguous, &["docs", "x"]),
        ];
        let files = [
            event(ChangeKind::Modify, &["index.html"]),
            event(ChangeKind::Remove, &["vendor"]),
            event(ChangeKind::Create, &[".swp"]),
        ];

        // when
        let reconcile = possibly_dirs
            .each_ref()
            .map(|e| classify(e).requests_reconcile());
        let no_reconcile = files.each_ref().map(|e| classify(e).requests_reconcile());

        // then
        assert_eq!(reconcile, [true; 5]);
        assert_eq!(no_reconcile, [false; 3]);
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
        assert_eq!(
            results,
            [
                Classification::Relevant(Relevance::File),
                Classification::Relevant(Relevance::FileOrDirectory),
                Classification::Relevant(Relevance::FileOrDirectory),
                Classification::Relevant(Relevance::FileOrDirectory),
            ]
        );
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
        assert_eq!(results, [Classification::Relevant(Relevance::Directory); 2]);
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
