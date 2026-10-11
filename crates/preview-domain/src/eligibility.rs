//! The one eligibility predicate shared by serving and watching (plan.md §5.2 rules 3 to 5,
//! §8.1 step 2; ADR 0012).
//!
//! Rules match whole [`Component`]s, never substrings: `my-vendor-artwork` is eligible while a
//! directory named `vendor` is not. Names are compared after a case fold, because the default
//! macOS filesystem is case-insensitive: `openat(dir, "NODE_MODULES")` there opens
//! `node_modules`, so an exact byte comparison would let a request bypass the deny list.

use crate::path::{Component, SitePath};

/// Whether an entry is a directory or a file (anything that is not a directory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    /// A directory.
    Dir,
    /// A file.
    File,
}

/// Where an entry sits: directly in the site root, or deeper.
///
/// Some names are reserved only at the root (the internal `/__site_preview/` endpoints), so
/// the predicate needs the position as well as the name; the watcher passes `Root` for the
/// entries of the root directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// A direct child of the site root.
    Root,
    /// Any entry below a subdirectory of the root.
    Nested,
}

/// The verdict of the predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Eligibility {
    /// The entry may be served and watched.
    Eligible,
    /// The entry is neither served nor watched, nor descended into.
    Denied(DenyReason),
}

/// Why an entry is denied. When several rules match, the most specific reason is reported:
/// `Reserved`, then `ExcludedDirectory`, then `EditorArtifact`, then `Hidden`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyReason {
    /// The name starts with `.` (`.env`, `.git`, `.well-known`; plan.md §15).
    Hidden,
    /// A directory named exactly `node_modules`, `vendor`, `.venv` or `__pycache__`.
    ExcludedDirectory,
    /// An editor swap, backup or temporary name: `*~`, `.*.swp`, `.*.swo`, `#*#`, `*.tmp`,
    /// or `4913` (the file vim creates to test that a directory is writable).
    EditorArtifact,
    /// `__site_preview` directly in the root, where the internal endpoints live.
    Reserved,
}

const EXCLUDED_DIRECTORIES: [&str; 4] = ["node_modules", "vendor", ".venv", "__pycache__"];
const RESERVED_ROOT_NAME: &str = "__site_preview";

/// Decides whether one entry is eligible.
///
/// Pinned by the `should_*` tests in this module, one per rule and reason, including
/// `should_allow_component_when_name_only_contains_vendor_substring` and
/// `should_deny_when_excluded_directory_differs_only_in_case`.
#[must_use]
pub fn component(name: &Component, kind: ComponentKind, level: Level) -> Eligibility {
    let folded = fold(name.as_str());
    if level == Level::Root && folded == RESERVED_ROOT_NAME {
        return Eligibility::Denied(DenyReason::Reserved);
    }
    if kind == ComponentKind::Dir && EXCLUDED_DIRECTORIES.contains(&folded.as_str()) {
        return Eligibility::Denied(DenyReason::ExcludedDirectory);
    }
    if is_editor_artifact(&folded) {
        return Eligibility::Denied(DenyReason::EditorArtifact);
    }
    if folded.starts_with('.') {
        return Eligibility::Denied(DenyReason::Hidden);
    }
    Eligibility::Eligible
}

/// Decides whether a request path is eligible: every component but the last is checked as a
/// directory, and the last as a directory when the path ends in `/`, otherwise as a file.
/// The first verdict that denies wins; the root path `/` is eligible.
///
/// A request without a trailing slash whose last component turns out to be a directory must
/// be re-checked with [`component`] and [`ComponentKind::Dir`] before it is redirected, or
/// the redirect would reveal an excluded directory. Pinned by
/// `should_deny_when_any_component_is_hidden` and
/// `should_check_last_component_as_directory_when_path_has_trailing_slash`.
#[must_use]
pub fn path(path: &SitePath) -> Eligibility {
    let last = if path.has_trailing_slash() {
        ComponentKind::Dir
    } else {
        ComponentKind::File
    };
    components(path.components(), last)
}

