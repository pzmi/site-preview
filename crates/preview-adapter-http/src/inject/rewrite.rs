//! The `lol_html` pass that inserts the client script (plan.md §6 steps 4 to 6).
//!
//! Insertion point: immediately before the first `</body>` end tag the HTML tokenizer emits
//! for a `<body>` element, so a `</body>` inside a script, style, comment, attribute value,
//! `<title>` or `<textarea>` is never matched (`lol_html` tracks those text states). End-tag
//! names are case-insensitive, and only the first body end tag counts. A `</html>` that closes
//! the body implicitly is the insertion point too. A body end tag inside a closed `<template>`
//! is skipped, since a script there would be inert. When no body end tag exists (it is
//! optional in HTML), the tag is appended at document end, where a browser places it in the
//! body.
//!
//! A page that already loads the client (a `<script>` whose `src` is exactly
//! `/__site_preview/client.js` as written, say a saved copy of a served page) gets no second
//! tag. Its stale `data-instance` and `data-revision` would make the client reload on every
//! load, so each such tag has both attributes set to the snapshot (added when missing); only
//! those start tags change, and every other byte is kept.
//!
//! Known limits, none of which corrupts the page (plan.md §15): an unclosed `<template>`, a
//! `</body>` inside SVG `<style>` text, or a frameset document can receive the tag where it is
//! inert, and a page loaded as a frame (`Sec-Fetch-Dest: frame`) is not injected at all, so
//! none of these live reloads; an entity-encoded `src`, or one with a query string, is not
//! recognized as the client and gets a second one.
//!
//! Safety net: the rewriter runs in `lol_html`'s strict mode, which refuses markup it cannot
//! tokenize unambiguously without a full tree builder, and every result is checked before it
//! is returned. The output must be the input with exactly the tag inserted at one offset, and
//! a second parse of the output must see the tag as a `<script>` element exactly once. That
//! rejects an append at document end that would land inside an unterminated script, comment,
//! `<textarea>` or `<plaintext>`, where the tag would become text or change a script's source.
//! A failed check is [`Rewrite::Failed`]: the caller serves the original bytes and warns
//! (plan.md §6 step 6; ADR 0043). Only memory is touched; nothing is written back to a source
//! file.

use core::cell::{Cell, RefCell};
use core::fmt;
use core::ops::Range;
use std::rc::Rc;

use lol_html::errors::RewritingError;
use lol_html::html_content::{ContentType, Element, EndTag};
use lol_html::{
    AsciiCompatibleEncoding, HtmlRewriter, MemorySettings, Settings, element, end, end_tag,
};

use super::{CLIENT_SCRIPT_PATH, ScriptTag};

/// The outcome of [`rewrite`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewrite {
    /// The page with the script tag inserted once.
    Injected(Vec<u8>),
    /// The page already loads `/__site_preview/client.js`: no tag was added, and every client
    /// tag's `data-instance` and `data-revision` now hold the snapshot (added when missing).
    /// Byte for byte the page when they already did.
    AlreadyPresent(Vec<u8>),
    /// The page could not be rewritten safely; serve it unchanged and warn.
    Failed(RewriteFailure),
}

/// Why [`rewrite`] refused. The text names a cause, never page content or a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteFailure {
    /// The markup is ambiguous to a streaming tokenizer (`lol_html` strict mode).
    ParsingAmbiguity,
    /// The parser's buffers exceeded their memory limit.
    MemoryLimitExceeded,
    /// The parser reported another error.
    Parser,
    /// The inserted tag would not be parsed as a `<script>` element (it would land in text, a
    /// comment, a script's source or a `<plaintext>` tail).
    NotAnElement,
    /// The output was not the input plus one inserted tag.
    UnexpectedOutput,
}

impl fmt::Display for RewriteFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ParsingAmbiguity => "the markup is ambiguous to the streaming HTML parser",
            Self::MemoryLimitExceeded => "the HTML parser exceeded its memory limit",
            Self::Parser => "the HTML parser reported an error",
            Self::NotAnElement => "the end of the document is inside text, a comment or a script",
            Self::UnexpectedOutput => "the rewritten page differs from the original elsewhere",
        })
    }
}

