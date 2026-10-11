//! The `Content-Type` of a served file, from its name (plan.md §5.1; ADR 0022).
//!
//! The lookup is by the last extension of the served file's own name, ASCII
//! case-insensitive: the overrides below first, then `mime_guess`, then
//! `application/octet-stream`. The overrides pin the types plan.md §5.1 names and the text
//! types that carry a charset, so a `mime_guess` upgrade cannot change them. Pinned by the
//! `should_map_content_type_when_extension_is_*` tests below.

use std::path::Path;

/// The type of a file whose extension nothing recognises.
const OCTET_STREAM: &str = "application/octet-stream";

/// Extensions whose type is fixed here rather than looked up. Lowercase.
const OVERRIDES: [(&str, &str); 7] = [
    ("css", "text/css; charset=utf-8"),
    ("geojson", "application/geo+json"),
    ("gpx", "application/gpx+xml"),
    ("js", "text/javascript; charset=utf-8"),
    ("kml", "application/vnd.google-earth.kml+xml"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("wasm", "application/wasm"),
];

/// What a file name says about its `Content-Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaType {
    /// `text/html`, whose charset depends on the bytes: see [`html_content_type`].
    Html,
    /// Any other type, final as given.
    Fixed(&'static str),
}

/// The media type of a file called `name`.
pub(crate) fn media_type(name: &str) -> MediaType {
    let Some(extension) = Path::new(name).extension().and_then(|e| e.to_str()) else {
        return MediaType::Fixed(OCTET_STREAM);
    };
    let extension = extension.to_ascii_lowercase();
    if let Some((_, fixed)) = OVERRIDES.iter().find(|(ext, _)| *ext == extension) {
        return MediaType::Fixed(fixed);
    }
    match mime_guess::from_ext(&extension).first_raw() {
        Some("text/html") => MediaType::Html,
        Some(guessed) => MediaType::Fixed(guessed),
        None => MediaType::Fixed(OCTET_STREAM),
    }
}

/// The `Content-Type` of an HTML file: `text/html; charset=utf-8` only when its bytes are
/// known to be UTF-8, else plain `text/html`, so a legacy-encoded page keeps the browser's
/// own detection (its `<meta charset>`) instead of being mislabeled (ADR 0022).
pub(crate) const fn html_content_type(utf8: bool) -> &'static str {
    if utf8 {
        "text/html; charset=utf-8"
    } else {
        "text/html"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_type(name: &str) -> &'static str {
        match media_type(name) {
            MediaType::Html => html_content_type(true),
            MediaType::Fixed(fixed) => fixed,
        }
    }

    #[test]
    fn should_map_content_type_when_extension_is_html() {
        // given
        let names = ["index.html", "old.htm"];

        // when
        let types = names.map(media_type);

        // then
        assert_eq!(types, [MediaType::Html, MediaType::Html]);
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
    }

    #[test]
    fn should_map_content_type_when_extension_is_css() {
        // given
        let name = "styles.css";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "text/css; charset=utf-8");
    }

    #[test]
    fn should_map_content_type_when_extension_is_js() {
        // given
        let names = ["app.js", "module.mjs"];

        // when
        let types = names.map(content_type);

        // then
        assert_eq!(types, ["text/javascript; charset=utf-8"; 2]);
    }

    #[test]
    fn should_map_content_type_when_extension_is_svg() {
        // given
        let name = "sample.svg";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "image/svg+xml");
    }

    #[test]
    fn should_map_content_type_when_extension_is_json() {
        // given
        let name = "data.json";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "application/json");
    }

    #[test]
    fn should_map_content_type_when_extension_is_gpx() {
        // given
        let name = "routes.gpx";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "application/gpx+xml");
    }

    #[test]
    fn should_map_content_type_when_extension_is_geojson() {
        // given
        let name = "areas.geojson";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "application/geo+json");
    }

    #[test]
    fn should_map_content_type_when_extension_is_kml() {
        // given
        let name = "places.kml";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "application/vnd.google-earth.kml+xml");
    }

    #[test]
    fn should_map_content_type_when_extension_is_wasm() {
        // given
        let name = "module.wasm";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "application/wasm");
    }

    #[test]
    fn should_map_content_type_when_extension_is_pdf() {
        // given
        let name = "manual.pdf";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "application/pdf");
    }

    #[test]
    fn should_map_content_type_when_extension_is_webp() {
        // given
        let name = "photo.webp";

        // when
        let content_type = content_type(name);

        // then
        assert_eq!(content_type, "image/webp");
    }

    #[test]
    fn should_map_content_type_when_extension_is_uppercase() {
        // given
        let names = ["INDEX.HTML", "Routes.GPX", "APP.Js"];

        // when
        let types = names.map(media_type);

        // then
        assert_eq!(
            types,
            [
                MediaType::Html,
                MediaType::Fixed("application/gpx+xml"),
                MediaType::Fixed("text/javascript; charset=utf-8"),
            ]
        );
    }

    #[test]
    fn should_map_octet_stream_when_extension_is_unknown_or_absent() {
        // given
        let names = ["blob.unknownext", "Makefile", "archive.", "notes.html.bak9"];

        // when
        let types = names.map(content_type);

        // then
        assert_eq!(types, [OCTET_STREAM; 4]);
    }

    #[test]
    fn should_omit_charset_when_html_is_not_known_utf8() {
        // given
        let utf8 = false;

        // when
        let content_type = html_content_type(utf8);

        // then
        assert_eq!(content_type, "text/html");
    }
}
