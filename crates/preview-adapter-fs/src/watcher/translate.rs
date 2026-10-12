//! Translating a `notify` event into the domain's change events (plan.md section 8.2).
//!
//! The mapping says only what the operating system said; deciding relevance is the domain's
//! `classify`. The backends differ: Linux inotify reports a rename as `Name(From)` and
//! `Name(To)` and, when it pairs them, once more as `Name(Both)` with both paths; macOS FSEvents
//! reports `Name(Any)` once per side and coalesces flags, so a single path can carry several
//! kinds in a row. Every path of every rename becomes a `Rename`: a duplicate costs nothing
//! under the global debounce, a missing side would lose a change.
//!
//! | `notify` kind | [`ChangeKind`] |
//! |:--|:--|
//! | `Access(_)` | `AccessOnly` |
//! | `Create(Folder)`, `Remove(Folder)` | `DirectoryChanged` |
//! | `Create(File, Any, Other)` | `Create` |
//! | `Remove(File, Any, Other)` | `Remove` |
//! | `Modify(Data(_))` | `Modify` |
//! | `Modify(Metadata(_))` | `MetadataOnly` |
//! | `Modify(Name(_))` | `Rename`, one per path |
//! | `Modify(Any, Other)`, `Any`, `Other` | `Ambiguous` |
//! | any kind with the `Rescan` flag | one `Ambiguous` on the root |
//!
//! Paths are made root-relative component-wise (`/site2/x` is not below `/site`). A path
//! outside the root, or with a component that is not UTF-8 or not a valid [`Component`] (a name
//! that cannot be requested), is dropped. No `notify` type leaves the crate.

use std::path::{self, Path};

use notify::event::{CreateKind, ModifyKind, RemoveKind};
use notify::{Event, EventKind};
use preview_domain::{ChangeEvent, ChangeKind, Component, RelPath};

/// The change events `event` stands for, in its path order; empty when none of its paths is
/// inside `root`. Pinned by the table tests of this module.
#[must_use]
pub(crate) fn translate(event: Event, root: &Path) -> Vec<ChangeEvent> {
    if event.need_rescan() {
        return vec![ChangeEvent {
            kind: ChangeKind::Ambiguous,
            path: RelPath::root(),
        }];
    }
    let kind = change_kind(event.kind);
    event
        .paths
        .iter()
        .filter_map(|path| relative(path, root))
        .map(|path| ChangeEvent { kind, path })
        .collect()
}

fn change_kind(kind: EventKind) -> ChangeKind {
    match kind {
        EventKind::Access(_) => ChangeKind::AccessOnly,
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => {
            ChangeKind::DirectoryChanged
        }
        EventKind::Create(CreateKind::File | CreateKind::Any | CreateKind::Other) => {
            ChangeKind::Create
        }
        EventKind::Remove(RemoveKind::File | RemoveKind::Any | RemoveKind::Other) => {
            ChangeKind::Remove
        }
        EventKind::Modify(ModifyKind::Data(_)) => ChangeKind::Modify,
        EventKind::Modify(ModifyKind::Metadata(_)) => ChangeKind::MetadataOnly,
        EventKind::Modify(ModifyKind::Name(_)) => ChangeKind::Rename,
        EventKind::Modify(ModifyKind::Any | ModifyKind::Other)
        | EventKind::Any
        | EventKind::Other => ChangeKind::Ambiguous,
    }
}

