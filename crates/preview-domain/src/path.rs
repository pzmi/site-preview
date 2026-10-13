//! Request-path decoding: the one function every served path passes through before any file
//! is opened (plan.md §5.2, ADR 0012).
//!
//! The output is a [`SitePath`]: a list of validated relative [`Component`]s beneath the site
//! root plus a trailing-slash flag. Nothing here touches the filesystem; whether a component is
//! *allowed* to be served is the eligibility predicate's job, and confinement below the root is
//! the per-component `openat` of the filesystem adapter (ADR 0004).

use std::fmt;

use percent_encoding::percent_decode;

/// Why a request path was refused.
///
/// The variant is coarse on purpose: a caller maps it to a status code and must not echo the
/// offending path back to the client or into a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathRefusal {
    /// The text is not a well-formed request path: not absolute, a `%` without two hex
    /// digits, a decoded NUL or other control character, invalid UTF-8 after decoding, or a
    /// literal `?` or `#` (the caller passes the path without query or fragment).
    Malformed,
    /// The path tries to change its own structure: a `.` or `..` component (literal or
    /// encoded), an encoded separator (`%2f`, `%5c`), a backslash, or an empty interior
    /// component (`a//b`).
    Traversal,
}

impl fmt::Display for PathRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed request path",
            Self::Traversal => "request path escapes its components",
        })
    }
}

impl std::error::Error for PathRefusal {}

/// One validated, decoded path component: a single directory-entry name.
///
/// Invariant: non-empty, valid UTF-8, not `.` or `..`, and free of `/`, `\` and control
/// characters (U+0000 to U+001F and U+007F). It is safe to pass to `openat` as one name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Component(String);

impl Component {
    /// Validates an already-decoded name, as the watcher receives it from the filesystem.
    ///
    /// # Errors
    ///
    /// [`PathRefusal::Traversal`] for an empty name, `.`, `..`, or a name containing `/` or
    /// `\`; [`PathRefusal::Malformed`] for a name containing a control character. Pinned by
    /// `should_refuse_component_when_name_is_not_a_single_entry`.
    pub fn new(name: &str) -> Result<Self, PathRefusal> {
        if name.chars().any(|c| c.is_ascii_control()) {
            return Err(PathRefusal::Malformed);
        }
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            return Err(PathRefusal::Traversal);
        }
        Ok(Self(name.to_owned()))
    }

    /// The decoded name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Component {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A decoded request path: validated components beneath the site root.
///
/// The fields are private so that a `SitePath` exists only as the output of
/// [`decode_request_path`] (or of validated components), never as unchecked text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SitePath {
    components: Vec<Component>,
    trailing_slash: bool,
}

impl SitePath {
    /// The components, outermost first. Empty for the site root.
    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    /// Whether the request path ended in `/` (a directory request). True for the site root
    /// `/`. Pinned by `should_keep_trailing_slash_when_path_ends_with_slash`.
    #[must_use]
    pub fn has_trailing_slash(&self) -> bool {
        self.trailing_slash
    }

    /// Whether this is the site root (`/`).
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.components.is_empty()
    }
}

/// Decodes the path of a request target into a [`SitePath`] (plan.md §5.2 rules 1 and 2).
///
/// `raw` is the path exactly as it arrived, without query or fragment, starting with `/`.
/// Each `/`-separated segment is percent-decoded **once** (`%252e` becomes the literal name
/// `%2e`, never `.`), then validated as a [`Component`]. Separators are recognised only before
/// decoding, so an encoded `/` or `\` can never split or join components; it is refused.
///
/// # Errors
///
/// [`PathRefusal`] as documented on its variants. The cases of plan.md §12 are pinned by the
/// `should_refuse_*` tests in this module, and the absence of `.`, `..`, separators and empty
/// names in any accepted output by
/// `should_yield_only_single_entry_components_when_decoding_any_short_input`.
pub fn decode_request_path(raw: &str) -> Result<SitePath, PathRefusal> {
    let Some(rest) = raw.strip_prefix('/') else {
        return Err(PathRefusal::Malformed);
    };
    if rest.contains(['?', '#']) {
        return Err(PathRefusal::Malformed);
    }
    if rest.is_empty() {
        return Ok(SitePath {
            components: Vec::new(),
            trailing_slash: true,
        });
    }
    let (body, trailing_slash) = match rest.strip_suffix('/') {
        Some(body) => (body, true),
        None => (rest, false),
    };
    let components = body
        .split('/')
        .map(decode_segment)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SitePath {
        components,
        trailing_slash,
    })
}

fn decode_segment(segment: &str) -> Result<Component, PathRefusal> {
    if segment.contains('\\') {
        return Err(PathRefusal::Traversal);
    }
    if !has_well_formed_escapes(segment.as_bytes()) {
        return Err(PathRefusal::Malformed);
    }
    let bytes: Vec<u8> = percent_decode(segment.as_bytes()).collect();
    let name = String::from_utf8(bytes).map_err(|_| PathRefusal::Malformed)?;
    Component::new(&name)
}

