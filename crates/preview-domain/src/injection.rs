//! Whether a response may carry the injected reload script (plan.md §6 steps 1 to 3; ADR 0022).
//!
//! The HTTP adapter translates the request and the resolved file into an [`InjectionInput`];
//! [`decide`] answers [`InjectionDecision::Inject`] only for a `200` GET of `text/html` that a
//! browser renders as a document, at most [`MAX_INJECTION_INPUT_BYTES`] long, whose bytes are
//! UTF-8. Everything else is served unchanged. Rewriting itself (where to insert, an already
//! present client script) belongs to the adapter's HTML rewriter; this module decides only
//! whether to try.
//!
//! # Fallback without Fetch Metadata
//!
//! A browser that sends `Sec-Fetch-Dest` tells navigations (`document`, `iframe`) apart from
//! script fetches (`empty`), and only the former are injected. A client that sends no
//! `Sec-Fetch-Dest` at all (an old browser, `curl`, an agent) is treated as a navigation and
//! injected, as a documented best-effort fallback (plan.md §6 step 2): such a client cannot be
//! told apart from a `fetch()` of HTML by an old browser, and the cost of a wrong guess is an
//! inert `<script>` tag in HTML that script reads as data. A header that is present but holds
//! an empty or unknown value is not treated as absent: the client speaks Fetch Metadata and
//! did not say "navigation", so nothing is injected. Pinned by
//! `should_inject_when_dest_missing_and_html_get` and
//! `should_skip_when_fetch_dest_is_unknown_or_blank`.

use core::fmt;

use crate::warning_code;

/// The largest HTML body the adapter rewrites: 8 MiB, inclusive (plan.md §6 step 3).
///
/// A larger page is served unchanged with a warning. Pinned by
/// `should_inject_when_html_is_exactly_eight_mebibytes` and
/// `should_skip_with_warning_when_html_exceeds_eight_mebibytes`.
pub const MAX_INJECTION_INPUT_BYTES: u64 = 8 * 1024 * 1024;

/// The request method. Other methods are refused with `405` before injection is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestMethod {
    /// `GET`: the body is sent and may be rewritten.
    Get,
    /// `HEAD`: no body, so nothing to rewrite (plan.md §5.1).
    Head,
}

/// What the served file is, as far as injection cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Media {
    /// `text/html`, the only media type that is injected.
    Html,
    /// Anything else: XHTML, SVG, XML, JSON, scripts, images, downloads.
    Other,
}

impl Media {
    /// Classifies a MIME type: [`Media::Html`] only when its essence (the part before any `;`,
    /// without surrounding whitespace, ASCII case-insensitive) is exactly `text/html`.
    ///
    /// `application/xhtml+xml` and `image/svg+xml` are XML documents, where an injected tag
    /// could make the document ill-formed, so they are [`Media::Other`]. Pinned by
    /// `should_skip_when_media_is_svg` and `should_classify_media_when_essence_varies`.
    #[must_use]
    pub fn from_essence(mime: &str) -> Self {
        let essence = mime.split(';').next().unwrap_or_default().trim();
        if essence.eq_ignore_ascii_case("text/html") {
            Self::Html
        } else {
            Self::Other
        }
    }
}

/// The value of a present `Sec-Fetch-Dest` request header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchDest {
    /// `document`: a top-level navigation.
    Document,
    /// `iframe`: a navigation inside an `<iframe>`.
    Iframe,
    /// `empty`: `fetch()`, `XMLHttpRequest` and other script-initiated requests.
    Empty,
    /// Any other value, including an empty one and the legacy `frame`.
    Other,
}

impl FetchDest {
    /// Parses a present header value. Tokens are matched exactly (Fetch Metadata values are
    /// lowercase); surrounding spaces and tabs are ignored. Pinned by
    /// `should_parse_fetch_dest_when_value_is_known_unknown_or_blank`.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim_matches([' ', '\t']) {
            "document" => Self::Document,
            "iframe" => Self::Iframe,
            "empty" => Self::Empty,
            _ => Self::Other,
        }
    }
}

/// Everything [`decide`] needs, translated by the HTTP adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InjectionInput {
    /// The request method.
    pub method: RequestMethod,
    /// True only for a `200 OK` response carrying the whole file; error and partial
    /// responses are never injected.
    pub status_ok: bool,
    /// The served media type.
    pub media: Media,
    /// `Sec-Fetch-Dest`, or `None` when the request has no such header.
    pub fetch_dest: Option<FetchDest>,
    /// The file size in bytes.
    pub size: u64,
    /// The body is valid UTF-8 and either starts with a UTF-8 byte order mark or any
    /// `<meta charset>` names UTF-8 (ADR 0022 and its amendments; see
    /// [`charset_label_is_utf8`]). Consulted last, after the size limit, so an adapter never
    /// needs to read an oversize body to fill it in.
    pub utf8_ok: bool,
}