impl From<RewritingError> for RewriteFailure {
    fn from(error: RewritingError) -> Self {
        match error {
            RewritingError::ParsingAmbiguity(_) => Self::ParsingAmbiguity,
            RewritingError::MemoryLimitExceeded(_) => Self::MemoryLimitExceeded,
            _ => Self::Parser,
        }
    }
}

/// Inserts `tag` into the UTF-8 HTML page `html`.
///
/// Pinned by the tests of this module:
/// `should_insert_before_real_body_end_when_script_contains_fake_body_tag`,
/// `should_insert_before_body_end_when_comment_contains_fake_body_tag`,
/// `should_insert_when_closing_tag_is_mixed_case`, `should_append_when_body_end_absent`,
/// `should_not_duplicate_when_client_already_present`,
/// `should_refresh_snapshot_attributes_when_client_tag_is_stale` and
/// `should_return_failed_when_parser_errors`. The caller bounds the input (8 MiB, HTTP-07)
/// and has already decided that the page is UTF-8 HTML to inject.
#[must_use]
pub fn rewrite(html: &[u8], tag: &ScriptTag) -> Rewrite {
    rewrite_with_csp(html, tag).0
}

/// [`rewrite`], and the `content` of each `<meta http-equiv="Content-Security-Policy">` in the
/// page's head (before the `<body>` start tag, outside a `<template>`), in document order, for
/// [`super::csp::detect`]. Read in the same pass; the meta elements are not touched. Empty
/// when the page could not be tokenized.
#[must_use]
pub fn rewrite_with_csp(html: &[u8], tag: &ScriptTag) -> (Rewrite, Vec<String>) {
    rewrite_with_memory_limit(html, tag, usize::MAX)
}

fn rewrite_with_memory_limit(
    html: &[u8],
    tag: &ScriptTag,
    memory_limit: usize,
) -> (Rewrite, Vec<String>) {
    let (inserted, policies) = match insert(html, tag, memory_limit) {
        Ok((Inserted::Output(output), policies)) => (output, policies),
        Ok((Inserted::ClientPresent, policies)) => {
            return match refresh(html, tag, memory_limit) {
                Ok(refreshed) => (Rewrite::AlreadyPresent(refreshed), policies),
                Err(failure) => (Rewrite::Failed(failure), Vec::new()),
            };
        }
        Err(failure) => return (Rewrite::Failed(failure), Vec::new()),
    };
    if !is_single_insertion(html, &inserted, tag.as_str().as_bytes()) {
        return (
            Rewrite::Failed(RewriteFailure::UnexpectedOutput),
            Vec::new(),
        );
    }
    match client_tags(&inserted, memory_limit).map(|tags| tags.len()) {
        Ok(1) => (Rewrite::Injected(inserted), policies),
        Ok(_) => (Rewrite::Failed(RewriteFailure::NotAnElement), Vec::new()),
        Err(failure) => (Rewrite::Failed(failure), Vec::new()),
    }
}

enum Inserted {
    Output(Vec<u8>),
    ClientPresent,
}

fn settings_base<'h>(memory_limit: usize) -> Settings<'h, 'static> {
    // Pinned explicitly rather than relying on defaults that a later release could change:
    // strict mode is what makes an ambiguous page fail instead of being mis-tokenized.
    Settings::new()
        .with_strict(true)
        .with_encoding(AsciiCompatibleEncoding::utf_8())
        .with_adjust_charset_on_meta_tag(false)
        .with_enable_esi_tags(false)
        .with_memory_settings(MemorySettings::new().with_max_allowed_memory_usage(memory_limit))
}

fn is_client_script(element: &Element<'_, '_>) -> bool {
    element.get_attribute("src").as_deref() == Some(CLIENT_SCRIPT_PATH)
}

