//! Black-box HTTP and security suite (plan.md §12 "HTTP and security integration tests",
//! the M1 exit gate of §11; foundation.md §7 rows 1 to 4).
//!
//! Every test runs the real binary on a [`Site`]: a copy of the shared fixture with
//! synthetic secrets planted beside and inside the root, which no response may ever carry.

pub mod support;

use std::fs;
use std::os::unix::fs::symlink;

use support::{
    ENV_CANARY, GIT_CONFIG_CANARY, NODE_MODULES_CANARY, OUTSIDE_CANARY, PreviewProcess, Reply,
    Site, VENDOR_CANARY, fixture_site, tree_hash,
};

/// The constant body of every error status (`errors.rs` in the HTTP adapter).
fn error_body(status: u16) -> &'static str {
    match status {
        400 => "400 Bad Request",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        421 => "Host not allowed",
        _ => "an unexpected status",
    }
}

/// Asserts `reply` is the refusal `status` with its constant body and no planted secret.
fn assert_refused(reply: &Reply, status: u16, case: &str) {
    assert_eq!(reply.status, status, "{case}: {}", reply.text());
    assert_eq!(reply.text(), error_body(status), "{case}");
    reply.assert_leaks_nothing(case);
}

/// Asserts a traversal attempt got 400 or 404 with nothing but the constant error body.
fn assert_traversal_refused(reply: &Reply, case: &str) {
    assert!(
        matches!(reply.status, 400 | 404),
        "{case}: got {}",
        reply.status
    );
    assert_refused(reply, reply.status, case);
}

#[test]
fn should_serve_root_index_when_root_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.get("/");

    // then
    assert_eq!(reply.status, 200);
    assert_eq!(reply.content_type(), Some("text/html; charset=utf-8"));
    assert!(reply.text().contains("<h1>Route planner</h1>"), "{reply:?}");
}

#[test]
fn should_serve_nested_index_when_directory_requested_with_slash() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.get("/guide/");

    // then
    assert_eq!(reply.status, 200);
    assert!(reply.text().contains("<h1>Guide</h1>"), "{reply:?}");
}

#[test]
fn should_redirect_with_307_when_directory_requested_without_slash() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.get("/guide");

    // then
    assert_eq!(reply.status, 307);
    assert_eq!(reply.location(), Some("/guide/"));
    assert!(!reply.text().contains("<h1>"), "{reply:?}");
}

#[test]
fn should_return_404_without_listing_when_directory_has_no_index() {
    // given: `images/` holds `sample.svg` and no index
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let reply = server.get("/images/");

    // then
    assert_refused(&reply, 404, "/images/");
    assert!(!reply.text().contains("sample.svg"), "{reply:?}");
}

#[test]
fn should_send_file_bytes_with_length_when_get_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let expected = fs::read(fixture_site().join("styles.css")).expect("the fixture stylesheet");

    // when
    let reply = server.get("/styles.css");

    // then
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, expected);
    assert_eq!(
        reply.header("content-length"),
        Some(expected.len().to_string().as_str())
    );
}

#[test]
fn should_send_get_headers_without_body_when_head_requested() {
    for target in ["/styles.css", "/", "/data.json"] {
        // given
        let site = Site::copy();
        let server = PreviewProcess::serve(&site);
        let get = server.get(target);

        // when
        let head = server.head(target);

        // then
        assert_eq!(head.status, 200, "{target}");
        assert!(head.body.is_empty(), "{target}: HEAD sent a body");
        assert_eq!(head.content_type(), get.content_type(), "{target}");
        assert_eq!(
            head.header("content-length"),
            Some(get.body.len().to_string().as_str()),
            "{target}"
        );
    }
}

#[test]
fn should_return_404_when_path_names_nothing() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for target in [
        "/missing.html",
        "/guide/missing/",
        "/styles.css/x",
        "/styles.css/",
    ] {
        // when
        let reply = server.get(target);

        // then
        assert_refused(&reply, 404, target);
    }
}

#[test]
fn should_return_405_with_allow_when_method_is_not_get_or_head() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for method in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS"] {
        // when
        let reply = server.request(method, "/", &[("Content-Length", "0")]);

        // then
        assert_refused(&reply, 405, method);
        assert_eq!(reply.header("allow"), Some("GET, HEAD"), "{method}");
    }
}

