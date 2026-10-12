//! Black-box tests of `site-preview serve`: the real binary on a temporary copy of the shared
//! fixture site (ADR 0016), a real listener on an OS-assigned port, and raw HTTP/1.1 requests.
//! The refusals (dotfiles, traversal, symlinks, Host) live in `http_security.rs`.

pub mod support;

use std::fs;

use predicates::prelude::*;
use support::{DEADLINE, PreviewProcess, Site, fixture_site, site_preview, tree_hash};

#[test]
fn should_serve_index_when_started_on_fixture() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.get("/");

    // then
    // The page is compared by its own content: the reload client is injected into served
    // HTML, so the response is not byte-equal to the file once injection exists.
    assert_eq!(reply.status, 200);
    let page = String::from_utf8(reply.body).expect("a UTF-8 page");
    assert!(page.contains("<h1>Route planner</h1>"), "{page}");
    assert!(
        server.url().starts_with("http://127.0.0.1:"),
        "not loopback: {}",
        server.url()
    );
}

#[test]
fn should_serve_stylesheet_bytes_unchanged_when_requested() {
    // given
    let site = Site::copy();
    let before = tree_hash(site.path());
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.get("/styles.css");
    drop(server);

    // then
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.body,
        fs::read(fixture_site().join("styles.css")).expect("the fixture stylesheet")
    );
    assert_eq!(
        tree_hash(site.path()),
        before,
        "serving changed the site tree"
    );
    assert_eq!(
        tree_hash(&fixture_site()),
        site.pristine_hash(),
        "the shared fixture differs from its copy"
    );
}

#[test]
fn should_label_content_type_when_serving_fixture_files() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let expected = [
        ("/", "text/html; charset=utf-8"),
        ("/guide/", "text/html; charset=utf-8"),
        ("/styles.css", "text/css; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/data.json", "application/json"),
        ("/routes.gpx", "application/gpx+xml"),
        ("/images/sample.svg", "image/svg+xml"),
    ];

    // when
    let replies: Vec<_> = expected.iter().map(|(path, _)| server.get(path)).collect();

    // then
    for ((path, content_type), reply) in expected.iter().zip(&replies) {
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(reply.content_type(), Some(*content_type), "{path}");
    }
}

#[test]
fn should_redirect_to_slash_with_query_when_directory_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let redirect = server.get("/guide?from=home");
    let target = server.get(redirect.location().unwrap_or("/"));

    // then
    assert_eq!(redirect.status, 307);
    assert_eq!(redirect.location(), Some("/guide/?from=home"));
    assert_eq!(target.status, 200);
    // Compared by content, not bytes: served HTML gains the reload client once injection exists.
    let page = String::from_utf8(target.body).expect("a UTF-8 page");
    assert!(page.contains("<h1>Guide</h1>"), "{page}");
}

#[test]
fn should_exit_1_when_root_missing() {
    // given
    let parent = tempfile::tempdir().expect("a temporary directory");
    let missing = parent.path().join("missing");
    let mut command = site_preview();

    // when
    let assert = command.arg("serve").arg(&missing).assert();

    // then
    assert
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(
            "site-preview: error: cannot serve ",
        ))
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn should_leave_source_bytes_unchanged_when_served() {
    // given
    let site = Site::copy();
    let index = fs::read(site.path().join("index.html")).expect("the fixture page");
    let before = tree_hash(site.path());
    let server = PreviewProcess::serve(&site);

    // when
    let page = server.request("GET", "/", &[("Sec-Fetch-Dest", "document")]);
    let fetched = server.request("GET", "/", &[("Sec-Fetch-Dest", "empty")]);

    // then: the navigation is injected once, before `</body>`, with an exact length; the
    // script fetch and the file on disk are the original bytes
    let text = page.text();
    assert_eq!(
        text.matches("/__site_preview/client.js").count(),
        1,
        "{text}"
    );
    assert!(
        text.contains("data-revision=\"0\"></script></body>"),
        "{text}"
    );
    assert_eq!(
        page.header("content-length"),
        Some(page.body.len().to_string().as_str())
    );
    assert_eq!(fetched.body, index);
    assert_eq!(
        fs::read(site.path().join("index.html")).expect("the page"),
        index
    );
    assert_eq!(tree_hash(site.path()), before);
}

#[test]
fn should_warn_on_stderr_once_when_html_is_not_utf8() {
    // given
    let site = Site::copy();
    fs::write(site.path().join("old.html"), b"<body><p>caf\xe9</p></body>").expect("a page");
    let server = PreviewProcess::serve(&site);

    // when
    let first = server.get("/old.html");
    let second = server.get("/old.html");
    let after = server.get("/");

    // then
    assert_eq!(first.content_type(), Some("text/html"));
    assert_eq!(first.body, second.body);
    assert_eq!(after.status, 200);
    let warning = server
        .stderr()
        .recv_timeout(DEADLINE)
        .expect("a warning line on stderr");
    assert!(
        warning.starts_with("site-preview: warning[html-not-utf8]: /old.html: "),
        "{warning}"
    );
    assert!(
        server.stderr().try_recv().is_err(),
        "a second warning for the same page"
    );
}