fn insert(
    html: &[u8],
    tag: &ScriptTag,
    memory_limit: usize,
) -> Result<(Inserted, Vec<String>), RewriteFailure> {
    let markup: Rc<str> = Rc::from(tag.as_str());
    // End-tag handlers must be 'static, so state they share is reference counted.
    let injected = Rc::new(Cell::new(false));
    let template_depth = Rc::new(Cell::new(0_usize));
    let client_present = Cell::new(false);
    let body_seen = Cell::new(false);
    let policies = RefCell::new(Vec::new());
    let mut output = Vec::with_capacity(html.len() + markup.len());

    let settings = settings_base(memory_limit)
        .append_element_content_handler(element!("template", |el| {
            template_depth.set(template_depth.get() + 1);
            let depth = Rc::clone(&template_depth);
            el.on_end_tag(end_tag!(move |_end: &mut EndTag<'_>| {
                depth.set(depth.get().saturating_sub(1));
                Ok(())
            }))
        }))
        .append_element_content_handler(element!("meta[http-equiv]", |el| {
            let is_csp = el
                .get_attribute("http-equiv")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("content-security-policy"));
            if is_csp && !body_seen.get() && template_depth.get() == 0 {
                policies
                    .borrow_mut()
                    .push(el.get_attribute("content").unwrap_or_default());
            }
            Ok(())
        }))
        .append_element_content_handler(element!("body", |el| {
            body_seen.set(true);
            if injected.get() || template_depth.get() > 0 {
                return Ok(());
            }
            let injected = Rc::clone(&injected);
            let depth = Rc::clone(&template_depth);
            let markup = Rc::clone(&markup);
            el.on_end_tag(end_tag!(move |end: &mut EndTag<'_>| {
                if !injected.get() && depth.get() == 0 {
                    end.before(&markup, ContentType::Html);
                    injected.set(true);
                }
                Ok(())
            }))
        }))
        .append_element_content_handler(element!("script[src]", |el| {
            if is_client_script(el) {
                client_present.set(true);
            }
            Ok(())
        }))
        .append_document_content_handler(end!(|end| {
            if !injected.get() {
                end.append(&markup, ContentType::Html);
                injected.set(true);
            }
            Ok(())
        }));

    let mut rewriter = HtmlRewriter::new(settings, |chunk: &[u8]| {
        output.extend_from_slice(chunk);
    });
    rewriter.write(html)?;
    rewriter.end()?;

    let policies = policies.take();
    if client_present.get() {
        return Ok((Inserted::ClientPresent, policies));
    }
    Ok((Inserted::Output(output), policies))
}

