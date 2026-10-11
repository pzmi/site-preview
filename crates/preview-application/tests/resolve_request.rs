//! Component tests of the `ResolveRequest` query over the in-memory site.

use std::cell::Cell;
use std::io::{ErrorKind, Read};

use preview_application::{ForbiddenCause, ForbiddenReason, ReloadState, ResolveRequest, Resolved};
use preview_domain::eligibility::DenyReason;
use preview_domain::{ChangeSet, SiteState, decode_request_path};
use preview_testkit::{FixedInstanceIds, InMemorySite};

fn state() -> SiteState {
    SiteState::initial(FixedInstanceIds::default().next_id())
}

/// Resolves `raw` and reports how many times the snapshot was taken.
fn resolve(site: &InMemorySite, raw: &str) -> (Resolved, usize) {
    let taken = Cell::new(0);
    let resolved = ResolveRequest { raw_path: raw }.execute(site, || {
        taken.set(taken.get() + 1);
        state()
    });
    (resolved, taken.get())
}

fn served(resolved: Resolved) -> (String, Vec<u8>, SiteState) {
    let Resolved::File {
        mut file,
        snapshot,
        name,
    } = resolved
    else {
        panic!("expected a file, got {resolved:?}");
    };
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("in-memory read");
    (name.as_str().to_owned(), body, snapshot)
}

fn site() -> InMemorySite {
    InMemorySite::new()
        .with_file("index.html", b"<h1>home</h1>")
        .with_file("guide/index.html", b"<h1>guide</h1>")
        .with_file("css/site.css", b"body{}")
        .with_dir("empty")
}

#[test]
fn should_redirect_when_directory_requested_without_slash() {
    // given
    let site = site();
    // when
    let (resolved, _) = resolve(&site, "/guide");
    // then
    let Resolved::RedirectToSlash { path } = resolved else {
        panic!("expected a redirect, got {resolved:?}");
    };
    assert_eq!(path, decode_request_path("/guide").expect("decodes"));
}

#[test]
fn should_redirect_when_directory_without_index_requested_without_slash() {
    // given
    let site = site();
    // when
    let (resolved, _) = resolve(&site, "/empty");
    // then
    assert!(
        matches!(resolved, Resolved::RedirectToSlash { .. }),
        "{resolved:?}"
    );
}

#[test]
fn should_return_not_found_when_directory_lacks_index() {
    // given
    let site = site();
    // when
    let (resolved, _) = resolve(&site, "/empty/");
    // then
    assert!(matches!(resolved, Resolved::NotFound), "{resolved:?}");
}

#[test]
fn should_serve_index_when_directory_requested_with_slash() {
    // given
    let site = site();
    // when
    let root = served(resolve(&site, "/").0);
    let guide = served(resolve(&site, "/guide/").0);
    // then
    assert_eq!(root.0, "index.html");
    assert_eq!(root.1, b"<h1>home</h1>");
    assert_eq!(guide.0, "index.html");
    assert_eq!(guide.1, b"<h1>guide</h1>");
}

#[test]
fn should_serve_file_with_its_name_when_file_requested() {
    // given
    let site = site();
    // when
    let (name, body, snapshot) = served(resolve(&site, "/css/site.css").0);
    // then
    assert_eq!(name, "site.css");
    assert_eq!(body, b"body{}");
    assert_eq!(snapshot, state());
}

#[test]
fn should_forbid_when_component_hidden() {
    // given
    let site = site()
        .with_file(".git/config", b"secret")
        .with_file(".env", b"KEY=1");
    // when
    let results = ["/.git/config", "/.env", "/.git/"].map(|raw| resolve(&site, raw));
    // then
    for (resolved, taken) in results {
        assert!(
            matches!(
                resolved,
                Resolved::Forbidden(ForbiddenCause::Ineligible(DenyReason::Hidden))
            ),
            "{resolved:?}"
        );
        assert_eq!(taken, 0);
    }
    assert_eq!(site.open_count(), 0);
}

#[test]
fn should_not_open_when_path_is_ineligible() {
    // given
    let site = site()
        .with_file("a/node_modules/x.js", b"x")
        .with_file("page.html~", b"old")
        .with_file("__site_preview/events", b"fake")
        .with_file("node_modules/index.html", b"pkg");
    // when
    let results = [
        "/a/node_modules/x.js",
        "/page.html~",
        "/__site_preview/events",
        "/node_modules/",
    ]
    .map(|raw| resolve(&site, raw));
    // then
    for (resolved, taken) in results {
        assert!(matches!(resolved, Resolved::Forbidden(_)), "{resolved:?}");
        assert_eq!(taken, 0);
    }
    assert_eq!(site.open_count(), 0);
}

