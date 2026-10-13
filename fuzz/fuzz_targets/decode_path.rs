//! Fuzz target for `decode_request_path` and `eligibility::path` (plan.md §5.2, SEC-04).
//!
//! For arbitrary bytes (made UTF-8 lossily, because the function takes `&str` and the HTTP
//! layer hands it text) the decoder must never panic, and every path it accepts must hold the
//! properties the filesystem adapter relies on before it opens a name:
//!
//! 1. no component is empty, `.` or `..`, or contains `/`, `\` or an ASCII control character;
//! 2. `eligibility::path` gives exactly the verdict of [`oracle`], an independent
//!    implementation of the deny rules as `eligibility.rs` documents them, in both directions
//!    (eligible exactly when the oracle denies nothing) and with the same reason: a code point
//!    HFS+ ignores, the reserved root name, an excluded directory, an editor artifact or a
//!    leading `.`, each compared after the documented case fold (ASCII, LONG S as `s`, the
//!    dotted and dotless i as `i`), which the oracle implements itself rather than calling;
//! 3. decoding is deterministic, and re-encoding the components and decoding again gives the
//!    same path (one decode only: `%252e` stays the literal name `%2e`).

#![no_main]

use libfuzzer_sys::fuzz_target;
use preview_domain::eligibility::{self, DenyReason, Eligibility};
use preview_domain::{SitePath, decode_request_path};

fuzz_target!(|data: &[u8]| {
    let raw = String::from_utf8_lossy(data);
    let Ok(path) = decode_request_path(&raw) else {
        return;
    };
    assert_components_are_single_entries(&path);
    assert_eligibility_matches_the_oracle(&path);
    assert_decoding_is_stable(&raw, &path);
});

fn assert_components_are_single_entries(path: &SitePath) {
    for component in path.components() {
        let name = component.as_str();
        assert!(!name.is_empty(), "empty component");
        assert!(name != "." && name != "..", "dot component {name:?}");
        assert!(!name.contains(['/', '\\']), "separator in {name:?}");
        assert!(
            !name.chars().any(|c| c.is_ascii_control()),
            "control character in {name:?}"
        );
    }
    assert_eq!(path.is_root(), path.components().is_empty());
    if path.is_root() {
        assert!(path.has_trailing_slash(), "the root is a directory request");
    }
}

fn assert_eligibility_matches_the_oracle(path: &SitePath) {
    let verdict = eligibility::path(path);
    let expected = match oracle(path) {
        Some(reason) => Eligibility::Denied(reason),
        None => Eligibility::Eligible,
    };
    assert_eq!(
        verdict,
        expected,
        "eligibility::path disagrees with the oracle for {:?} (trailing slash: {})",
        path.components()
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        path.has_trailing_slash()
    );
}

/// The deny rules, written from their specification in `eligibility.rs` and not from its
/// code: the first component that is denied decides (every component but the last is a
/// directory, the last is one only with a trailing slash; the first is in the root), and
/// within a component the most specific reason wins. `None` means eligible.
fn oracle(path: &SitePath) -> Option<DenyReason> {
    let count = path.components().len();
    path.components()
        .iter()
        .enumerate()
        .find_map(|(index, component)| {
            let is_dir = index + 1 < count || path.has_trailing_slash();
            denied(component.as_str(), is_dir, index == 0)
        })
}

/// The code points HFS+ ignores in names, listed one by one: ZERO WIDTH SPACE, NON-JOINER and
/// JOINER, LEFT-TO-RIGHT and RIGHT-TO-LEFT MARK, the five embeddings and overrides, the six
/// deprecated format characters and ZERO WIDTH NO-BREAK SPACE.
const IGNORABLE: [char; 17] = [
    '\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}',
    '\u{202D}', '\u{202E}', '\u{206A}', '\u{206B}', '\u{206C}', '\u{206D}', '\u{206E}', '\u{206F}',
    '\u{FEFF}',
];

const EXCLUDED_DIRECTORIES: [&str; 4] = ["node_modules", "vendor", ".venv", "__pycache__"];

const RESERVED_ROOT_NAME: &str = "__site_preview";

fn denied(name: &str, is_dir: bool, in_root: bool) -> Option<DenyReason> {
    if name.chars().any(|c| IGNORABLE.contains(&c)) {
        return Some(DenyReason::IgnorableCodePoint);
    }
    let folded = case_fold(name);
    if in_root && folded == RESERVED_ROOT_NAME {
        return Some(DenyReason::Reserved);
    }
    if is_dir && EXCLUDED_DIRECTORIES.contains(&folded.as_str()) {
        return Some(DenyReason::ExcludedDirectory);
    }
    if is_editor_artifact(&folded) {
        return Some(DenyReason::EditorArtifact);
    }
    if folded.starts_with('.') {
        return Some(DenyReason::Hidden);
    }
    None
}

/// The case fold the domain documents: `A` to `Z` become `a` to `z`, LATIN SMALL LETTER LONG
/// S (U+017F) becomes `s`, LATIN CAPITAL LETTER I WITH DOT ABOVE (U+0130) and LATIN SMALL
/// LETTER DOTLESS I (U+0131) become `i`; every other character is kept as it is (so this is
/// not `str::to_lowercase`, which maps more characters and maps U+0130 to two).
fn case_fold(name: &str) -> String {
    const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
    const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    name.chars()
        .map(|c| match c {
            '\u{017F}' => 's',
            '\u{0130}' | '\u{0131}' => 'i',
            _ => UPPER
                .find(c)
                .and_then(|at| LOWER[at..].chars().next())
                .unwrap_or(c),
        })
        .collect()
}

/// `*~`, `.*.swp` and `.*.swo` (a dot, at least one more character, then the suffix), `#*#`
/// (at least two characters), `*.tmp`, `*.bak`, `*.orig`, `*.rej` and `4913`.
fn is_editor_artifact(folded: &str) -> bool {
    let swap = |suffix: &str| {
        folded
            .strip_suffix(suffix)
            .is_some_and(|stem| stem.starts_with('.'))
    };
    let hash_pair = folded.chars().count() >= 2 && folded.starts_with('#') && folded.ends_with('#');
    folded.ends_with('~')
        || swap(".swp")
        || swap(".swo")
        || hash_pair
        || folded.ends_with(".tmp")
        || folded.ends_with(".bak")
        || folded.ends_with(".orig")
        || folded.ends_with(".rej")
        || folded == "4913"
}

fn assert_decoding_is_stable(raw: &str, path: &SitePath) {
    assert_eq!(
        decode_request_path(raw).as_ref(),
        Ok(path),
        "decoding is not deterministic"
    );
    let rebuilt = re_encode(path);
    assert_eq!(
        decode_request_path(&rebuilt).as_ref(),
        Ok(path),
        "re-encoded path {rebuilt:?} decodes differently"
    );
}

/// The canonical request path of `path`: every byte outside the RFC 3986 unreserved set is
/// percent-encoded, so decoding it once must give back exactly the same components.
fn re_encode(path: &SitePath) -> String {
    let mut out = String::from("/");
    for (index, component) in path.components().iter().enumerate() {
        if index > 0 {
            out.push('/');
        }
        for byte in component.as_str().bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    if path.has_trailing_slash() && !path.is_root() {
        out.push('/');
    }
    out
}
