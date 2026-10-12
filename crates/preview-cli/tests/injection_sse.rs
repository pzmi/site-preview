//! Black-box injection and live-reload stream suite (plan.md §12 "HTTP and security
//! integration tests", the M2 exit gate of §11).
//!
//! Every test runs the real binary on a [`Site`] and speaks raw HTTP to it. Deferred, because
//! the watcher is not wired into the binary yet (CLI-03): a `reload` event after a file
//! change, the 15 s keepalive on a real clock and the stream ending on shutdown. Their
//! payloads are pinned at component level in `preview-adapter-http`'s `tests/sse.rs`; the
//! end-to-end versions belong to E2E-03 (watcher) and E2E-09 (hot reload).

pub mod support;

use std::fs;

use preview_testkit::poll::wait_until;
use support::{DEADLINE, EventStream, PreviewProcess, Reply, Site, tree_hash};

const DOCUMENT: [(&str, &str); 1] = [("Sec-Fetch-Dest", "document")];
const EVENTS: &str = "/__site_preview/events";
const CLIENT: &str = "/__site_preview/client.js";
const MIB: usize = 1024 * 1024;

/// The injected element: from the `<script` that loads the client to its closing tag.
fn injected_tag(page: &str) -> Option<&str> {
    let at = page.find(CLIENT)?;
    let start = page[..at].rfind("<script")?;
    let end = at + page[at..].find("</script>")? + "</script>".len();
    Some(&page[start..end])
}

fn warnings(lines: &[String], code: &str) -> Vec<String> {
    let prefix = format!("site-preview: warning[{code}]: ");
    lines
        .iter()
        .filter(|line| line.starts_with(&prefix))
        .cloned()
        .collect()
}

fn refused(result: Result<EventStream, Reply>) -> Reply {
    match result {
        Err(reply) => reply,
        Ok(_) => panic!("the stream opened, but a refusal was expected"),
    }
}

fn opened(result: Result<EventStream, Reply>) -> EventStream {
    match result {
        Ok(stream) => stream,
        Err(reply) => panic!("the stream was refused: {reply:?}"),
    }
}

#[test]
fn should_inject_once_and_match_content_length_when_page_is_navigated() {
    // given
    let site = Site::copy();
    let original = fs::read_to_string(site.path().join("index.html")).expect("the fixture page");
    let server = PreviewProcess::serve(&site);

    // when
    let page = server.request("GET", "/", &DOCUMENT);

    // then
    let text = page.text();
    assert_eq!(page.status, 200);
    assert_eq!(text.matches(CLIENT).count(), 1, "{text}");
    assert_eq!(
        page.header("content-length"),
        Some(page.body.len().to_string().as_str())
    );
    let tag = injected_tag(&text).expect("the injected script element");
    assert_eq!(
        text.replacen(tag, "", 1),
        original,
        "only the tag was added"
    );
    assert!(text.contains(&format!("{tag}</body>")), "{text}");
}

#[test]
fn should_inject_when_fetch_destination_is_missing_or_iframe() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let plain = server.get("/");
    let framed = server.request("GET", "/", &[("Sec-Fetch-Dest", "iframe")]);

    // then
    for (case, reply) in [("no header", plain), ("iframe", framed)] {
        assert_eq!(reply.text().matches(CLIENT).count(), 1, "{case}");
        assert_eq!(
            reply.header("content-length"),
            Some(reply.body.len().to_string().as_str()),
            "{case}"
        );
    }
}

#[test]
fn should_inject_before_real_closing_tag_when_body_tag_text_appears_earlier() {
    // given
    let site = Site::copy();
    let page = "<!doctype html><html><head><title>t</title></head><body>\
        <!-- </body> --><script>var s = \"</body>\";</script><p>end</p></body></html>";
    fs::write(site.path().join("tricky.html"), page).expect("a page");
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.request("GET", "/tricky.html", &DOCUMENT);

    // then
    let text = reply.text();
    let tag = injected_tag(&text).expect("the injected script element");
    assert_eq!(text.matches(CLIENT).count(), 1, "{text}");
    assert_eq!(text.replacen(tag, "", 1), page, "only the tag was added");
    assert!(
        text.ends_with(&format!("<p>end</p>{tag}</body></html>")),
        "{text}"
    );
}

