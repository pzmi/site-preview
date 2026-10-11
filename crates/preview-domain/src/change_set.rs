//! The set of changed paths one debounced revision carries (ADR 0030, ADR 0031).
//!
//! Each relevant change is recorded as a root-relative URL path with an [`AssetKind`], so the
//! browser client can swap a stylesheet, cache-bust an asset or reload a script in place.
//! Anything the client cannot apply in place sets [`ChangeSet::is_full`], and a full set always
//! means a full-page reload: more than [`MAX_CHANGES`] paths, an overflow or rescan, a
//! directory change, an `html` or `other` path, or a removed `css`, `js` or `asset` path.
//!
//! Rejected alternatives: carrying `RelPath`s and encoding in the HTTP adapter (every consumer
//! would need the same encoding, and the domain already owns the decoder it must round-trip
//! through), and an unbounded set with the cap checked at send time (a burst of thousands of
//! changes would then be held in memory for nothing).

use std::collections::BTreeSet;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

use crate::change::{ChangeEvent, ChangeKind, Classification};
use crate::rel_path::RelPath;

/// The most paths a hot (non-full) change set carries; one more forces a full reload (ADR 0031).
pub const MAX_CHANGES: usize = 64;

/// What the browser can do with a changed path, by the final component's extension (ADR 0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    /// `.css`: swapped in place.
    Css,
    /// `.js`, `.mjs`: reloaded with scroll restore, or hot-accepted by the page.
    Js,
    /// `.html`, `.htm`: always a full reload.
    Html,
    /// Images, fonts and media: cache-busted in place.
    Asset,
    /// Anything else: always a full reload.
    Other,
}

const ASSET_EXTENSIONS: &[&str] = &[
    "png", "apng", "jpg", "jpeg", "gif", "webp", "avif", "svg", "ico", "bmp", "woff", "woff2",
    "ttf", "otf", "eot", "mp4", "webm", "ogv", "ogg", "oga", "mp3", "m4a", "aac", "wav", "flac",
    "opus",
];

impl AssetKind {
    /// The kind of a file named `name`, from the text after its last `.`, compared without
    /// regard to ASCII case (`SITE.CSS` is `Css`, as the MIME lookup treats it). A name with no
    /// extension, or whose only `.` is the first character, is `Other`.
    ///
    /// Pinned by `should_classify_kind_when_extension_is_known`.
    #[must_use]
    pub fn of_name(name: &str) -> Self {
        let Some((stem, extension)) = name.rsplit_once('.') else {
            return Self::Other;
        };
        if stem.is_empty() {
            return Self::Other;
        }
        let extension = extension.to_ascii_lowercase();
        match extension.as_str() {
            "css" => Self::Css,
            "js" | "mjs" => Self::Js,
            "html" | "htm" => Self::Html,
            other if ASSET_EXTENSIONS.contains(&other) => Self::Asset,
            _ => Self::Other,
        }
    }

    /// Whether the client can apply a change of this kind without reloading the document.
    #[must_use]
    pub const fn is_hot(self) -> bool {
        matches!(self, Self::Css | Self::Js | Self::Asset)
    }
}

/// The bytes a URL path component escapes: the WHATWG URL "path percent-encode set" (C0
/// controls, space, `"`, `#`, `<`, `>`, `?`, `^`, `` ` ``, `{`, `}`) plus `%`, with non-ASCII
/// always escaped as UTF-8, which is close to what a browser's `URL.pathname` yields for a
/// reference to the same file. Browsers differ on a few of these characters, so a client
/// matches on decoded paths rather than on this text. `%` is added because a literal `%` must
/// not be read back as an escape.
const PATH_COMPONENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'%');

/// One changed file as the browser addresses it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChangedPath {
    url_path: String,
    kind: AssetKind,
}