/// Why a response is served without the reload script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// A `HEAD` request has no body.
    NotGet,
    /// The response is not a full `200`.
    NotOk,
    /// The media type is not `text/html`.
    NotHtml,
    /// `Sec-Fetch-Dest` is present and is neither `document` nor `iframe`.
    NotNavigation,
    /// The HTML is larger than [`MAX_INJECTION_INPUT_BYTES`]; warn once per path.
    TooLarge,
    /// The HTML is not UTF-8, or its `<meta charset>` disagrees; warn once per path.
    NotUtf8,
}

impl SkipReason {
    /// The warning to report once per path, for the reasons that mean "this page will not
    /// live-reload" rather than "this response was never meant to". Pinned by
    /// `should_warn_only_when_skip_reason_is_size_or_encoding`.
    #[must_use]
    pub const fn warning(self) -> Option<InjectionWarning> {
        match self {
            Self::TooLarge => Some(InjectionWarning::HtmlTooLarge),
            Self::NotUtf8 => Some(InjectionWarning::HtmlNotUtf8),
            Self::NotGet | Self::NotOk | Self::NotHtml | Self::NotNavigation => None,
        }
    }

    /// Whether the skip is reported as a warning.
    #[must_use]
    pub const fn warns(self) -> bool {
        self.warning().is_some()
    }
}

/// A warning about a page that is served without live reload (ADR 0020 codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InjectionWarning {
    /// The page exceeds the rewrite limit.
    HtmlTooLarge,
    /// The page is not UTF-8.
    HtmlNotUtf8,
}

impl InjectionWarning {
    /// The stable warning code printed as `warning[<code>]`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::HtmlTooLarge => warning_code::HTML_TOO_LARGE,
            Self::HtmlNotUtf8 => warning_code::HTML_NOT_UTF8,
        }
    }
}

impl fmt::Display for InjectionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HtmlTooLarge => "HTML is larger than 8 MiB; served without live reload",
            Self::HtmlNotUtf8 => "HTML is not UTF-8; served unchanged without live reload",
        })
    }
}

/// The verdict of [`decide`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InjectionDecision {
    /// Rewrite the body to carry the reload script.
    Inject,
    /// Serve the body unchanged.
    Skip(SkipReason),
}

/// Decides whether to inject, checking in this order and reporting the first rule that fails:
/// method, status, media type, fetch destination, size, encoding.
///
/// The order puts the size limit before the encoding so an oversize page is reported as
/// [`SkipReason::TooLarge`] whatever `utf8_ok` says. Pinned by the five task tests
/// (`should_skip_when_fetch_dest_is_empty`, `should_skip_when_method_is_head`,
/// `should_skip_with_warning_when_html_exceeds_eight_mebibytes`,
/// `should_inject_when_dest_missing_and_html_get`, `should_skip_when_media_is_svg`),
/// `should_report_first_failing_rule_when_several_fail` and
/// `should_inject_exactly_when_every_condition_holds`.
#[must_use]
pub const fn decide(input: &InjectionInput) -> InjectionDecision {
    let reason = if matches!(input.method, RequestMethod::Head) {
        SkipReason::NotGet
    } else if !input.status_ok {
        SkipReason::NotOk
    } else if matches!(input.media, Media::Other) {
        SkipReason::NotHtml
    } else if !matches!(
        input.fetch_dest,
        None | Some(FetchDest::Document | FetchDest::Iframe)
    ) {
        SkipReason::NotNavigation
    } else if input.size > MAX_INJECTION_INPUT_BYTES {
        SkipReason::TooLarge
    } else if !input.utf8_ok {
        SkipReason::NotUtf8
    } else {
        return InjectionDecision::Inject;
    };
    InjectionDecision::Skip(reason)
}