/// Decides whether a root-relative sequence of components is eligible: every component but
/// the last is a directory, the last has the kind `last`. The first verdict that denies wins;
/// an empty sequence (the root) is eligible. This is the walk behind [`path`], shared with
/// the watcher so that watching applies exactly the rules serving does (ADR 0012). Pinned by
/// `should_deny_when_any_component_is_hidden` and
/// `should_check_last_as_given_kind_when_components_are_walked`.
#[must_use]
pub fn components(names: &[Component], last: ComponentKind) -> Eligibility {
    let last_index = names.len().saturating_sub(1);
    for (index, name) in names.iter().enumerate() {
        let kind = if index < last_index {
            ComponentKind::Dir
        } else {
            last
        };
        let level = if index == 0 {
            Level::Root
        } else {
            Level::Nested
        };
        let verdict = component(name, kind, level);
        if verdict != Eligibility::Eligible {
            return verdict;
        }
    }
    Eligibility::Eligible
}

fn is_editor_artifact(folded: &str) -> bool {
    let swap = |suffix: &str| {
        folded.starts_with('.') && folded.ends_with(suffix) && folded.len() > suffix.len()
    };
    folded.ends_with('~')
        || swap(".swp")
        || swap(".swo")
        || (folded.len() >= 2 && folded.starts_with('#') && folded.ends_with('#'))
        || folded.ends_with(".tmp")
        || folded == "4913"
}