#[test]
fn should_return_not_found_when_excluded_directory_requested_without_slash() {
    // given
    let site = site()
        .with_file("node_modules/index.html", b"pkg")
        .with_dir("vendor")
        .with_symlink("docs/__pycache__/index.html")
        .with_io_error("lib/vendor/index.html", ErrorKind::InvalidData);
    let absent = InMemorySite::new();
    // when
    let present = [
        "/node_modules",
        "/vendor",
        "/docs/__pycache__",
        "/lib/vendor",
    ]
    .map(|raw| resolve(&site, raw).0);
    let missing = resolve(&absent, "/node_modules").0;
    // then
    for resolved in present {
        assert!(matches!(resolved, Resolved::NotFound), "{resolved:?}");
    }
    assert!(matches!(missing, Resolved::NotFound), "{missing:?}");
}

#[test]
fn should_serve_file_when_name_is_excluded_only_for_directories() {
    // given
    let site = site().with_file("vendor", b"plain file");
    // when
    let (name, body, _) = served(resolve(&site, "/vendor").0);
    // then
    assert_eq!(name, "vendor");
    assert_eq!(body, b"plain file");
}

#[test]
fn should_snapshot_state_before_open_when_resolving_file() {
    // given
    let site = site();
    let reload = ReloadState::new(FixedInstanceIds::default().next_id());
    let writer = reload.clone();
    site.on_open(move || {
        writer
            .publish_next(ChangeSet::new())
            .expect("revision advances");
    });
    // when
    let resolved = ResolveRequest {
        raw_path: "/index.html",
    }
    .execute(&site, || reload.current());
    // then
    let (_, _, snapshot) = served(resolved);
    assert_eq!(snapshot, state());
    assert_eq!(reload.current().revision().get(), 1);
    assert_eq!(site.open_count(), 1);
}

#[test]
fn should_return_bad_request_when_encoding_malformed() {
    // given
    let site = site();
    // when
    let results = ["/%zz", "/a%", "/a%00b", "relative"].map(|raw| resolve(&site, raw));
    // then
    for (resolved, taken) in results {
        assert!(matches!(resolved, Resolved::BadRequest), "{resolved:?}");
        assert_eq!(taken, 0);
    }
    assert_eq!(site.open_count(), 0);
}

#[test]
fn should_return_not_found_when_path_attempts_traversal() {
    // given
    let site = site();
    // when
    let results = [
        "/../etc/passwd",
        "/%2e%2e/x",
        "/a%2fb",
        "/guide//index.html",
    ]
    .map(|raw| resolve(&site, raw));
    // then
    for (resolved, taken) in results {
        assert!(matches!(resolved, Resolved::NotFound), "{resolved:?}");
        assert_eq!(taken, 0);
    }
    assert_eq!(site.open_count(), 0);
}

#[test]
fn should_return_not_found_when_file_missing() {
    // given
    let site = site();
    // when
    let (resolved, _) = resolve(&site, "/missing.html");
    // then
    assert!(matches!(resolved, Resolved::NotFound), "{resolved:?}");
}

#[test]
fn should_return_not_found_when_file_requested_with_trailing_slash() {
    // given
    let site = site();
    // when
    let (resolved, _) = resolve(&site, "/css/site.css/");
    // then
    assert!(matches!(resolved, Resolved::NotFound), "{resolved:?}");
}

#[test]
fn should_forbid_when_content_refuses_path() {
    // given
    let site = site()
        .with_symlink("link.html")
        .with_symlink("linked")
        .with_special("fifo");
    // when
    let results = ["/link.html", "/linked/page.html", "/fifo"].map(|raw| resolve(&site, raw).0);
    // then
    let causes = results.map(|resolved| match resolved {
        Resolved::Forbidden(ForbiddenCause::Content(reason)) => reason,
        other => panic!("expected a content refusal, got {other:?}"),
    });
    assert_eq!(
        causes,
        [
            ForbiddenReason::Symlink,
            ForbiddenReason::Symlink,
            ForbiddenReason::NotRegular
        ]
    );
}

#[test]
fn should_report_unavailable_when_open_fails_with_io_error() {
    // given
    let site = site().with_io_error("broken.html", ErrorKind::InvalidData);
    // when
    let (resolved, _) = resolve(&site, "/broken.html");
    // then
    assert!(
        matches!(resolved, Resolved::Unavailable(ErrorKind::InvalidData)),
        "{resolved:?}"
    );
}

#[test]
fn should_print_no_content_when_resolved_is_debug_printed() {
    // given
    let site = site().with_file("secret.txt", b"do-not-print");
    // when
    let text = format!("{:?}", resolve(&site, "/secret.txt").0);
    // then
    assert!(!text.contains("do-not-print"), "{text}");
    assert!(text.contains("len: 12"), "{text}");
}