/// Whether a `<meta charset>` label names UTF-8 under the WHATWG Encoding Standard label
/// table: `utf-8`, `utf8`, `unicode-1-1-utf-8`, `unicode11utf8`, `unicode20utf8` or
/// `x-unicode20utf8`, ASCII case-insensitive, ignoring surrounding ASCII whitespace.
///
/// A page whose meta names another encoding is not injected even when its bytes happen to be
/// valid UTF-8 (ADR 0022). That includes the UTF-16 labels, which an HTML parser would
/// reinterpret as UTF-8: the declared intent disagrees, so the page is left alone. Pinned by
/// `should_accept_charset_label_when_it_names_utf8` and
/// `should_refuse_charset_label_when_it_names_another_encoding`.
#[must_use]
pub fn charset_label_is_utf8(label: &str) -> bool {
    const UTF8_LABELS: [&str; 6] = [
        "unicode-1-1-utf-8",
        "unicode11utf8",
        "unicode20utf8",
        "utf-8",
        "utf8",
        "x-unicode20utf8",
    ];
    let label = label.trim_matches(['\t', '\n', '\u{c}', '\r', ' ']);
    UTF8_LABELS.iter().any(|l| l.eq_ignore_ascii_case(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn html_get() -> InjectionInput {
        InjectionInput {
            method: RequestMethod::Get,
            status_ok: true,
            media: Media::Html,
            fetch_dest: Some(FetchDest::Document),
            size: 1_024,
            utf8_ok: true,
        }
    }

    #[test]
    fn should_skip_when_fetch_dest_is_empty() {
        // given
        let input = InjectionInput {
            fetch_dest: Some(FetchDest::parse("empty")),
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Skip(SkipReason::NotNavigation));
    }

    #[test]
    fn should_skip_when_method_is_head() {
        // given
        let input = InjectionInput {
            method: RequestMethod::Head,
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Skip(SkipReason::NotGet));
    }

    #[test]
    fn should_skip_with_warning_when_html_exceeds_eight_mebibytes() {
        // given
        let input = InjectionInput {
            size: MAX_INJECTION_INPUT_BYTES + 1,
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Skip(SkipReason::TooLarge));
        assert_eq!(
            SkipReason::TooLarge.warning().map(InjectionWarning::code),
            Some("html-too-large")
        );
    }

    #[test]
    fn should_inject_when_dest_missing_and_html_get() {
        // given
        let input = InjectionInput {
            fetch_dest: None,
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Inject);
    }

    #[test]
    fn should_skip_when_media_is_svg() {
        // given
        let input = InjectionInput {
            media: Media::from_essence("image/svg+xml"),
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Skip(SkipReason::NotHtml));
    }

    #[test]
    fn should_inject_when_dest_is_document_or_iframe() {
        // given
        let dests = [FetchDest::parse("document"), FetchDest::parse("iframe")];
        // when
        let decisions = dests.map(|d| {
            decide(&InjectionInput {
                fetch_dest: Some(d),
                ..html_get()
            })
        });
        // then
        assert_eq!(decisions, [InjectionDecision::Inject; 2]);
    }

    #[test]
    fn should_skip_when_fetch_dest_is_unknown_or_blank() {
        // given
        let values = [
            "",
            "  ",
            "frame",
            "script",
            "Document",
            "image",
            "nested-document",
        ];
        // when
        let decisions = values.map(|v| {
            decide(&InjectionInput {
                fetch_dest: Some(FetchDest::parse(v)),
                ..html_get()
            })
        });
        // then
        assert_eq!(
            decisions,
            [InjectionDecision::Skip(SkipReason::NotNavigation); 7]
        );
    }

    #[test]
    fn should_parse_fetch_dest_when_value_is_known_unknown_or_blank() {
        // given
        let values = ["document", " iframe\t", "empty", "", "DOCUMENT", "frame"];
        // when
        let parsed = values.map(FetchDest::parse);
        // then
        assert_eq!(
            parsed,
            [
                FetchDest::Document,
                FetchDest::Iframe,
                FetchDest::Empty,
                FetchDest::Other,
                FetchDest::Other,
                FetchDest::Other,
            ]
        );
    }

    #[test]
    fn should_skip_when_response_is_not_ok() {
        // given
        let input = InjectionInput {
            status_ok: false,
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Skip(SkipReason::NotOk));
    }

    #[test]
    fn should_classify_media_when_essence_varies() {
        // given
        let mimes = [
            "text/html",
            "text/html; charset=utf-8",
            " TEXT/HTML ;charset=x",
            "application/xhtml+xml",
            "image/svg+xml",
            "application/xml",
            "text/xml",
            "text/htmlx",
            "",
        ];
        // when
        let media = mimes.map(Media::from_essence);
        // then
        assert_eq!(
            media,
            [
                Media::Html,
                Media::Html,
                Media::Html,
                Media::Other,
                Media::Other,
                Media::Other,
                Media::Other,
                Media::Other,
                Media::Other,
            ]
        );
    }

    #[test]
    fn should_inject_when_html_is_exactly_eight_mebibytes() {
        // given
        let input = InjectionInput {
            size: 8_388_608,
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Inject);
    }

    #[test]
    fn should_skip_with_warning_when_html_is_not_utf8() {
        // given
        let input = InjectionInput {
            utf8_ok: false,
            ..html_get()
        };
        // when
        let decision = decide(&input);
        // then
        assert_eq!(decision, InjectionDecision::Skip(SkipReason::NotUtf8));
        assert_eq!(
            SkipReason::NotUtf8.warning().map(InjectionWarning::code),
            Some("html-not-utf8")
        );
    }

    #[test]
    fn should_report_first_failing_rule_when_several_fail() {
        // given
        let oversize_latin1 = InjectionInput {
            size: u64::MAX,
            utf8_ok: false,
            ..html_get()
        };
        let head_of_error = InjectionInput {
            method: RequestMethod::Head,
            status_ok: false,
            media: Media::Other,
            ..html_get()
        };
        let empty_dest_svg = InjectionInput {
            media: Media::Other,
            fetch_dest: Some(FetchDest::Empty),
            ..html_get()
        };
        // when
        let decisions = [
            decide(&oversize_latin1),
            decide(&head_of_error),
            decide(&empty_dest_svg),
        ];
        // then
        assert_eq!(
            decisions,
            [
                InjectionDecision::Skip(SkipReason::TooLarge),
                InjectionDecision::Skip(SkipReason::NotGet),
                InjectionDecision::Skip(SkipReason::NotHtml),
            ]
        );
    }

    #[test]
    fn should_warn_only_when_skip_reason_is_size_or_encoding() {
        // given
        let reasons = [
            SkipReason::NotGet,
            SkipReason::NotOk,
            SkipReason::NotHtml,
            SkipReason::NotNavigation,
            SkipReason::TooLarge,
            SkipReason::NotUtf8,
        ];
        // when
        let warns = reasons.map(SkipReason::warns);
        // then
        assert_eq!(warns, [false, false, false, false, true, true]);
    }

    #[test]
    fn should_accept_charset_label_when_it_names_utf8() {
        // given
        let labels = [
            "utf-8",
            "UTF-8",
            " utf8\n",
            "Unicode-1-1-UTF-8",
            "unicode11utf8",
            "unicode20utf8",
            "x-unicode20utf8",
        ];
        // when
        let accepted = labels.map(charset_label_is_utf8);
        // then
        assert_eq!(accepted, [true; 7]);
    }

    #[test]
    fn should_refuse_charset_label_when_it_names_another_encoding() {
        // given
        let labels = [
            "",
            "iso-8859-1",
            "windows-1252",
            "utf-16",
            "utf-16le",
            "utf-8x",
            "utf 8",
            "\u{a0}utf-8",
            "shift_jis",
        ];
        // when
        let accepted = labels.map(charset_label_is_utf8);
        // then
        assert_eq!(accepted, [false; 9]);
    }

    #[test]
    fn should_describe_warning_when_displayed() {
        // given
        let warning = InjectionWarning::HtmlTooLarge;
        // when
        let text = warning.to_string();
        // then
        assert!(text.contains("8 MiB"));
        assert!(InjectionWarning::HtmlNotUtf8.to_string().contains("UTF-8"));
    }

    fn any_input() -> impl Strategy<Value = InjectionInput> {
        let method = prop_oneof![Just(RequestMethod::Get), Just(RequestMethod::Head)];
        let media = prop_oneof![Just(Media::Html), Just(Media::Other)];
        let dest = prop_oneof![
            Just(None),
            Just(Some(FetchDest::Document)),
            Just(Some(FetchDest::Iframe)),
            Just(Some(FetchDest::Empty)),
            Just(Some(FetchDest::Other)),
        ];
        let size = prop_oneof![
            0..=MAX_INJECTION_INPUT_BYTES + 2,
            MAX_INJECTION_INPUT_BYTES - 1..=MAX_INJECTION_INPUT_BYTES + 1,
            any::<u64>(),
        ];
        (method, any::<bool>(), media, dest, size, any::<bool>()).prop_map(
            |(method, status_ok, media, fetch_dest, size, utf8_ok)| InjectionInput {
                method,
                status_ok,
                media,
                fetch_dest,
                size,
                utf8_ok,
            },
        )
    }

    proptest! {
        #[test]
        fn should_inject_exactly_when_every_condition_holds(input in any_input()) {
            // given
            let navigation = matches!(
                input.fetch_dest,
                None | Some(FetchDest::Document | FetchDest::Iframe)
            );
            let expected = input.method == RequestMethod::Get
                && input.status_ok
                && input.media == Media::Html
                && navigation
                && input.size <= MAX_INJECTION_INPUT_BYTES
                && input.utf8_ok;
            // when
            let decision = decide(&input);
            // then
            prop_assert_eq!(decision == InjectionDecision::Inject, expected);
            if let InjectionDecision::Skip(reason) = decision {
                prop_assert_eq!(
                    reason.warns(),
                    matches!(reason, SkipReason::TooLarge | SkipReason::NotUtf8)
                );
            }
        }
    }
}