#[test]
fn should_inject_before_closing_tag_when_it_is_uppercase() {
    // given
    let site = Site::copy();
    let page = "<!DOCTYPE HTML><HTML><HEAD><TITLE>t</TITLE></HEAD><BODY><P>end</P></BODY></HTML>";
    fs::write(site.path().join("upper.html"), page).expect("a page");
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.request("GET", "/upper.html", &DOCUMENT);

    // then
    let text = reply.text();
    let tag = injected_tag(&text).expect("the injected script element");
    assert_eq!(text.matches(CLIENT).count(), 1, "{text}");
    assert!(
        text.contains(&format!("<P>end</P>{tag}</BODY>")),
        "the tag goes right before the original-case close: {text}"
    );
    assert_eq!(text.replacen(tag, "", 1), page, "only the tag was added");
}

#[test]
fn should_serve_original_bytes_when_fetch_destination_is_empty() {
    // given
    let site = Site::copy();
    let original = fs::read(site.path().join("index.html")).expect("the fixture page");
    let server = PreviewProcess::serve(&site);

    // when
    let fetched = server.request("GET", "/", &[("Sec-Fetch-Dest", "empty")]);

    // then
    assert_eq!(fetched.status, 200);
    assert_eq!(fetched.body, original);
    assert_eq!(
        fetched.header("content-length"),
        Some(original.len().to_string().as_str())
    );
}

#[test]
fn should_answer_head_with_get_headers_when_page_is_injected() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let get = server.request("GET", "/", &DOCUMENT);
    let head = server.request("HEAD", "/", &DOCUMENT);

    // then
    let stable = |reply: &Reply| -> Vec<(String, String)> {
        reply
            .headers
            .iter()
            .filter(|(name, _)| name != "date")
            .cloned()
            .collect()
    };
    assert_eq!(head.status, 200);
    assert_eq!(stable(&head), stable(&get));
    assert_eq!(
        head.header("content-length"),
        Some(get.body.len().to_string().as_str()),
        "HEAD announces the rewritten length"
    );
    assert!(head.body.is_empty(), "{:?}", head.body);
}

#[test]
fn should_not_inject_when_resource_is_not_html() {
    // given
    let site = Site::copy();
    let trap = "</body>";
    let planted = [
        (
            "data.kml",
            format!("<kml><Document>{trap}</Document></kml>"),
        ),
        ("doc.pdf", format!("%PDF-1.4\n{trap}\n%%EOF\n")),
    ];
    for (name, content) in &planted {
        fs::write(site.path().join(name), content).expect("a planted file");
    }
    let server = PreviewProcess::serve(&site);
    let files = [
        "data.json",
        "images/sample.svg",
        "routes.gpx",
        "data.kml",
        "doc.pdf",
    ];

    // when
    let replies: Vec<_> = files
        .iter()
        .map(|name| (*name, server.request("GET", &format!("/{name}"), &DOCUMENT)))
        .collect();

    // then
    for (name, reply) in replies {
        let on_disk = fs::read(site.path().join(name)).expect("the file");
        assert_eq!(reply.status, 200, "{name}");
        assert_eq!(reply.body, on_disk, "{name} is served as stored");
        assert!(!reply.text().contains(CLIENT), "{name}");
    }
}

#[test]
fn should_not_inject_when_response_is_an_error() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let missing_page = server.request("GET", "/missing.html", &DOCUMENT);
    let missing_dir = server.request("GET", "/nothing/", &DOCUMENT);
    let hidden = server.request("GET", "/.env", &DOCUMENT);
    let post = server.request("POST", "/", &DOCUMENT);

    // then
    assert_eq!(missing_page.status, 404);
    assert_eq!(missing_dir.status, 404);
    assert_eq!(hidden.status, 403);
    assert_eq!(post.status, 405);
    for (case, reply) in [
        ("missing page", &missing_page),
        ("missing directory", &missing_dir),
        ("hidden file", &hidden),
        ("POST", &post),
    ] {
        assert!(!reply.text().contains(CLIENT), "{case}");
        assert!(!reply.text().contains("<script"), "{case}");
        reply.assert_leaks_nothing(case);
    }
}