impl ChangedPath {
    /// The changed file at `path`. The URL path is `/` followed by each component
    /// percent-encoded and joined with `/` (`css/my site.css` gives `/css/my%20site.css`), and
    /// it decodes back to the same components through [`crate::decode_request_path`]. The
    /// root itself gives `/` with kind `Other`.
    ///
    /// Pinned by `should_percent_encode_url_path_when_name_has_space` and
    /// `should_round_trip_through_decoder_when_component_is_arbitrary`.
    #[must_use]
    pub fn new(path: &RelPath) -> Self {
        let mut url_path = String::new();
        for component in path.components() {
            url_path.push('/');
            url_path.extend(utf8_percent_encode(component.as_str(), PATH_COMPONENT));
        }
        if url_path.is_empty() {
            url_path.push('/');
        }
        let kind = path
            .components()
            .last()
            .map_or(AssetKind::Other, |name| AssetKind::of_name(name.as_str()));
        Self { url_path, kind }
    }

    /// The percent-encoded, root-relative URL path, starting with `/`.
    #[must_use]
    pub fn url_path(&self) -> &str {
        &self.url_path
    }

    /// The kind derived from the final component's extension.
    #[must_use]
    pub const fn kind(&self) -> AssetKind {
        self.kind
    }
}

/// Why a change set requires a full-page reload (ADR 0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullReason {
    /// More than [`MAX_CHANGES`] distinct paths changed.
    CapExceeded,
    /// An overflow or an unattributable event: the changed paths are unknown.
    Rescan,
    /// A directory was created, removed or renamed.
    DirectoryChanged,
    /// An `html` or `other` path changed.
    DocumentChanged,
    /// A `css`, `js` or `asset` path was removed; nothing can be swapped in its place.
    Removed,
}

/// The changes accumulated over one debounce window.
///
/// Invariant: a set that is not full holds at most [`MAX_CHANGES`] paths, all of a hot kind
/// (`css`, `js`, `asset`); a full set holds at most [`MAX_CHANGES`] paths of any kind, kept for
/// information only. Pinned by `should_be_hot_only_when_every_change_is_hot_and_within_cap`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    changes: BTreeSet<ChangedPath>,
    reason: Option<FullReason>,
}