#[test]
fn should_send_no_store_and_nosniff_when_answering_any_status() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let bad_host = format!("GET / HTTP/1.1\r\nHost: evil.example:{}\r\n", server.port());

    // when
    let replies = [
        ("200", server.get("/styles.css")),
        ("200 head", server.head("/")),
        ("307", server.get("/guide")),
        ("400", server.get("/%ff")),
        ("403", server.get("/.env")),
        ("404", server.get("/missing")),
        (
            "405",
            server.request("POST", "/", &[("Content-Length", "0")]),
        ),
        ("421", server.raw(bad_host.as_bytes())),
    ];

    // then
    for (case, reply) in &replies {
        assert_eq!(
            reply.status.to_string(),
            case.split(' ').next().unwrap_or_default(),
            "{case}"
        );
        assert_eq!(reply.header("cache-control"), Some("no-store"), "{case}");
        assert_eq!(
            reply.header("x-content-type-options"),
            Some("nosniff"),
            "{case}"
        );
        assert_eq!(reply.header("access-control-allow-origin"), None, "{case}");
    }
}

#[test]
fn should_not_serve_git_config_when_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for target in ["/.git/config", "/.git/", "/.git", "/.GIT/config"] {
        // when
        let get = server.get(target);
        let head = server.head(target);

        // then
        assert_refused(&get, 403, target);
        assert_eq!(head.status, 403, "HEAD {target}");
        assert!(!get.text().contains(GIT_CONFIG_CANARY), "{target}");
    }
}

#[test]
fn should_not_serve_env_when_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for target in ["/.env", "/.ENV", "/%2eenv", "/guide/../.env"] {
        // when
        let reply = server.get(target);

        // then
        assert!(
            matches!(reply.status, 403 | 404),
            "{target}: got {}",
            reply.status
        );
        assert_refused(&reply, reply.status, target);
        assert!(!reply.text().contains(ENV_CANARY), "{target}");
    }
    assert_refused(&server.get("/.env"), 403, "/.env");
}

#[test]
fn should_not_serve_vendor_file_when_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let file = server.get("/vendor/x.js");
    let folded = server.get("/VENDOR/x.js");
    let directory = server.get("/vendor");

    // then: a path through it is forbidden; the directory itself is not even acknowledged
    assert_refused(&file, 403, "/vendor/x.js");
    assert_refused(&folded, 403, "/VENDOR/x.js");
    assert_refused(&directory, 404, "/vendor");
    assert!(!file.text().contains(VENDOR_CANARY));
}

#[test]
fn should_not_serve_node_modules_file_when_requested() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    // when
    let file = server.get("/node_modules/x.js");
    let folded = server.get("/Node_Modules/x.js");
    let directory = server.get("/node_modules");

    // then: a path through it is forbidden; the directory itself is not even acknowledged
    assert_refused(&file, 403, "/node_modules/x.js");
    assert_refused(&folded, 403, "/Node_Modules/x.js");
    assert_refused(&directory, 404, "/node_modules");
    assert!(!file.text().contains(NODE_MODULES_CANARY));
}

#[test]
fn should_refuse_traversal_when_dot_segments_are_percent_encoded() {
    // given: one `..` above the root leads to `outside/secret.txt`
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for target in [
        "/%2e%2e/",
        "/%2e%2e/outside/secret.txt",
        "/%2E%2E/outside/secret.txt",
        "/.%2e/outside/secret.txt",
        "/guide/%2e%2e/%2e%2e/outside/secret.txt",
        "/../outside/secret.txt",
    ] {
        // when
        let reply = server.get(target);

        // then
        assert_traversal_refused(&reply, target);
    }
}

#[test]
fn should_refuse_traversal_when_slash_is_percent_encoded() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for target in [
        "/..%2f",
        "/..%2foutside%2fsecret.txt",
        "/..%2Foutside/secret.txt",
        "/%2e%2e%2foutside%2fsecret.txt",
        "/guide/..%2f..%2foutside/secret.txt",
        "/..%5coutside%5csecret.txt",
    ] {
        // when
        let reply = server.get(target);

        // then
        assert_traversal_refused(&reply, target);
    }
}

#[test]
fn should_refuse_traversal_when_dot_segments_are_double_encoded() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);

    for target in [
        "/%252e%252e/",
        "/%252e%252e/outside/secret.txt",
        "/%252e%252e%252foutside%252fsecret.txt",
        "/guide/%252e%252e/%252e%252e/outside/secret.txt",
    ] {
        // when
        let reply = server.get(target);

        // then
        assert_traversal_refused(&reply, target);
    }
}

#[test]
fn should_refuse_symlink_when_it_points_outside_root() {
    // given
    let site = Site::copy();
    symlink(
        site.outside().join("secret.txt"),
        site.path().join("leak.txt"),
    )
    .expect("a symlink");
    let server = PreviewProcess::serve(&site);

    // when
    let get = server.get("/leak.txt");
    let head = server.head("/leak.txt");

    // then
    assert_refused(&get, 403, "/leak.txt");
    assert_eq!(head.status, 403);
    assert!(!get.text().contains(OUTSIDE_CANARY));
}