/// Lowercases ASCII and maps the non-ASCII letters a case-insensitive filesystem may equate
/// with a letter of a rule name (LATIN SMALL LETTER LONG S with `s`; the dotted and dotless
/// i with `i`), so every such spelling of a rule name compares equal to it. Over-folding can
/// only deny more. Pinned by
/// `should_deny_when_excluded_directory_differs_only_in_case`.
fn fold(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\u{017F}' => 's',
            '\u{0130}' | '\u{0131}' => 'i',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::decode_request_path;

    fn name(text: &str) -> Component {
        Component::new(text).expect("valid test component")
    }

    fn site_path(raw: &str) -> SitePath {
        decode_request_path(raw).expect("valid test path")
    }

    fn verdicts(names: &[&str], kind: ComponentKind, level: Level) -> Vec<Eligibility> {
        names
            .iter()
            .map(|n| component(&name(n), kind, level))
            .collect()
    }

    #[test]
    fn should_allow_component_when_name_only_contains_vendor_substring() {
        // given
        let names = [
            "my-vendor-artwork",
            "vendors",
            "node_modules_backup",
            "venv",
        ];

        // when
        let results = verdicts(&names, ComponentKind::Dir, Level::Nested);

        // then
        for (n, result) in names.iter().zip(results) {
            assert_eq!(result, Eligibility::Eligible, "{n}");
        }
    }

    #[test]
    fn should_deny_when_directory_is_excluded() {
        // given
        let names = ["node_modules", "vendor", ".venv", "__pycache__"];

        // when
        let results = verdicts(&names, ComponentKind::Dir, Level::Nested);

        // then
        for (n, result) in names.iter().zip(results) {
            assert_eq!(
                result,
                Eligibility::Denied(DenyReason::ExcludedDirectory),
                "{n}"
            );
        }
    }

    #[test]
    fn should_allow_file_when_name_matches_excluded_directory() {
        // given
        let names = ["vendor", "node_modules", "__pycache__"];

        // when
        let results = verdicts(&names, ComponentKind::File, Level::Nested);

        // then
        for (n, result) in names.iter().zip(results) {
            assert_eq!(result, Eligibility::Eligible, "{n}");
        }
    }

    #[test]
    fn should_deny_when_excluded_directory_differs_only_in_case() {
        // given: ASCII case variants, LONG S for `s` and dotless i for `i`
        let names = [
            "NODE_MODULES",
            "Vendor",
            "__PYCACHE__",
            "node_module\u{17f}",
            ".VENV",
        ];

        // when
        let results = verdicts(&names, ComponentKind::Dir, Level::Nested);
        let reserved = verdicts(
            &["__Site_Preview", "__s\u{131}te_preview"],
            ComponentKind::Dir,
            Level::Root,
        );

        // then
        for (n, result) in names.iter().zip(results) {
            assert_eq!(
                result,
                Eligibility::Denied(DenyReason::ExcludedDirectory),
                "{n}"
            );
        }
        assert_eq!(reserved, [Eligibility::Denied(DenyReason::Reserved); 2]);
    }

    #[test]
    fn should_deny_when_name_is_hidden() {
        // given
        let names = [".env", ".git", ".well-known", "..x", ".DS_Store"];

        // when
        let files = verdicts(&names, ComponentKind::File, Level::Nested);
        let dirs = verdicts(&names, ComponentKind::Dir, Level::Root);

        // then
        for result in files.into_iter().chain(dirs) {
            assert_eq!(result, Eligibility::Denied(DenyReason::Hidden));
        }
    }

    #[test]
    fn should_deny_when_any_component_is_hidden() {
        // given
        let paths = [
            "/.well-known/x",
            "/docs/.git/config",
            "/a/b/.env",
            "/a/.hidden/",
        ];

        // when
        let results: Vec<_> = paths.iter().map(|p| path(&site_path(p))).collect();

        // then
        for (p, result) in paths.iter().zip(results) {
            assert_eq!(result, Eligibility::Denied(DenyReason::Hidden), "{p}");
        }
    }

    #[test]
    fn should_deny_when_file_ends_with_tilde() {
        // given
        let file = name("index.html~");

        // when
        let result = component(&file, ComponentKind::File, Level::Nested);

        // then
        assert_eq!(result, Eligibility::Denied(DenyReason::EditorArtifact));
    }

    #[test]
    fn should_deny_when_name_is_editor_artifact() {
        // given
        let names = [
            "..swp",
            ".index.html.swp",
            ".index.html.swo",
            ".x.SWP",
            "#index.html#",
            "##",
            "draft.tmp",
            "DRAFT.TMP",
            "4913",
            "notes~",
        ];

        // when
        let results = verdicts(&names, ComponentKind::File, Level::Nested);

        // then
        for (n, result) in names.iter().zip(results) {
            assert_eq!(
                result,
                Eligibility::Denied(DenyReason::EditorArtifact),
                "{n}"
            );
        }
    }

    #[test]
    fn should_allow_when_name_only_resembles_editor_artifact() {
        // given
        let names = [
            "#",
            "#hashtag",
            "page.swp.html",
            "x.swp",
            "tmp",
            "template.tmpl",
            "49130",
            "a~b",
        ];

        // when
        let results = verdicts(&names, ComponentKind::File, Level::Nested);

        // then
        for (n, result) in names.iter().zip(results) {
            assert_eq!(result, Eligibility::Eligible, "{n}");
        }
    }

    #[test]
    fn should_deny_when_root_component_is_reserved() {
        // given
        let paths = [
            "/__site_preview/client.js",
            "/__site_preview/",
            "/__site_preview",
        ];

        // when
        let results: Vec<_> = paths.iter().map(|p| path(&site_path(p))).collect();

        // then
        for (p, result) in paths.iter().zip(results) {
            assert_eq!(result, Eligibility::Denied(DenyReason::Reserved), "{p}");
        }
    }

    #[test]
    fn should_allow_reserved_name_when_component_is_nested() {
        // given
        let nested = site_path("/docs/__site_preview/page.html");

        // when
        let result = path(&nested);

        // then
        assert_eq!(result, Eligibility::Eligible);
    }

    #[test]
    fn should_check_last_component_as_directory_when_path_has_trailing_slash() {
        // given
        let as_directory = site_path("/lib/vendor/");
        let as_file = site_path("/lib/vendor");
        let interior = site_path("/vendor/lib.js");

        // when
        let results = [path(&as_directory), path(&as_file), path(&interior)];

        // then
        assert_eq!(
            results,
            [
                Eligibility::Denied(DenyReason::ExcludedDirectory),
                Eligibility::Eligible,
                Eligibility::Denied(DenyReason::ExcludedDirectory),
            ]
        );
    }

    #[test]
    fn should_check_last_as_given_kind_when_components_are_walked() {
        // given
        let names = [name("lib"), name("vendor")];

        // when
        let as_file = components(&names, ComponentKind::File);
        let as_dir = components(&names, ComponentKind::Dir);
        let root = components(&[], ComponentKind::Dir);

        // then
        assert_eq!(as_file, Eligibility::Eligible);
        assert_eq!(as_dir, Eligibility::Denied(DenyReason::ExcludedDirectory));
        assert_eq!(root, Eligibility::Eligible);
    }

    #[test]
    fn should_allow_path_when_every_component_is_eligible() {
        // given
        let paths = ["/", "/index.html", "/guide/", "/caf%C3%A9/my%20page.html"];

        // when
        let results: Vec<_> = paths.iter().map(|p| path(&site_path(p))).collect();

        // then
        for (p, result) in paths.iter().zip(results) {
            assert_eq!(result, Eligibility::Eligible, "{p}");
        }
    }
}