#[test]
fn should_stream_oversize_html_unchanged_when_larger_than_8_mib() {
    // given
    let site = Site::copy();
    let mut page = b"<html><body>".to_vec();
    page.resize(9 * MIB, b'x');
    page.extend_from_slice(b"</body></html>");
    fs::write(site.path().join("big.html"), &page).expect("a big page");
    let server = PreviewProcess::serve(&site);

    // when
    let first = server.request("GET", "/big.html", &DOCUMENT);
    let second = server.request("GET", "/big.html", &DOCUMENT);
    let lines = server.stop_and_stderr();

    // then
    for reply in [&first, &second] {
        assert_eq!(reply.status, 200);
        assert!(reply.body == page, "the 9 MiB page changed in transit");
        assert_eq!(
            reply.header("content-length"),
            Some(page.len().to_string().as_str())
        );
    }
    let found = warnings(&lines, "html-too-large");
    assert_eq!(found.len(), 1, "{lines:?}");
    assert!(found[0].contains("/big.html"), "{found:?}");
}

#[test]
fn should_serve_plain_html_when_meta_charset_contradicts_utf8() {
    // given
    let site = Site::copy();
    let page = "<html><head><meta charset=\"iso-8859-1\"></head><body>caf\u{e9}</body></html>";
    fs::write(site.path().join("latin.html"), page).expect("a page");
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.request("GET", "/latin.html", &DOCUMENT);
    let lines = server.stop_and_stderr();

    // then
    assert_eq!(reply.content_type(), Some("text/html"));
    assert_eq!(reply.body, page.as_bytes());
    assert_eq!(warnings(&lines, "html-not-utf8").len(), 1, "{lines:?}");
}

#[test]
fn should_warn_once_and_keep_page_bytes_when_meta_csp_blocks_reload() {
    // given
    let site = Site::copy();
    let meta = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'\">";
    let open = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'\">";
    for (name, policy) in [("locked.html", meta), ("open.html", open)] {
        let page = format!("<html><head>{policy}</head><body>x</body></html>");
        fs::write(site.path().join(name), page).expect("a page");
    }
    let server = PreviewProcess::serve(&site);

    // when
    let locked = server.request("GET", "/locked.html", &DOCUMENT);
    let again = server.request("GET", "/locked.html", &DOCUMENT);
    let open_page = server.request("GET", "/open.html", &DOCUMENT);
    let lines = server.stop_and_stderr();

    // then
    assert!(locked.text().contains(meta), "the policy is kept verbatim");
    assert_eq!(locked.body, again.body);
    assert!(open_page.text().contains(open));
    let found = warnings(&lines, "csp-blocks-live-reload");
    assert_eq!(
        found.len(),
        1,
        "once per path, none for a permissive page: {lines:?}"
    );
    assert!(found[0].contains("/locked.html"), "{found:?}");
}

#[test]
fn should_refuse_with_403_when_events_origin_is_foreign() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let port = server.port();
    let foreign_port = format!("http://127.0.0.1:{}", port.wrapping_add(1));
    let attempts: [(&str, Vec<(&str, &str)>); 5] = [
        ("foreign host", vec![("Origin", "http://evil.example")]),
        (
            "cross-site fetch",
            vec![
                ("Sec-Fetch-Site", "cross-site"),
                ("Origin", "http://evil.example"),
            ],
        ),
        ("opaque origin", vec![("Origin", "null")]),
        ("other port", vec![("Origin", foreign_port.as_str())]),
        (
            "fetch metadata alone",
            vec![("Sec-Fetch-Site", "cross-site")],
        ),
    ];

    // when
    let replies: Vec<_> = attempts
        .iter()
        .map(|(case, headers)| (*case, refused(server.events(headers))))
        .collect();

    // then
    for (case, reply) in replies {
        assert_eq!(reply.status, 403, "{case}");
        assert_eq!(reply.text(), "403 Forbidden", "{case}");
        assert_eq!(reply.header("cache-control"), Some("no-store"), "{case}");
    }
}

