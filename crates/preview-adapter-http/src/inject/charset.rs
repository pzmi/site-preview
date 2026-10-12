//! Whether an HTML page is UTF-8 by its bytes and by its own `<meta>` declaration (ADR 0022).

use core::fmt;

use lol_html::{AsciiCompatibleEncoding, HtmlRewriter, Settings, element};
use preview_domain::injection::charset_label_is_utf8;

/// What a page's bytes and first charset declaration say about its encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    /// Valid UTF-8, and the first `<meta>` charset declaration (if any) names UTF-8.
    Utf8,
    /// The bytes are not valid UTF-8.
    InvalidBytes,
    /// Valid UTF-8 bytes, but the first `<meta>` declaration names another encoding (or one
    /// that could not be read), so a browser would decode the page differently.
    DeclaredOther,
}

impl Encoding {
    /// Whether the page may be labeled `charset=utf-8` and rewritten.
    pub(crate) const fn is_utf8(self) -> bool {
        matches!(self, Self::Utf8)
    }
}

/// Classifies `html`.
///
/// A declaration is the first `<meta charset>` element, or the first
/// `<meta http-equiv="content-type" content="…; charset=…">`, anywhere in the document the
/// tokenizer sees as markup (not in a comment or script text): a browser that finds one
/// past its 1024-byte prescan still switches encoding when it reaches it, so the whole page
/// is scanned. Pinned by `should_classify_encoding_when_meta_varies`.
pub(crate) fn classify(html: &[u8]) -> Encoding {
    if std::str::from_utf8(html).is_err() {
        return Encoding::InvalidBytes;
    }
    match declared_charset(html) {
        Declared::None => Encoding::Utf8,
        Declared::Label(label) if charset_label_is_utf8(&label) => Encoding::Utf8,
        Declared::Label(_) | Declared::Unreadable => Encoding::DeclaredOther,
    }
}

enum Declared {
    None,
    Label(String),
    Unreadable,
}

/// Raised by the `meta` handler to stop the scan at the first declaration.
#[derive(Debug)]
struct Found;

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("charset declaration found")
    }
}

impl core::error::Error for Found {}

fn declared_charset(html: &[u8]) -> Declared {
    let mut label = None;
    let settings = Settings::new()
        .with_strict(false)
        .with_encoding(AsciiCompatibleEncoding::utf_8())
        .with_adjust_charset_on_meta_tag(false)
        .with_enable_esi_tags(false)
        .append_element_content_handler(element!("meta", |el| {
            let declared = el.get_attribute("charset").or_else(|| {
                let http_equiv = el.get_attribute("http-equiv")?;
                if !http_equiv.eq_ignore_ascii_case("content-type") {
                    return None;
                }
                charset_from_content(&el.get_attribute("content")?).map(str::to_owned)
            });
            match declared {
                Some(declared) => {
                    label = Some(declared);
                    Err(Box::new(Found))
                }
                None => Ok(()),
            }
        }));
    let mut scanner = HtmlRewriter::new(settings, |_: &[u8]| {});
    let scanned = scanner.write(html).and_then(|()| scanner.end());
    match (label, scanned) {
        (Some(label), _) => Declared::Label(label),
        (None, Ok(())) => Declared::None,
        (None, Err(_)) => Declared::Unreadable,
    }
}

/// The WHATWG "extracting a character encoding from a meta element" algorithm over a
/// `content` attribute value: the value after the first `charset` (ASCII case-insensitive)
/// followed by optional whitespace and `=`, quoted or up to whitespace or `;`.
fn charset_from_content(content: &str) -> Option<&str> {
    const WS: [char; 5] = ['\t', '\n', '\u{c}', '\r', ' '];
    let lower = content.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let at = from + lower.get(from..)?.find("charset")?;
        let rest = content.get(at + "charset".len()..)?.trim_start_matches(WS);
        let Some(value) = rest.strip_prefix('=') else {
            from = content.len() - rest.len();
            continue;
        };
        let value = value.trim_start_matches(WS);
        return match value.chars().next()? {
            quote @ ('"' | '\'') => {
                let inner = &value[1..];
                inner.find(quote).map(|end| &inner[..end])
            }
            _ => {
                let end = value.find(|c| WS.contains(&c) || c == ';');
                Some(end.map_or(value, |end| &value[..end]))
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_classify_encoding_when_meta_varies() {
        // given
        let cases: [(&[u8], Encoding); 10] = [
            (b"<p>plain</p>", Encoding::Utf8),
            (
                "<meta charset=utf-8><p>caf\u{e9}</p>".as_bytes(),
                Encoding::Utf8,
            ),
            (b"<meta charset=\"UTF8\">", Encoding::Utf8),
            (
                b"<meta charset=shift_jis><p>ascii</p>",
                Encoding::DeclaredOther,
            ),
            (
                b"<meta http-equiv=Content-Type content=\"text/html; charset=windows-1252\">",
                Encoding::DeclaredOther,
            ),
            (
                b"<meta http-equiv=content-type content='text/html;charset=\"utf-8\"'>",
                Encoding::Utf8,
            ),
            (b"<!-- <meta charset=shift_jis> --><p>x</p>", Encoding::Utf8),
            (b"<script>'<meta charset=latin1>'</script>", Encoding::Utf8),
            (
                b"<meta charset=utf-8><meta charset=shift_jis>",
                Encoding::Utf8,
            ),
            (b"<p>caf\xe9</p>", Encoding::InvalidBytes),
        ];

        for (html, expected) in cases {
            // when
            let encoding = classify(html);

            // then
            assert_eq!(encoding, expected, "{}", String::from_utf8_lossy(html));
        }
    }

    #[test]
    fn should_extract_charset_when_content_attribute_varies() {
        // given
        let cases = [
            ("text/html; charset=utf-8", Some("utf-8")),
            ("text/html;CHARSET = 'koi8-r' ", Some("koi8-r")),
            ("text/html; charset=\"unterminated", None),
            ("text/html; charsetx; charset=big5", Some("big5")),
            ("text/html", None),
            ("charset=", None),
        ];

        for (content, expected) in cases {
            // when
            let charset = charset_from_content(content);

            // then
            assert_eq!(charset, expected, "{content}");
        }
    }
}
