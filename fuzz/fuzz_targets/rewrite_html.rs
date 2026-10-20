//! Fuzz target for the HTML rewriter `inject::rewrite` (plan.md §6 steps 4 to 6, SEC-05).
//!
//! The input is the page's raw bytes, passed as they are: the server hands the rewriter only
//! pages it classified as UTF-8, but the function takes bytes and must hold for any. The
//! rewriter must never panic, and:
//!
//! 1. on `Injected`, the output is the input with the tag inserted at one offset (its length
//!    is the input's plus the tag's, and removing the tag at that offset gives back the input
//!    bytes), and an independent parse of the output (the same strict `lol_html` settings,
//!    written here, not the rewriter's own check) sees exactly one client `<script>` element;
//! 2. on `AlreadyPresent`, the output still loads the client, and every client tag carries the
//!    snapshot's `data-instance` and `data-revision`;
//! 3. `Failed` is always allowed: the caller serves the original bytes.

#![no_main]

use std::cell::RefCell;

use libfuzzer_sys::fuzz_target;
use lol_html::{AsciiCompatibleEncoding, HtmlRewriter, MemorySettings, Settings, element};
use preview_adapter_http::inject::{CLIENT_SCRIPT_PATH, Rewrite, ScriptTag, rewrite};
use preview_domain::{InstanceId, Revision};

const INSTANCE: &str = "fuzz-instance";
const REVISION: u64 = 7;

fuzz_target!(|html: &[u8]| {
    let instance = InstanceId::parse(INSTANCE).expect("a valid instance id");
    let tag = ScriptTag::new(&instance, Revision::from(REVISION));
    match rewrite(html, &tag) {
        Rewrite::Injected(output) => {
            assert_single_insertion(html, &output, tag.as_str().as_bytes());
            let tags = client_tags(&output);
            assert_eq!(
                tags.len(),
                1,
                "injected output has {} client tags",
                tags.len()
            );
        }
        Rewrite::AlreadyPresent(output) => {
            let tags = client_tags(&output);
            assert!(
                !tags.is_empty(),
                "AlreadyPresent output lost the client tag"
            );
            let revision = REVISION.to_string();
            for (found_instance, found_revision) in tags {
                assert_eq!(
                    found_instance.as_deref(),
                    Some(INSTANCE),
                    "stale data-instance"
                );
                assert_eq!(
                    found_revision.as_deref(),
                    Some(revision.as_str()),
                    "stale data-revision"
                );
            }
        }
        Rewrite::Failed(_) => {}
    }
});

/// Asserts that `output` is `input` with `tag` inserted at exactly one offset.
fn assert_single_insertion(input: &[u8], output: &[u8], tag: &[u8]) {
    assert_eq!(
        output.len(),
        input.len() + tag.len(),
        "output is not the input plus one tag"
    );
    let removable = (0..=input.len()).any(|k| {
        output[k..k + tag.len()] == *tag
            && output[..k] == input[..k]
            && output[k + tag.len()..] == input[k..]
    });
    assert!(
        removable,
        "removing the tag does not give back the input bytes"
    );
}

/// The `data-instance` and `data-revision` of every `<script>` element whose `src` is the
/// client path, as a strict-mode tokenizer sees `html`.
///
/// The output was already tokenized by the rewriter with these settings, so a parse error here
/// is itself a finding.
fn client_tags(html: &[u8]) -> Vec<(Option<String>, Option<String>)> {
    let found = RefCell::new(Vec::new());
    let settings = Settings::new()
        // Match the production parser's full start-tag capture. In lol_html 3.0.1,
        // the fast scanner can mistake <noscript> inside <iframe/a=> for a real
        // start tag and miss the client after </iframe> (fuzz run 39389804492).
        // This no-op disables that shortcut without sharing the rewriter's check.
        .append_element_content_handler(element!("*", |_| Ok(())))
        .with_strict(true)
        .with_encoding(AsciiCompatibleEncoding::utf_8())
        .with_adjust_charset_on_meta_tag(false)
        .with_enable_esi_tags(false)
        .with_memory_settings(MemorySettings::new().with_max_allowed_memory_usage(usize::MAX))
        .append_element_content_handler(element!("script[src]", |el| {
            if el.get_attribute("src").as_deref() == Some(CLIENT_SCRIPT_PATH) {
                found.borrow_mut().push((
                    el.get_attribute("data-instance"),
                    el.get_attribute("data-revision"),
                ));
            }
            Ok(())
        }));
    let mut parser = HtmlRewriter::new(settings, |_: &[u8]| {});
    parser.write(html).expect("the rewriter's output tokenizes");
    parser.end().expect("the rewriter's output tokenizes");
    found.into_inner()
}