#[test]
fn should_open_stream_when_origin_is_same_origin_or_absent() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let same = server.url().trim_end_matches('/').to_owned();

    // when
    let mut bare = opened(server.events(&[]));
    let mut browser =
        opened(server.events(&[("Origin", same.as_str()), ("Sec-Fetch-Site", "same-origin")]));

    // then
    assert_eq!(bare.baseline().event.as_deref(), Some("state"));
    assert_eq!(browser.baseline().event.as_deref(), Some("state"));
}

#[test]
fn should_send_state_baseline_matching_page_revision_when_stream_opens() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let page = server.request("GET", "/", &DOCUMENT).text();
    let revision = page
        .split("data-revision=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a data-revision attribute")
        .to_owned();
    let instance = page
        .split("data-instance=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a data-instance attribute")
        .to_owned();

    // when
    let mut stream = opened(server.events(&[]));
    let frame = stream.baseline();

    // then
    assert_eq!(frame.id.as_deref(), Some(revision.as_str()), "{frame:?}");
    assert_eq!(
        frame.data,
        [format!(
            "{{\"instance_id\":\"{instance}\",\"revision\":{revision}}}"
        )]
    );
    assert_eq!(
        frame.raw,
        format!(
            "event: state\nid: {revision}\ndata: {{\"instance_id\":\"{instance}\",\"revision\":{revision}}}"
        )
    );
}

#[test]
fn should_stream_with_event_headers_when_events_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let stream = opened(server.events(&[]));

    // then
    assert_eq!(stream.header("content-type"), Some("text/event-stream"));
    assert_eq!(stream.header("cache-control"), Some("no-store"));
    assert_eq!(stream.header("x-accel-buffering"), Some("no"));
    assert_eq!(stream.header("x-content-type-options"), Some("nosniff"));
}

#[test]
fn should_answer_headers_only_when_head_requests_events() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.head(EVENTS);

    // then
    assert_eq!(reply.status, 200);
    assert_eq!(reply.content_type(), Some("text/event-stream"));
    assert_eq!(reply.header("x-accel-buffering"), Some("no"));
    assert!(reply.body.is_empty(), "{:?}", reply.body);
}

#[test]
fn should_refuse_exactly_one_stream_with_503_when_65_connect() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let mut open = Vec::new();
    let mut refusals = Vec::new();
    for _ in 0..65 {
        match server.events(&[]) {
            Ok(mut stream) => {
                stream.baseline();
                open.push(stream);
            }
            Err(reply) => refusals.push(reply),
        }
    }

    // then
    assert_eq!(open.len(), 64);
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].status, 503);
    assert_eq!(refusals[0].header("retry-after"), Some("2"));
    assert_eq!(refusals[0].header("cache-control"), Some("no-store"));
}

#[test]
fn should_free_permit_when_stream_disconnects() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let mut open: Vec<EventStream> = (0..64)
        .map(|_| {
            let mut stream = opened(server.events(&[]));
            stream.baseline();
            stream
        })
        .collect();
    assert_eq!(
        refused(server.events(&[])).status,
        503,
        "the budget is spent"
    );

    // when
    drop(open.pop());

    // then: polled, because only the server's scheduler decides when it notices the close
    let mut reopened = None;
    wait_until(DEADLINE, "a freed stream permit", || {
        reopened = server.events(&[]).ok();
        reopened.is_some()
    })
    .expect("a permit comes back");
    let mut stream = reopened.expect("an open stream");
    assert_eq!(stream.baseline().event.as_deref(), Some("state"));
    assert_eq!(refused(server.events(&[])).status, 503, "still at the cap");
}

#[test]
fn should_leave_source_tree_unchanged_when_pages_and_streams_are_served() {
    // given
    let site = Site::copy();
    let before = tree_hash(site.path());
    let server = PreviewProcess::serve(&site);

    // when
    let mut stream = opened(server.events(&[]));
    stream.baseline();
    server.request("GET", "/", &DOCUMENT);
    server.request("GET", "/guide/", &DOCUMENT);
    server.request("HEAD", "/", &DOCUMENT);
    server.get("/.env");
    refused(server.events(&[("Origin", "http://evil.example")]));
    drop(stream);

    // then
    assert_eq!(tree_hash(site.path()), before);
}