/// `percent_decode` passes a stray `%` through unchanged, so well-formedness is checked first:
/// every `%` must be followed by two hex digits.
fn has_well_formed_escapes(bytes: &[u8]) -> bool {
    let mut index = 0;
    while let Some(offset) = bytes[index..].iter().position(|&b| b == b'%') {
        let at = index + offset;
        match bytes.get(at + 1..at + 3) {
            Some([high, low]) if high.is_ascii_hexdigit() && low.is_ascii_hexdigit() => {
                index = at + 3;
            }
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(path: &SitePath) -> Vec<&str> {
        path.components().iter().map(Component::as_str).collect()
    }

    #[test]
    fn should_decode_root_when_path_is_single_slash() {
        // given
        let raw = "/";

        // when
        let path = decode_request_path(raw);

        // then
        let path = path.expect("root decodes");
        assert!(path.is_root());
        assert!(path.has_trailing_slash());
    }

    #[test]
    fn should_split_components_when_path_is_plain() {
        // given
        let raw = "/guide/intro.html";

        // when
        let path = decode_request_path(raw);

        // then
        let path = path.expect("plain path decodes");
        assert_eq!(names(&path), ["guide", "intro.html"]);
        assert!(!path.has_trailing_slash());
    }

    #[test]
    fn should_keep_trailing_slash_when_path_ends_with_slash() {
        // given
        let raw = "/guide/";

        // when
        let path = decode_request_path(raw);

        // then
        let path = path.expect("directory path decodes");
        assert_eq!(names(&path), ["guide"]);
        assert!(path.has_trailing_slash());
    }

    #[test]
    fn should_decode_space_when_path_contains_percent_20() {
        // given
        let raw = "/my%20page.html";

        // when
        let path = decode_request_path(raw);

        // then
        assert_eq!(names(&path.expect("space decodes")), ["my page.html"]);
    }

    #[test]
    fn should_decode_unicode_when_path_contains_encoded_utf8() {
        // given
        let raw = "/caf%C3%A9/men%c3%BC.html";

        // when
        let path = decode_request_path(raw);

        // then
        assert_eq!(
            names(&path.expect("unicode decodes")),
            ["caf\u{e9}", "men\u{fc}.html"]
        );
    }

    #[test]
    fn should_accept_unicode_when_path_contains_literal_utf8() {
        // given
        let raw = "/caf\u{e9}.html";

        // when
        let path = decode_request_path(raw);

        // then
        assert_eq!(names(&path.expect("literal unicode")), ["caf\u{e9}.html"]);
    }

    #[test]
    fn should_decode_once_when_path_is_double_encoded() {
        // given
        let raw = "/%252e%252e/%252f";

        // when
        let path = decode_request_path(raw);

        // then
        assert_eq!(names(&path.expect("decoded once")), ["%2e%2e", "%2f"]);
    }

    #[test]
    fn should_refuse_when_path_contains_encoded_dot_dot() {
        // given
        let raws = ["/%2e%2e/etc/passwd", "/a/%2E%2E", "/a/.%2e/b", "/%2e%2e/"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_encoded_dot() {
        // given
        let raws = ["/%2e/index.html", "/a/%2E"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_literal_dot_segments() {
        // given
        let raws = ["/..", "/../x", "/a/../b", "/./a", "/a/.", "/a/./"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_encoded_slash() {
        // given
        let raws = ["/a%2fb", "/a%2Fb", "/..%2f..%2fetc", "/%2f"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_encoded_backslash() {
        // given
        let raws = ["/a%5cb", "/..%5C..%5Cetc"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_literal_backslash() {
        // given
        let raws = ["/a\\b", "/..\\..\\etc", "/\\"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_has_empty_interior_component() {
        // given
        let raws = ["//", "//etc/passwd", "/a//b", "/a//"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_percent_escape_is_malformed() {
        // given
        let raws = ["/%", "/a%", "/a%2", "/%zz", "/%g0", "/%%32%65", "/%2"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_nul() {
        // given
        let raws = ["/a%00b", "/index.html%00.png", "/a\0b"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_contains_control_character() {
        // given
        let raws = ["/a%0ab", "/a%0D%0ASet-Cookie:x", "/a%7f", "/a\tb"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_decoded_bytes_are_invalid_utf8() {
        // given: a lone continuation byte, a truncated sequence, overlong `.` and `/`
        // encodings, and an encoded UTF-16 surrogate
        let raws = [
            "/%80",
            "/caf%C3",
            "/%C0%AE%C0%AE",
            "/%C0%AF",
            "/%ED%A0%80",
            "/%FF",
        ];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_is_not_absolute() {
        // given
        let raws = ["", "index.html", "%2findex.html", "http://example.test/"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{raw}");
        }
    }

    #[test]
    fn should_refuse_when_path_carries_query_or_fragment() {
        // given
        let raws = ["/a?b=c", "/a#top", "/?"];

        // when
        let results: Vec<_> = raws.iter().map(|raw| decode_request_path(raw)).collect();

        // then
        for (raw, result) in raws.iter().zip(results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{raw}");
        }
    }

    #[test]
    fn should_keep_encoded_question_mark_when_it_is_part_of_a_name() {
        // given
        let raw = "/what%3F.html";

        // when
        let path = decode_request_path(raw);

        // then
        assert_eq!(names(&path.expect("encoded ? is a name")), ["what?.html"]);
    }

    #[test]
    fn should_keep_dotted_names_when_they_are_not_dot_segments() {
        // given
        let raw = "/.../..a/a../.hidden";

        // when
        let path = decode_request_path(raw);

        // then: hidden names are the eligibility predicate's concern, not the decoder's
        assert_eq!(
            names(&path.expect("dotted names")),
            ["...", "..a", "a..", ".hidden"]
        );
    }

    #[test]
    fn should_refuse_component_when_name_is_not_a_single_entry() {
        // given
        let traversal = ["", ".", "..", "a/b", "a\\b"];
        let malformed = ["a\0b", "a\nb"];

        // when
        let traversal_results: Vec<_> = traversal.iter().map(|n| Component::new(n)).collect();
        let malformed_results: Vec<_> = malformed.iter().map(|n| Component::new(n)).collect();

        // then
        for (name, result) in traversal.iter().zip(traversal_results) {
            assert_eq!(result, Err(PathRefusal::Traversal), "{name:?}");
        }
        for (name, result) in malformed.iter().zip(malformed_results) {
            assert_eq!(result, Err(PathRefusal::Malformed), "{name:?}");
        }
        assert_eq!(Component::new("a b").map(|c| c.0), Ok("a b".to_owned()));
    }

    /// Every string of up to four symbols from an alphabet chosen to build dot segments,
    /// escapes (including `%2e`, `%2f`, `%5c`, `%25`) and separators, behind a leading `/`.
    fn short_inputs() -> Vec<String> {
        const ALPHABET: [&str; 10] = ["/", ".", "%", "2", "e", "f", "5", "c", "\\", "%25"];
        let mut inputs = vec![String::from("/")];
        let mut frontier = vec![String::from("/")];
        for _ in 0..4 {
            frontier = frontier
                .iter()
                .flat_map(|prefix| ALPHABET.iter().map(move |s| format!("{prefix}{s}")))
                .collect();
            inputs.extend(frontier.iter().cloned());
        }
        inputs
    }

    #[test]
    fn should_yield_only_single_entry_components_when_decoding_any_short_input() {
        // given
        let inputs = short_inputs();

        // when
        let decoded: Vec<_> = inputs
            .iter()
            .filter_map(|raw| decode_request_path(raw).ok().map(|path| (raw, path)))
            .collect();

        // then
        assert!(decoded.len() > 100, "the alphabet produces accepted paths");
        for (raw, path) in decoded {
            for component in path.components() {
                let name = component.as_str();
                assert!(!name.is_empty(), "{raw}");
                assert!(name != "." && name != "..", "{raw}");
                assert!(!name.contains(['/', '\\', '\0']), "{raw}");
            }
        }
    }

    #[test]
    fn should_round_trip_when_components_are_reencoded() {
        // given
        let accepted: Vec<SitePath> = short_inputs()
            .iter()
            .filter_map(|raw| decode_request_path(raw).ok())
            .collect();

        // when
        let reencoded: Vec<String> = accepted
            .iter()
            .map(|path| {
                let mut text = String::new();
                for component in path.components() {
                    text.push('/');
                    let encoded = percent_encoding::utf8_percent_encode(
                        component.as_str(),
                        percent_encoding::NON_ALPHANUMERIC,
                    );
                    text.push_str(&encoded.to_string());
                }
                if path.has_trailing_slash() {
                    text.push('/');
                }
                text
            })
            .collect();

        // then
        for (path, text) in accepted.iter().zip(&reencoded) {
            assert_eq!(decode_request_path(text).as_ref(), Ok(path), "{text}");
        }
    }

    #[test]
    fn should_tell_root_from_nested_when_path_is_decoded() {
        // given
        let root = decode_request_path("/").expect("decodes");
        let nested = decode_request_path("/a").expect("decodes");

        // when
        let verdicts = (root.is_root(), nested.is_root());

        // then
        assert_eq!(verdicts, (true, false));
    }

    #[test]
    fn should_give_the_name_when_component_is_viewed_as_a_str_and_refusals_display() {
        // given
        let component = Component::new("a.css").expect("valid");

        // when
        let viewed: &str = component.as_ref();

        // then
        assert_eq!(viewed, "a.css");
        assert_eq!(PathRefusal::Malformed.to_string(), "malformed request path");
        assert_eq!(
            PathRefusal::Traversal.to_string(),
            "request path escapes its components"
        );
    }
}
