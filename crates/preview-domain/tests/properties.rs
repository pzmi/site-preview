//! Property tests over the public API of `preview-domain` (foundation.md section 5, "Property").
//!
//! Conventions every property test in this crate follows:
//!
//! - Name `should_<outcome>_when_<condition>`, with `// given`, `// when`, `// then` in the
//!   body, as for any other test (ARCH-test-name).
//! - Use [`config`] (or the default configuration, which behaves the same) so a failing case is
//!   written to `crates/preview-domain/proptest-regressions/<test file>.txt`, next to the
//!   sources. Those files are committed: a persisted seed is replayed first on every later run,
//!   so a found counterexample stays a regression test. `.gitignore` must not list the
//!   directory. A fix that makes a seed obsolete deletes that line, never the whole file by
//!   reflex.
//! - Do not set `cases` in code. CI raises the case count through `PROPTEST_CASES`, set to 1024
//!   by the `cargo xtask test` unit-tests stage (tools/xtask/src/test.rs); a developer can raise
//!   it the same way, and that stage keeps a value of 1024 or more (a lower one is raised to
//!   1024).
//! - Generate values with strategies that reach the interesting boundary (dots, escapes,
//!   separators) rather than arbitrary bytes only, and keep the exhaustive deterministic tests
//!   in the module as the floor.
//!
//! Properties that need private items live in their module's `tests`; they use the same
//! persistence and the same names.

use preview_domain::eligibility::{self, Eligibility};
use preview_domain::{Component, PathRefusal, SitePath, decode_request_path};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

/// The shared configuration: failures persist in `proptest-regressions/properties.txt` beside
/// `src/`. `Direct` rather than `SourceParallel`: the latter looks for a `lib.rs` or `main.rs`
/// next to the test file and silently persists nothing for a file under `tests/`. The path is
/// relative to the working directory, which cargo and nextest both set to the crate root.
fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/properties.txt",
        ))),
        ..ProptestConfig::default()
    }
}

/// Fragments that build dot segments, escapes (also double-encoded), separators and non-ASCII.
fn fragment() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "/", "/", ".", "..", "%", "2", "e", "f", "5", "c", "\\", "%25", "%2e", "%2E", "%2f", "%5c",
        "%00", "%0a", "%7f", "%c3%a9", "%ff", "a", "b", "~", " ", "é", "?",
    ])
}

/// A request-path candidate: a leading `/` then up to a dozen fragments.
fn request_path() -> impl Strategy<Value = String> {
    prop::collection::vec(fragment(), 0..12).prop_map(|parts| format!("/{}", parts.concat()))
}

/// A valid decoded component name, including dots inside and non-ASCII text.
fn component_name() -> impl Strategy<Value = Component> {
    "[^/\\\\\\p{Cc}]{1,12}".prop_filter_map("name must be a single entry", |name| {
        Component::new(&name).ok()
    })
}

/// Percent-encodes every byte except ASCII alphanumerics, `-`, `_` and `~`.
fn encode(name: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    out
}

fn assert_single_entry(path: &SitePath, raw: &str) -> Result<(), TestCaseError> {
    for component in path.components() {
        let name = component.as_str();
        prop_assert!(!name.is_empty(), "empty component from {raw:?}");
        prop_assert!(name != "." && name != "..", "dot component from {raw:?}");
        prop_assert!(
            !name.contains(['/', '\\', '\0']),
            "separator or NUL from {raw:?}"
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn should_yield_only_single_entry_components_when_decoding_any_request_path(
        raw in request_path(),
    ) {
        // given
        let raw = raw.as_str();

        // when
        let decoded = decode_request_path(raw);

        // then
        if let Ok(path) = decoded {
            assert_single_entry(&path, raw)?;
        }
    }

    #[test]
    fn should_not_panic_when_decoding_arbitrary_text(raw in any::<String>()) {
        // given
        let raw = raw.as_str();

        // when
        let decoded = decode_request_path(raw);

        // then
        if let Ok(path) = decoded {
            assert_single_entry(&path, raw)?;
        }
    }

    #[test]
    fn should_round_trip_when_components_are_percent_encoded(
        names in prop::collection::vec(component_name(), 1..5),
        trailing_slash in any::<bool>(),
    ) {
        // given
        let mut raw = String::new();
        for name in &names {
            raw.push('/');
            raw.push_str(&encode(name.as_str()));
        }
        if trailing_slash {
            raw.push('/');
        }

        // when
        let decoded = decode_request_path(&raw);

        // then
        let path = decoded.map_err(|refusal| TestCaseError::fail(format!("{raw:?}: {refusal}")))?;
        prop_assert_eq!(path.components(), names.as_slice());
        prop_assert_eq!(path.has_trailing_slash(), trailing_slash);
    }

    #[test]
    fn should_refuse_as_traversal_when_a_segment_is_a_dot_segment_in_any_encoding(
        before in prop::collection::vec(component_name(), 0..3),
        dots in prop::sample::select(vec![".", "..", "%2e", "%2E%2e", ".%2E", "%2e%2e"]),
    ) {
        // given
        let mut raw = String::new();
        for name in &before {
            raw.push('/');
            raw.push_str(&encode(name.as_str()));
        }
        raw.push('/');
        raw.push_str(dots);
        raw.push_str("/tail");

        // when
        let decoded = decode_request_path(&raw);

        // then
        prop_assert_eq!(decoded, Err(PathRefusal::Traversal));
    }

    #[test]
    fn should_have_no_hidden_component_when_path_is_eligible(
        names in prop::collection::vec("[.a-z~#]{1,6}", 0..5),
        trailing_slash in any::<bool>(),
    ) {
        // given
        let mut raw = format!("/{}", names.join("/"));
        if trailing_slash && !names.is_empty() {
            raw.push('/');
        }
        let Ok(path) = decode_request_path(&raw) else {
            return Ok(());
        };

        // when
        let verdict = eligibility::path(&path);

        // then
        let any_hidden = path.components().iter().any(|c| c.as_str().starts_with('.'));
        if verdict == Eligibility::Eligible {
            prop_assert!(!any_hidden, "eligible path {raw:?} has a hidden component");
        }
        if any_hidden {
            prop_assert!(verdict != Eligibility::Eligible, "hidden path {raw:?} was eligible");
        }
    }
}