#[test]
fn should_refuse_path_when_symlink_is_in_the_middle() {
    // given: `escape` is a directory symlink to the outside directory
    let site = Site::copy();
    symlink(site.outside(), site.path().join("escape")).expect("a symlink");
    let server = PreviewProcess::serve(&site);

    for target in ["/escape/secret.txt", "/escape/", "/escape"] {
        // when
        let reply = server.get(target);

        // then
        assert_refused(&reply, 403, target);
    }
}

#[test]
fn should_refuse_symlink_when_it_points_inside_root() {
    // given: in-root aliases of a served file and a served directory
    let site = Site::copy();
    symlink("styles.css", site.path().join("alias.css")).expect("a file symlink");
    symlink("guide", site.path().join("docs")).expect("a directory symlink");
    let server = PreviewProcess::serve(&site);

    for target in ["/alias.css", "/docs/", "/docs/index.html", "/docs"] {
        // when
        let reply = server.get(target);

        // then
        assert_refused(&reply, 403, target);
    }
    assert_eq!(server.get("/styles.css").status, 200, "the target itself");
}

#[test]
fn should_serve_site_when_root_argument_is_a_symlink() {
    // given: only components below the root are held to the no-symlink rule (plan.md §5.3)
    let site = Site::copy();
    let link_parent = tempfile::tempdir().expect("a temporary directory");
    let link = link_parent.path().join("root-link");
    symlink(site.path(), &link).expect("a root symlink");
    let server = PreviewProcess::start([std::ffi::OsStr::new("serve"), link.as_os_str()]);

    // when
    let reply = server.get("/");

    // then
    assert_eq!(reply.status, 200);
    assert!(reply.text().contains("<h1>Route planner</h1>"), "{reply:?}");
}

#[test]
fn should_reject_with_421_when_host_names_another_site() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let port = server.port();
    let bound = server.authority().to_owned();
    let forged: [Vec<u8>; 5] = [
        format!("GET / HTTP/1.1\r\nHost: evil.example:{port}\r\n").into_bytes(),
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n".to_vec(),
        format!("GET / HTTP/1.1\r\nHost: {bound}\r\nHost: evil.example:{port}\r\n").into_bytes(),
        format!("GET http://evil.example:{port}/ HTTP/1.1\r\nHost: {bound}\r\n").into_bytes(),
        format!("GET /__site_preview/events HTTP/1.1\r\nHost: evil.example:{port}\r\n")
            .into_bytes(),
    ];

    for head in forged {
        // when
        let reply = server.raw(&head);

        // then
        let case = String::from_utf8_lossy(&head);
        assert_refused(&reply, 421, &case);
    }
}

#[test]
fn should_serve_when_host_is_bound_address_or_localhost() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let port = server.port();
    let bound = server.authority().to_owned();

    for host in [bound, format!("localhost:{port}")] {
        // when
        let reply = server.raw(format!("GET / HTTP/1.1\r\nHost: {host}\r\n").as_bytes());

        // then
        assert_eq!(reply.status, 200, "{host}");
        assert!(reply.text().contains("<h1>"), "{host}: {reply:?}");
    }
}

#[test]
fn should_leave_tree_unchanged_when_suite_has_run() {
    // given: the hostile site, with every symlink the suite plants
    let site = Site::copy();
    symlink(
        site.outside().join("secret.txt"),
        site.path().join("leak.txt"),
    )
    .expect("a symlink");
    symlink(site.outside(), site.path().join("escape")).expect("a symlink");
    symlink("styles.css", site.path().join("alias.css")).expect("a symlink");
    let before = tree_hash(site.path());
    let outside_before = tree_hash(&site.outside());
    let fixture_before = tree_hash(&fixture_site());
    let server = PreviewProcess::serve(&site);
    let targets = [
        "/",
        "/guide",
        "/guide/",
        "/images/",
        "/styles.css",
        "/app.js",
        "/data.json",
        "/routes.gpx",
        "/images/sample.svg",
        "/missing",
        "/.git/config",
        "/.env",
        "/vendor/x.js",
        "/node_modules/x.js",
        "/%2e%2e/outside/secret.txt",
        "/..%2foutside%2fsecret.txt",
        "/%252e%252e/outside/secret.txt",
        "/leak.txt",
        "/escape/secret.txt",
        "/alias.css",
    ];

    // when
    for target in targets {
        server.get(target);
        server.head(target);
        server.request("POST", target, &[("Content-Length", "0")]);
    }
    drop(server);

    // then
    assert_eq!(tree_hash(site.path()), before, "serving changed the site");
    assert_eq!(
        tree_hash(&site.outside()),
        outside_before,
        "serving changed the outside"
    );
    assert_eq!(
        tree_hash(&fixture_site()),
        fixture_before,
        "the committed fixture changed while the suite served its copy"
    );
}