/// Whether `output` is `input` with `tag` inserted at a single offset and nothing else changed.
fn is_single_insertion(input: &[u8], output: &[u8], tag: &[u8]) -> bool {
    if output.len() != input.len() + tag.len() {
        return false;
    }
    let prefix = input.iter().zip(output).take_while(|(a, b)| a == b).count();
    let suffix = input
        .iter()
        .rev()
        .zip(output.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    // `input[..k] == output[..k]` holds for every k <= prefix, and
    // `input[k..] == output[k + tag.len()..]` for every k >= input.len() - suffix.
    let lowest = input.len().saturating_sub(suffix);
    let highest = prefix.min(input.len());
    lowest <= highest && (lowest..=highest).any(|k| output.get(k..k + tag.len()) == Some(tag))
}

/// The attribute names carrying the snapshot (plan.md §6).
const DATA_INSTANCE: &str = "data-instance";
const DATA_REVISION: &str = "data-revision";

/// A `<script>` start tag loading the client, as the tokenizer saw it.
struct ClientTag {
    /// The start tag's bytes in the parsed document.
    range: Range<usize>,
    instance: Option<String>,
    revision: Option<String>,
}

/// The `<script>` elements loading the client that the tokenizer sees in `html`.
fn client_tags(html: &[u8], memory_limit: usize) -> Result<Vec<ClientTag>, RewriteFailure> {
    let tags = RefCell::new(Vec::new());
    let settings =
        settings_base(memory_limit).append_element_content_handler(element!("script[src]", |el| {
            if is_client_script(el) {
                tags.borrow_mut().push(ClientTag {
                    range: el.source_location().bytes(),
                    instance: el.get_attribute(DATA_INSTANCE),
                    revision: el.get_attribute(DATA_REVISION),
                });
            }
            Ok(())
        }));
    let mut parser = HtmlRewriter::new(settings, |_: &[u8]| {});
    parser.write(html)?;
    parser.end()?;
    Ok(tags.take())
}

/// `html`, which already loads the client, with every client tag's snapshot attributes set
/// to `tag`'s values.
///
/// Only an attribute that differs is set, so `lol_html` re-serializes just those start tags
/// (other attributes keep their raw bytes). Checked like an insertion: a second parse must see
/// the same number of client tags, each carrying the snapshot, and the bytes between and
/// around them must equal the input's.
fn refresh(html: &[u8], tag: &ScriptTag, memory_limit: usize) -> Result<Vec<u8>, RewriteFailure> {
    let instance = tag.instance_attribute();
    let revision = tag.revision_attribute();
    let ranges = RefCell::new(Vec::new());
    let mut output = Vec::with_capacity(html.len() + 2 * tag.as_str().len());
    let settings =
        settings_base(memory_limit).append_element_content_handler(element!("script[src]", |el| {
            if !is_client_script(el) {
                return Ok(());
            }
            ranges.borrow_mut().push(el.source_location().bytes());
            for (name, value) in [(DATA_INSTANCE, instance), (DATA_REVISION, revision)] {
                if el.get_attribute(name).as_deref() != Some(value) {
                    el.set_attribute(name, value)?;
                }
            }
            Ok(())
        }));
    let mut rewriter = HtmlRewriter::new(settings, |chunk: &[u8]| {
        output.extend_from_slice(chunk);
    });
    rewriter.write(html)?;
    rewriter.end()?;

    let before = ranges.take();
    let after = client_tags(&output, memory_limit)?;
    let refreshed = !before.is_empty()
        && after.len() == before.len()
        && after.iter().all(|found| {
            found.instance.as_deref() == Some(instance)
                && found.revision.as_deref() == Some(revision)
        });
    let after_ranges: Vec<Range<usize>> = after.into_iter().map(|found| found.range).collect();
    let unchanged_elsewhere = matches!(
        (outside(html, &before), outside(&output, &after_ranges)),
        (Some(input), Some(rewritten)) if input == rewritten
    );
    if !refreshed || !unchanged_elsewhere {
        return Err(RewriteFailure::UnexpectedOutput);
    }
    Ok(output)
}

/// The bytes of `html` before, between and after the ordered, disjoint `ranges`; `None` when a
/// range is out of order or out of bounds.
fn outside<'a>(html: &'a [u8], ranges: &[Range<usize>]) -> Option<Vec<&'a [u8]>> {
    let mut parts = Vec::with_capacity(ranges.len() + 1);
    let mut from = 0;
    for range in ranges {
        parts.push(html.get(from..range.start)?);
        from = range.end;
    }
    parts.push(html.get(from..)?);
    Some(parts)
}

#[cfg(test)]
mod tests {
    use preview_domain::{InstanceId, Revision};

    use super::*;

    fn tag() -> ScriptTag {
        ScriptTag::new(
            &InstanceId::parse("startup-abc").expect("valid instance id"),
            Revision::from(42),
        )
    }

    fn injected(html: &str) -> String {
        match rewrite(html.as_bytes(), &tag()) {
            Rewrite::Injected(bytes) => String::from_utf8(bytes).expect("output stays UTF-8"),
            other => panic!("expected Injected for {html:?}, got {other:?}"),
        }
    }

    /// `html` with the tag inserted at the first occurrence of `marker`.
    fn expected_before(html: &str, marker: &str) -> String {
        let at = html.find(marker).expect("marker present");
        format!("{}{}{}", &html[..at], tag().as_str(), &html[at..])
    }