/// `path` relative to `root`, or `None` when it is outside it or names something that cannot
/// be requested.
pub(crate) fn relative(path: &Path, root: &Path) -> Option<RelPath> {
    let rest = path.strip_prefix(root).ok()?;
    let mut components = Vec::new();
    for part in rest.components() {
        let path::Component::Normal(name) = part else {
            return None;
        };
        components.push(Component::new(name.to_str()?).ok()?);
    }
    Some(RelPath::new(components))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use notify::event::{AccessKind, AccessMode, DataChange, Flag, MetadataKind, RenameMode};

    use super::*;

    const ROOT: &str = "/srv/site";

    fn rel(text: &str) -> RelPath {
        RelPath::new(
            text.split('/')
                .filter(|part| !part.is_empty())
                .map(|part| Component::new(part).expect("valid component"))
                .collect(),
        )
    }

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        paths.iter().fold(Event::new(kind), |event, path| {
            event.add_path(PathBuf::from(path))
        })
    }

    fn change(kind: ChangeKind, path: &str) -> ChangeEvent {
        ChangeEvent {
            kind,
            path: rel(path),
        }
    }

    #[test]
    fn should_map_each_kind_when_translating_one_path() {
        // given
        let table = [
            (
                EventKind::Access(AccessKind::Open(AccessMode::Any)),
                ChangeKind::AccessOnly,
            ),
            (
                EventKind::Access(AccessKind::Close(AccessMode::Write)),
                ChangeKind::AccessOnly,
            ),
            (EventKind::Access(AccessKind::Any), ChangeKind::AccessOnly),
            (EventKind::Create(CreateKind::File), ChangeKind::Create),
            (EventKind::Create(CreateKind::Any), ChangeKind::Create),
            (EventKind::Create(CreateKind::Other), ChangeKind::Create),
            (
                EventKind::Create(CreateKind::Folder),
                ChangeKind::DirectoryChanged,
            ),
            (EventKind::Remove(RemoveKind::File), ChangeKind::Remove),
            (EventKind::Remove(RemoveKind::Any), ChangeKind::Remove),
            (EventKind::Remove(RemoveKind::Other), ChangeKind::Remove),
            (
                EventKind::Remove(RemoveKind::Folder),
                ChangeKind::DirectoryChanged,
            ),
            (
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                ChangeKind::Modify,
            ),
            (
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                ChangeKind::Modify,
            ),
            (
                EventKind::Modify(ModifyKind::Data(DataChange::Size)),
                ChangeKind::Modify,
            ),
            (
                EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
                ChangeKind::MetadataOnly,
            ),
            (
                EventKind::Modify(ModifyKind::Metadata(MetadataKind::Permissions)),
                ChangeKind::MetadataOnly,
            ),
            (
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                ChangeKind::Rename,
            ),
            (
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                ChangeKind::Rename,
            ),
            (
                EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
                ChangeKind::Rename,
            ),
            (EventKind::Modify(ModifyKind::Any), ChangeKind::Ambiguous),
            (EventKind::Modify(ModifyKind::Other), ChangeKind::Ambiguous),
            (EventKind::Any, ChangeKind::Ambiguous),
            (EventKind::Other, ChangeKind::Ambiguous),
        ];

        for (kind, expected) in table {
            // when
            let changes = translate(event(kind, &["/srv/site/docs/a.html"]), Path::new(ROOT));

            // then
            assert_eq!(changes, [change(expected, "docs/a.html")], "for {kind:?}");
        }
    }

    #[test]
    fn should_emit_two_events_when_rename_has_both_paths() {
        // given
        let renamed = event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/srv/site/a.tmp", "/srv/site/a.html"],
        );

        // when
        let changes = translate(renamed, Path::new(ROOT));

        // then
        assert_eq!(
            changes,
            [
                change(ChangeKind::Rename, "a.tmp"),
                change(ChangeKind::Rename, "a.html"),
            ]
        );
    }

    #[test]
    fn should_drop_event_when_path_outside_root() {
        // given
        let outside = [
            event(EventKind::Create(CreateKind::File), &["/srv/other/a.html"]),
            event(EventKind::Create(CreateKind::File), &["/srv/site2/a.html"]),
            event(EventKind::Create(CreateKind::File), &["/srv"]),
            event(EventKind::Create(CreateKind::File), &["relative/a.html"]),
        ];

        // when
        let changes: Vec<_> = outside
            .into_iter()
            .flat_map(|e| translate(e, Path::new(ROOT)))
            .collect();

        // then
        assert_eq!(changes, []);
    }

    #[test]
    fn should_keep_inside_side_only_when_rename_crosses_root() {
        // given
        let moved_in = event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/tmp/upload.part", "/srv/site/upload.png"],
        );

        // when
        let changes = translate(moved_in, Path::new(ROOT));

        // then
        assert_eq!(changes, [change(ChangeKind::Rename, "upload.png")]);
    }

    #[test]
    fn should_drop_path_when_component_is_not_utf8() {
        // given
        let mut name = PathBuf::from("/srv/site/docs");
        name.push(OsStr::from_bytes(b"bad-\xff.html"));
        let created = Event::new(EventKind::Create(CreateKind::File))
            .add_path(name)
            .add_path(PathBuf::from("/srv/site/good.html"));

        // when
        let changes = translate(created, Path::new(ROOT));

        // then
        assert_eq!(changes, [change(ChangeKind::Create, "good.html")]);
    }

    #[test]
    fn should_drop_path_when_it_holds_parent_component() {
        // given
        let escaped = event(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            &["/srv/site/../secret.txt"],
        );

        // when
        let changes = translate(escaped, Path::new(ROOT));

        // then
        assert_eq!(changes, []);
    }

    #[test]
    fn should_report_root_when_event_is_on_root_itself() {
        // given
        let removed = event(EventKind::Remove(RemoveKind::Any), &[ROOT]);

        // when
        let changes = translate(removed, Path::new(ROOT));

        // then
        assert_eq!(
            changes,
            [ChangeEvent {
                kind: ChangeKind::Remove,
                path: RelPath::root(),
            }]
        );
    }

    #[test]
    fn should_report_ambiguous_root_when_rescan_flag_is_set() {
        // given
        let overflowed = Event::new(EventKind::Other).set_flag(Flag::Rescan);

        // when
        let changes = translate(overflowed, Path::new(ROOT));

        // then
        assert_eq!(
            changes,
            [ChangeEvent {
                kind: ChangeKind::Ambiguous,
                path: RelPath::root(),
            }]
        );
    }

    #[test]
    fn should_translate_nothing_when_event_has_no_path() {
        // given
        let bare = Event::new(EventKind::Create(CreateKind::File));

        // when
        let changes = translate(bare, Path::new(ROOT));

        // then
        assert_eq!(changes, []);
    }
}
