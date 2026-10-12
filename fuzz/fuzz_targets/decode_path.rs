//! Fuzz target for `decode_request_path` (plan.md §5.2, SEC-04).
//!
//! For arbitrary bytes (made UTF-8 lossily, because the function takes `&str` and the HTTP
//! layer hands it text) the decoder must never panic, and every path it accepts must hold the
//! properties the filesystem adapter relies on before it opens a name:
//!
//! 1. no component is empty, `.` or `..`, or contains `/`, `\` or an ASCII control character;
//! 2. `eligibility::path` agrees with the component walk `eligibility::components` and with an
//!    independent oracle of the deny rules (a leading `.`, the reserved root name, the excluded
//!    directories);
//! 3. decoding is deterministic, and re-encoding the components and decoding again gives the
//!    same path (one decode only: `%252e` stays the literal name `%2e`).

#![no_main]

use libfuzzer_sys::fuzz_target;
use preview_domain::eligibility::{self, ComponentKind, Eligibility};
use preview_domain::{SitePath, decode_request_path};

const EXCLUDED_DIRECTORIES: [&str; 4] = ["node_modules", "vendor", ".venv", "__pycache__"];

fuzz_target!(|data: &[u8]| {
    let raw = String::from_utf8_lossy(data);
    let Ok(path) = decode_request_path(&raw) else {
        return;
    };
    assert_components_are_single_entries(&path);
    assert_eligibility_is_consistent(&path);
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

fn assert_eligibility_is_consistent(path: &SitePath) {
    let verdict = eligibility::path(path);
    let last = if path.has_trailing_slash() {
        ComponentKind::Dir
    } else {
        ComponentKind::File
    };
    assert_eq!(
        verdict,
        eligibility::components(path.components(), last),
        "path and component walk disagree"
    );
    assert_eq!(verdict, eligibility::path(path), "verdict is not deterministic");
    if path.is_root() {
        assert_eq!(verdict, Eligibility::Eligible, "the root must be eligible");
    }
    if verdict != Eligibility::Eligible {
        return;
    }
    let count = path.components().len();
    for (index, component) in path.components().iter().enumerate() {
        let folded = component.as_str().to_lowercase();
        let is_dir = index + 1 < count || path.has_trailing_slash();
        assert!(!folded.starts_with('.'), "eligible hidden name {folded:?}");
        assert!(
            !(index == 0 && folded == "__site_preview"),
            "eligible reserved root name"
        );
        assert!(
            !(is_dir && EXCLUDED_DIRECTORIES.contains(&folded.as_str())),
            "eligible excluded directory {folded:?}"
        );
    }
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