    #[test]
    fn should_insert_before_real_body_end_when_script_contains_fake_body_tag() {
        // given
        let html = "<html><body><script>var s = \"</body>\"; // </body>\n</script>\
                    <p>x</p></body></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</body></html>"));
    }

    #[test]
    fn should_insert_before_body_end_when_comment_contains_fake_body_tag() {
        // given
        let html = "<html><body><!-- </body> --><p>x</p></body></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</body></html>"));
    }

    #[test]
    fn should_insert_before_body_end_when_fake_tags_sit_in_raw_text_and_attributes() {
        // given
        let html = "<html><head><title></body></title><style>p::after{content:\"</body>\"}\
                    </style></head><body><div title=\"</body>\">x</div>\
                    <textarea></body></textarea>&lt;/body&gt;</body></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</body></html>"));
    }

    #[test]
    fn should_insert_before_real_body_end_when_raw_text_elements_hold_fake_body_tag() {
        // given
        let cases = [
            "<html><body><noscript></body></noscript><p>x</p></body></html>",
            "<html><body><xmp></body></xmp></body></html>",
            "<html><body><iframe></body></iframe></body></html>",
            "<html><body><noembed></body></noembed></body></html>",
            "<html><body><noframes></body></noframes></body></html>",
            "<html><body><svg><![CDATA[</body>]]></svg></body></html>",
            "<html><body><script>a</scr</body>ipt></script></body></html>",
            "<html><body><script><!--<script></body></script>--></script></body></html>",
        ];

        for html in cases {
            // when
            let output = injected(html);

            // then
            assert_eq!(
                output,
                expected_before(html, "</body></html>"),
                "input {html:?}"
            );
        }
    }

    #[test]
    fn should_insert_when_closing_tag_is_mixed_case() {
        // given
        let html = "<HTML><BODY><p>x</p></BoDy></HTML>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</BoDy>"));
    }

    #[test]
    fn should_insert_once_before_first_body_end_when_body_end_repeats() {
        // given
        let html = "<html><body><p>a</p></body><p>b</p></body></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</body><p>b"));
        assert_eq!(output.matches(CLIENT_SCRIPT_PATH).count(), 1);
    }

    #[test]
    fn should_skip_template_body_end_when_template_holds_body_tags() {
        // given
        let html = "<html><body><template><body></body></template><p>x</p></body></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output.matches(CLIENT_SCRIPT_PATH).count(), 1);
        assert!(
            output.ends_with(&format!("{}</body></html>", tag().as_str()))
                || output.ends_with(tag().as_str()),
            "tag must not land inside the template: {output}"
        );
        assert!(!output.contains(&format!("{}</template>", tag().as_str())));
    }

    #[test]
    fn should_append_when_body_end_absent() {
        // given
        let cases = [
            "<!doctype html><title>t</title><p>x",
            "<html><body><p>x",
            "",
        ];

        for html in cases {
            // when
            let output = injected(html);

            // then
            assert_eq!(
                output,
                format!("{html}{}", tag().as_str()),
                "input {html:?}"
            );
        }
    }

    #[test]
    fn should_insert_before_html_end_when_it_closes_body_implicitly() {
        // given
        let html = "<html><body><p>x</p></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</html>"));
    }

    fn already_present(html: &str) -> String {
        match rewrite(html.as_bytes(), &tag()) {
            Rewrite::AlreadyPresent(bytes) => String::from_utf8(bytes).expect("output stays UTF-8"),
            other => panic!("expected AlreadyPresent for {html:?}, got {other:?}"),
        }
    }

    #[test]
    fn should_not_duplicate_when_client_already_present() {
        // given
        let cases = [
            (
                "<html><body><script src=\"/__site_preview/client.js\"></script></body></html>",
                "<html><body><script src=\"/__site_preview/client.js\" \
                 data-instance=\"startup-abc\" data-revision=\"42\"></script></body></html>",
            ),
            (
                "<html><head><script defer src='/__site_preview/client.js' data-revision=\"1\">\
                 </script></head><body></body></html>",
                "<html><head><script defer src='/__site_preview/client.js' data-revision=\"42\" \
                 data-instance=\"startup-abc\"></script></head><body></body></html>",
            ),
            (
                "<html><body></body></html><SCRIPT SRC=/__site_preview/client.js></SCRIPT>",
                "<html><body></body></html><SCRIPT SRC=/__site_preview/client.js \
                 data-instance=\"startup-abc\" data-revision=\"42\"></SCRIPT>",
            ),
        ];

        for (html, expected) in cases {
            // when
            let output = already_present(html);

            // then
            assert_eq!(output, expected, "input {html:?}");
            assert_eq!(output.matches(CLIENT_SCRIPT_PATH).count(), 1);
        }
    }

    #[test]
    fn should_refresh_snapshot_attributes_when_client_tag_is_stale() {
        // given
        // A served page saved to disk: its tag names an earlier run and revision.
        let before = "<!doctype html>\r\n<html><body><p>caf\u{e9}</p>\n";
        let after = "\n</body></html>\n";
        let stale = format!(
            "{before}<script defer src=\"/__site_preview/client.js\" \
             data-instance=\"startup-old\"\n  data-revision=\"7\"></script>{after}"
        );
        let current = format!("{before}{}{after}", tag().as_str());
        let twice =
            format!("{stale}<script src=\"/__site_preview/client.js\" data-revision=7></script>");

        // when
        let refreshed = already_present(&stale);
        let unchanged = already_present(&current);
        let both = already_present(&twice);

        // then
        assert_eq!(refreshed, current);
        assert_eq!(unchanged, current, "a current tag must keep its bytes");
        assert_eq!(
            both,
            format!(
                "{current}<script src=\"/__site_preview/client.js\" data-revision=\"42\" \
                 data-instance=\"startup-abc\"></script>"
            )
        );
    }

    #[test]
    fn should_inject_when_client_path_appears_only_as_text_or_comment() {
        // given
        let html = "<html><body><!-- <script src=\"/__site_preview/client.js\"></script> -->\
                    <script>var u = '/__site_preview/client.js';</script></body></html>";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</body></html>"));
    }

    #[test]
    fn should_return_failed_when_parser_errors() {
        // given
        // lol_html's documented ambiguous case: `<select><xmp>` cannot be tokenized without a
        // full tree builder, so strict mode refuses it.
        let ambiguous = "<html><body><select><xmp><script>\"use strict\";</script></select>\
                         </body></html>";
        // An unterminated start tag at the end of input must be buffered whole, so a small
        // memory limit is exceeded.
        let oversized = format!("<html><body><script src=\"{}", "x".repeat(64 * 1024));

        // when
        let ambiguous_outcome = rewrite(ambiguous.as_bytes(), &tag());
        let oversized_outcome = rewrite_with_memory_limit(oversized.as_bytes(), &tag(), 4 * 1024).0;

        // then
        assert_eq!(
            ambiguous_outcome,
            Rewrite::Failed(RewriteFailure::ParsingAmbiguity)
        );
        assert_eq!(
            oversized_outcome,
            Rewrite::Failed(RewriteFailure::MemoryLimitExceeded)
        );
    }

    #[test]
    fn should_return_failed_when_document_ends_inside_text_state() {
        // given
        let cases = [
            "<html><body><script>var a = 1;",
            "<html><body><p>x</p><!-- unterminated",
            "<html><body><textarea>draft",
            "<html><body><plaintext>raw",
            "<html><body><style>p{}",
            "<html><body><div title=\"x",
            "<html><body></",
        ];

        for html in cases {
            // when
            let outcome = rewrite(html.as_bytes(), &tag());

            // then
            assert_eq!(
                outcome,
                Rewrite::Failed(RewriteFailure::NotAnElement),
                "input {html:?}"
            );
        }
    }

    #[test]
    fn should_keep_every_other_byte_when_page_holds_bom_nul_and_multibyte_text() {
        // given
        let html = "\u{feff}<!DOCTYPE html>\r\n<html lang=\"pl\"><body>\0zażółć 🦀 &amp; &nbsp;\
                    <img src=x alt='a>b'></body></html>\r\n";

        // when
        let output = injected(html);

        // then
        assert_eq!(output, expected_before(html, "</body></html>"));
    }

    #[test]
    fn should_accept_only_single_insertion_when_output_compared() {
        // given
        let input = b"<p>ab</p>";
        let tag = b"<s>";

        // when
        let inserted = is_single_insertion(input, b"<p>a<s>b</p>", tag);
        let altered = is_single_insertion(input, b"<p>A<s>b</p>", tag);
        let longer = is_single_insertion(input, b"<p>a<s>b</p>!", tag);

        // then
        assert!(inserted);
        assert!(!altered);
        assert!(!longer);
    }

    #[test]
    fn should_collect_head_csp_metas_when_page_has_them() {
        // given
        let html = "<html><head><META http-equiv=' Content-Security-Policy ' content=\"a\">\
                    <meta http-equiv=\"refresh\" content=\"b\">\
                    <template><meta http-equiv=\"Content-Security-Policy\" content=\"t\"></template>\
                    <meta http-equiv=\"Content-Security-Policy\" content=\"c\"></head>\
                    <body><meta http-equiv=\"Content-Security-Policy\" content=\"d\"></body></html>";

        // when
        let (_, policies) = rewrite_with_csp(html.as_bytes(), &tag());

        // then
        assert_eq!(policies, ["a", "c"]);
    }
}