impl ChangeSet {
    /// An empty, hot change set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            changes: BTreeSet::new(),
            reason: None,
        }
    }

    /// Adds one classified event.
    ///
    /// `Ignored` changes nothing. `Rescan` marks the set full ([`FullReason::Rescan`]). A
    /// relevant directory change marks it full ([`FullReason::DirectoryChanged`]) without
    /// adding a path, since a directory has no kind. A relevant file change adds its
    /// [`ChangedPath`] once, however often it repeats; an `html` or `other` kind marks the set
    /// full ([`FullReason::DocumentChanged`]), and so does a `Remove` of any other kind
    /// ([`FullReason::Removed`]). A path that would be the 65th distinct entry is not stored
    /// and marks the set full ([`FullReason::CapExceeded`]).
    ///
    /// A `Rename` is not treated as a removal: editors that save atomically rename a
    /// temporary file onto the target, and that must stay hot (pinned by
    /// `should_keep_hot_when_only_css_changes`).
    ///
    /// Pinned by the `should_*` tests in this module.
    pub fn accumulate(&mut self, event: &ChangeEvent, classification: Classification) {
        match classification {
            Classification::Ignored(_) => {}
            Classification::Rescan => self.mark_full(FullReason::Rescan),
            Classification::Relevant { dir_changed: true } => {
                self.mark_full(FullReason::DirectoryChanged);
            }
            Classification::Relevant { dir_changed: false } => self.add(&event.path, event.kind),
        }
    }

    fn add(&mut self, path: &RelPath, change: ChangeKind) {
        let changed = ChangedPath::new(path);
        if !changed.kind.is_hot() {
            self.mark_full(FullReason::DocumentChanged);
        } else if change == ChangeKind::Remove {
            self.mark_full(FullReason::Removed);
        }
        if self.changes.contains(&changed) {
            return;
        }
        if self.changes.len() >= MAX_CHANGES {
            self.mark_full(FullReason::CapExceeded);
            return;
        }
        self.changes.insert(changed);
    }

    /// Forces a full reload. The first reason recorded is kept; later ones do not replace it.
    pub fn mark_full(&mut self, reason: FullReason) {
        self.reason.get_or_insert(reason);
    }

    /// Returns the accumulated set and leaves this one empty and hot.
    #[must_use]
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    /// The distinct changed paths, ordered by URL path.
    #[must_use]
    pub const fn changes(&self) -> &BTreeSet<ChangedPath> {
        &self.changes
    }

    /// Whether the browser must reload the whole page.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.reason.is_some()
    }

    /// The first reason the set became full, if it is.
    #[must_use]
    pub const fn reason(&self) -> Option<FullReason> {
        self.reason
    }

    /// Whether nothing was recorded: no path and not full.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.reason.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::classify;
    use crate::path::{Component, decode_request_path};
    use proptest::prelude::*;

    fn rel(parts: &[&str]) -> RelPath {
        RelPath::new(
            parts
                .iter()
                .map(|p| Component::new(p).expect("valid test component"))
                .collect(),
        )
    }

    fn event(kind: ChangeKind, parts: &[&str]) -> ChangeEvent {
        ChangeEvent {
            kind,
            path: rel(parts),
        }
    }

    fn accumulate_all(events: &[ChangeEvent]) -> ChangeSet {
        let mut set = ChangeSet::new();
        for e in events {
            set.accumulate(e, classify(e));
        }
        set
    }

    fn paths(set: &ChangeSet) -> Vec<(&str, AssetKind)> {
        set.changes()
            .iter()
            .map(|c| (c.url_path(), c.kind()))
            .collect()
    }

    #[test]
    fn should_mark_full_when_html_changes() {
        // given
        let events = [
            event(ChangeKind::Modify, &["css", "site.css"]),
            event(ChangeKind::Modify, &["index.html"]),
        ];

        // when
        let set = accumulate_all(&events);

        // then
        assert!(set.is_full());
        assert_eq!(set.reason(), Some(FullReason::DocumentChanged));
        assert_eq!(
            paths(&set),
            [
                ("/css/site.css", AssetKind::Css),
                ("/index.html", AssetKind::Html)
            ]
        );
    }

    #[test]
    fn should_keep_hot_when_only_css_changes() {
        // given
        let events = [
            event(ChangeKind::Modify, &["css", "site.css"]),
            event(ChangeKind::Create, &["css", "print.css"]),
            event(ChangeKind::Rename, &["theme.css"]),
        ];

        // when
        let set = accumulate_all(&events);

        // then
        assert!(!set.is_full());
        assert_eq!(set.reason(), None);
        assert_eq!(
            paths(&set),
            [
                ("/css/print.css", AssetKind::Css),
                ("/css/site.css", AssetKind::Css),
                ("/theme.css", AssetKind::Css),
            ]
        );
    }

    #[test]
    fn should_mark_full_when_more_than_64_paths_change() {
        // given
        let names: Vec<String> = (0..=MAX_CHANGES).map(|i| format!("s{i}.css")).collect();
        let mut set = ChangeSet::new();
        for name in &names[..MAX_CHANGES] {
            let e = event(ChangeKind::Modify, &[name]);
            set.accumulate(&e, classify(&e));
        }
        let at_cap = set.clone();

        // when
        let e = event(ChangeKind::Modify, &[&names[MAX_CHANGES]]);
        set.accumulate(&e, classify(&e));

        // then
        assert!(!at_cap.is_full());
        assert_eq!(at_cap.changes().len(), MAX_CHANGES);
        assert_eq!(set.reason(), Some(FullReason::CapExceeded));
        assert_eq!(set.changes().len(), MAX_CHANGES);
    }

    #[test]
    fn should_mark_full_when_overflow_rescan() {
        // given
        let ambiguous = event(ChangeKind::Ambiguous, &["css", "site.css"]);
        let mut set = ChangeSet::new();

        // when
        set.accumulate(&ambiguous, classify(&ambiguous));

        // then
        assert_eq!(set.reason(), Some(FullReason::Rescan));
        assert!(set.changes().is_empty());
    }

    #[test]
    fn should_deduplicate_when_same_path_changes_twice() {
        // given
        let events = [
            event(ChangeKind::Modify, &["app.js"]),
            event(ChangeKind::Create, &["app.js"]),
            event(ChangeKind::Modify, &["app.js"]),
        ];

        // when
        let set = accumulate_all(&events);

        // then
        assert_eq!(paths(&set), [("/app.js", AssetKind::Js)]);
        assert!(!set.is_full());
    }

    #[test]
    fn should_percent_encode_url_path_when_name_has_space() {
        // given
        let path = rel(&["my css", "a b.css"]);

        // when
        let changed = ChangedPath::new(&path);

        // then
        assert_eq!(changed.url_path(), "/my%20css/a%20b.css");
        assert_eq!(changed.kind(), AssetKind::Css);
    }

    #[test]
    fn should_escape_reserved_and_non_ascii_when_building_url_path() {
        // given
        let path = rel(&["100%", "a?b#c^.png", "zdjęcie.JPG"]);

        // when
        let changed = ChangedPath::new(&path);

        // then
        assert_eq!(
            changed.url_path(),
            "/100%25/a%3Fb%23c%5E.png/zdj%C4%99cie.JPG"
        );
        assert_eq!(changed.kind(), AssetKind::Asset);
    }

    #[test]
    fn should_classify_kind_when_extension_is_known() {
        // given
        let names = [
            "a.css",
            "B.CSS",
            "a.js",
            "a.mjs",
            "a.html",
            "a.HTM",
            "a.png",
            "a.woff2",
            "a.mp4",
            "a.svg",
            "a.json",
            "a.wasm",
            "README",
            "archive.tar.gz",
            ".css",
            "a.",
        ];

        // when
        let kinds = names.map(AssetKind::of_name);

        // then
        assert_eq!(
            kinds,
            [
                AssetKind::Css,
                AssetKind::Css,
                AssetKind::Js,
                AssetKind::Js,
                AssetKind::Html,
                AssetKind::Html,
                AssetKind::Asset,
                AssetKind::Asset,
                AssetKind::Asset,
                AssetKind::Asset,
                AssetKind::Other,
                AssetKind::Other,
                AssetKind::Other,
                AssetKind::Other,
                AssetKind::Other,
                AssetKind::Other,
            ]
        );
    }

    #[test]
    fn should_mark_full_when_hot_path_is_removed() {
        // given
        let events = [
            event(ChangeKind::Modify, &["a.css"]),
            event(ChangeKind::Remove, &["img", "logo.png"]),
        ];

        // when
        let set = accumulate_all(&events);

        // then
        assert_eq!(set.reason(), Some(FullReason::Removed));
        assert_eq!(set.changes().len(), 2);
    }

    #[test]
    fn should_mark_full_without_path_when_directory_changes() {
        // given
        let dir = event(ChangeKind::DirectoryChanged, &["styles.css"]);

        // when
        let set = accumulate_all(&[dir]);

        // then
        assert_eq!(set.reason(), Some(FullReason::DirectoryChanged));
        assert!(set.changes().is_empty());
    }

    #[test]
    fn should_mark_full_when_other_kind_changes() {
        // given
        let data = event(ChangeKind::Modify, &["data", "track.gpx"]);

        // when
        let set = accumulate_all(&[data]);

        // then
        assert_eq!(set.reason(), Some(FullReason::DocumentChanged));
        assert_eq!(paths(&set), [("/data/track.gpx", AssetKind::Other)]);
    }

    #[test]
    fn should_change_nothing_when_event_is_ignored() {
        // given
        let events = [
            event(ChangeKind::Modify, &[".git", "index.html"]),
            event(ChangeKind::AccessOnly, &["index.html"]),
        ];

        // when
        let set = accumulate_all(&events);

        // then
        assert!(set.is_empty());
        assert_eq!(set, ChangeSet::new());
    }

    #[test]
    fn should_keep_first_reason_and_reset_when_taken() {
        // given
        let mut set = ChangeSet::new();
        set.mark_full(FullReason::Rescan);
        set.mark_full(FullReason::CapExceeded);
        let e = event(ChangeKind::Modify, &["a.css"]);
        set.accumulate(&e, classify(&e));

        // when
        let taken = set.take();

        // then
        assert_eq!(taken.reason(), Some(FullReason::Rescan));
        assert_eq!(taken.changes().len(), 1);
        assert!(set.is_empty());
    }

    #[test]
    fn should_give_root_url_when_path_is_root() {
        // given
        let root = RelPath::root();

        // when
        let changed = ChangedPath::new(&root);

        // then
        assert_eq!(changed.url_path(), "/");
        assert_eq!(changed.kind(), AssetKind::Other);
    }

    fn change_kind() -> impl Strategy<Value = ChangeKind> {
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

    fn file_name() -> impl Strategy<Value = String> {
        (
            "[a-z]{1,4}",
            prop_oneof![
                Just("css"),
                Just("js"),
                Just("mjs"),
                Just("png"),
                Just("woff2"),
                Just("html"),
                Just("json"),
                Just("txt"),
            ],
        )
            .prop_map(|(stem, ext)| format!("{stem}.{ext}"))
    }

    fn change_event() -> impl Strategy<Value = ChangeEvent> {
        (
            change_kind(),
            proptest::collection::vec("[a-z]{1,3}", 0..3),
            file_name(),
        )
            .prop_map(|(kind, dirs, name)| {
                let mut parts: Vec<&str> = dirs.iter().map(String::as_str).collect();
                parts.push(&name);
                event(kind, &parts)
            })
    }

    fn hot_event() -> impl Strategy<Value = ChangeEvent> {
        (
            proptest::collection::vec("[a-z]{1,3}", 0..2),
            "[a-z]{1,4}",
            prop_oneof![Just("css"), Just("js"), Just("png")],
        )
            .prop_map(|(dirs, stem, ext)| {
                let name = format!("{stem}.{ext}");
                let mut parts: Vec<&str> = dirs.iter().map(String::as_str).collect();
                parts.push(&name);
                event(ChangeKind::Modify, &parts)
            })
    }

    proptest! {
        #[test]
        fn should_be_hot_only_when_every_change_is_hot_and_within_cap(
            hot in proptest::collection::vec(hot_event(), 0..100),
            mixed in proptest::collection::vec(change_event(), 0..4),
            at in any::<prop::sample::Index>(),
        ) {
            // given
            let mut events = hot;
            let split = at.index(events.len() + 1);
            events.splice(split..split, mixed);
            let mut set = ChangeSet::new();

            // when
            for e in &events {
                set.accumulate(e, classify(e));
            }

            // then
            prop_assert!(set.changes().len() <= MAX_CHANGES);
            if !set.is_full() {
                prop_assert!(set.changes().iter().all(|c| c.kind().is_hot()));
                let removed_relevant = events.iter().any(|e| {
                    e.kind == ChangeKind::Remove
                        && matches!(classify(e), Classification::Relevant { .. })
                });
                prop_assert!(!removed_relevant);
            }
        }

        #[test]
        fn should_round_trip_through_decoder_when_component_is_arbitrary(
            names in proptest::collection::vec("[^/\\\\\\x00-\\x1f\\x7f]{1,8}", 1..4),
        ) {
            // given
            let components: Vec<Component> = names
                .iter()
                .filter_map(|n| Component::new(n).ok())
                .collect();
            prop_assume!(!components.is_empty());
            let path = RelPath::new(components.clone());

            // when
            let changed = ChangedPath::new(&path);
            let decoded = decode_request_path(changed.url_path());

            // then
            let decoded = decoded.expect("an emitted URL path always decodes");
            prop_assert_eq!(decoded.components(), components.as_slice());
            prop_assert!(!decoded.has_trailing_slash());
        }
    }
}
