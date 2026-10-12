//! Injection of the client script into served HTML (plan.md §6, §7.3; ADR 0022, ADR 0046),
//! driven in process with `tower::ServiceExt::oneshot`.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::inject::ScriptTag;
use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, HtmlBudget, Injection, Shutdown,
    StreamLimit, WarningSink, build_router,
};
use preview_application::{Opened, ReloadState, SiteContent};
use preview_domain::{BoundAddr, ChangeSet, HostPolicy, SiteState, decode_request_path};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

const PAGE: &str = "<!doctype html><html><head><title>t</title></head>\
                    <body><h1>Hi</h1></body></html>";

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String)>>);

impl WarningSink for Recorder {
    fn warn(&self, code: &str, message: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((code.to_owned(), message.to_owned()));
    }
}

impl Recorder {
    fn warnings(&self) -> Vec<(String, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Server {
    site: InMemorySite,
    reload: ReloadState,
    warnings: Arc<Recorder>,
    /// Built once, as composition does, so every request shares its once-per-path memo.
    injection: Injection,
}

impl Server {
    fn new(site: InMemorySite) -> Self {
        let mut ids = FixedInstanceIds::new(7, 1);
        let warnings = Arc::new(Recorder::default());
        Self {
            site,
            reload: ReloadState::new(ids.next_id()),
            injection: Injection::new(warnings.clone()),
            warnings,
        }
    }

    fn with_budget(mut self, budget: HtmlBudget) -> Self {
        self.injection = self.injection.with_budget(budget);
        self
    }

    fn app(&self) -> Router {
        build_router(AppState {
            content: Arc::new(self.site.clone()),
            reload: self.reload.clone(),
            host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
            shutdown: Shutdown::new().signal(),
            keep_alive: DEFAULT_KEEP_ALIVE,
            streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
            injection: self.injection.clone(),
        })
    }

    async fn send(&self, method: &str, uri: &str, dest: Option<&str>) -> Response<Body> {
        let mut request = Request::builder()
            .header("host", "127.0.0.1")
            .method(method)
            .uri(uri);
        if let Some(dest) = dest {
            request = request.header("sec-fetch-dest", dest);
        }
        let request = request.body(Body::empty()).expect("request");
        self.app().oneshot(request).await.expect("a response")
    }

    fn tag(&self, state: &SiteState) -> String {
        ScriptTag::from_state(state).as_str().to_owned()
    }

    /// The bytes the content port holds for `path` now.
    fn source(&self, path: &str) -> Vec<u8> {
        let path = decode_request_path(path).expect("a valid path");
        let Ok(Opened::File(mut file)) = self.site.open(&path) else {
            panic!("{path:?} is not a file");
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).expect("readable");
        bytes
    }
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

async fn body(response: Response<Body>) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes()
        .to_vec()
}

fn injected(page: &str, tag: &str) -> String {
    page.replacen("</body>", &format!("{tag}</body>"), 1)
}

#[tokio::test]
async fn should_inject_snapshot_revision_when_document_get() {
    // given: a save lands while the file is being opened
    let server = Server::new(InMemorySite::new().with_file("index.html", PAGE.as_bytes()));
    let before = server.reload.current();
    let reload = server.reload.clone();
    server.site.on_open(move || {
        reload
            .publish_next(ChangeSet::new())
            .expect("revision left");
    });

    // when
    let response = server.send("GET", "/", Some("document")).await;

    // then: the page carries the state read before the open, so the client reloads once
    assert_eq!(response.status(), StatusCode::OK);
    let page = String::from_utf8(body(response).await).expect("UTF-8");
    assert_eq!(page, injected(PAGE, &server.tag(&before)));
    assert!(page.contains("data-revision=\"0\""), "{page}");
    assert_eq!(server.reload.current().revision().get(), 1);
}

#[tokio::test]
async fn should_inject_when_fetch_dest_missing_or_iframe() {
    // given
    let server = Server::new(InMemorySite::new().with_file("index.html", PAGE.as_bytes()));
    let expected = injected(PAGE, &server.tag(&server.reload.current()));

    for dest in [None, Some("iframe")] {
        // when
        let response = server.send("GET", "/index.html", dest).await;

        // then
        assert_eq!(
            String::from_utf8(body(response).await).expect("UTF-8"),
            expected,
            "{dest:?}"
        );
    }
}

#[tokio::test]
async fn should_not_modify_when_sec_fetch_dest_empty() {
    // given
    let server = Server::new(InMemorySite::new().with_file("index.html", PAGE.as_bytes()));

    // when
    let response = server.send("GET", "/", Some("empty")).await;

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        header(&response, "content-length"),
        Some(PAGE.len().to_string().as_str())
    );
    assert_eq!(body(response).await, PAGE.as_bytes());
    assert!(server.warnings.warnings().is_empty());
}

#[tokio::test]
async fn should_set_content_length_of_rewritten_body_when_injected() {
    // given
    let server = Server::new(InMemorySite::new().with_file("index.html", PAGE.as_bytes()));
    let tag = server.tag(&server.reload.current());

    // when
    let response = server.send("GET", "/", Some("document")).await;

    // then
    let length: usize = header(&response, "content-length")
        .and_then(|v| v.parse().ok())
        .expect("a Content-Length");
    assert_eq!(header(&response, "etag"), None);
    assert_eq!(header(&response, "last-modified"), None);
    assert_eq!(
        header(&response, "content-type"),
        Some("text/html; charset=utf-8")
    );
    let sent = body(response).await;
    assert_eq!(length, sent.len());
    assert_eq!(length, PAGE.len() + tag.len());
}

#[tokio::test]
async fn should_serve_unchanged_and_warn_once_when_html_over_limit() {
    // given
    let page = vec![b'a'; 8 * 1024 * 1024 + 1];
    let server = Server::new(InMemorySite::new().with_file("huge.html", &page));

    // when
    let first = server.send("GET", "/huge.html", Some("document")).await;
    let first_type = header(&first, "content-type").map(str::to_owned);
    let first_body = body(first).await;
    let second = body(server.send("GET", "/huge.html", None).await).await;

    // then
    assert_eq!(first_type.as_deref(), Some("text/html"));
    assert!(first_body == page, "the first body differs from the file");
    assert!(second == page, "the second body differs from the file");
    let warnings = server.warnings.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].0, "html-too-large");
    assert!(warnings[0].1.starts_with("/huge.html: "), "{warnings:?}");
}

#[tokio::test]
async fn should_not_inject_when_head() {
    // given
    let server = Server::new(InMemorySite::new().with_file("index.html", PAGE.as_bytes()));
    let get = server.send("GET", "/", None).await;
    let get_headers = get.headers().clone();
    let get_length = body(get).await.len();

    // when
    let head = server.send("HEAD", "/", None).await;

    // then: the headers the GET carries, the rewritten length included, and no body
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers(), &get_headers);
    assert_eq!(
        header(&head, "content-length"),
        Some(get_length.to_string().as_str())
    );
    assert!(body(head).await.is_empty());
}

#[tokio::test]
async fn should_not_inject_when_response_is_error() {
    // given
    let server = Server::new(
        InMemorySite::new()
            .with_file("index.html", PAGE.as_bytes())
            .with_file(".hidden.html", PAGE.as_bytes()),
    );

    for (uri, status) in [
        ("/missing.html", StatusCode::NOT_FOUND),
        ("/.hidden.html", StatusCode::FORBIDDEN),
        ("/%zz.html", StatusCode::BAD_REQUEST),
    ] {
        // when
        let response = server.send("GET", uri, Some("document")).await;

        // then
        assert_eq!(response.status(), status, "{uri}");
        assert_eq!(
            header(&response, "content-type"),
            Some("text/plain; charset=utf-8"),
            "{uri}"
        );
        let text = String::from_utf8(body(response).await).expect("UTF-8");
        assert!(!text.contains("<script"), "{uri}: {text}");
    }
}

#[tokio::test]
async fn should_leave_source_bytes_unchanged_when_injected() {
    // given
    let server = Server::new(InMemorySite::new().with_file("index.html", PAGE.as_bytes()));
    let before = server.source("/index.html");

    // when
    let served = body(server.send("GET", "/", Some("document")).await).await;

    // then
    assert_ne!(served, before);
    assert_eq!(server.source("/index.html"), before);
}

#[tokio::test]
async fn should_not_label_utf8_when_meta_names_shift_jis() {
    // given: ASCII bytes, which are valid UTF-8, declared as Shift_JIS
    let page = b"<!doctype html><meta charset=shift_jis><body><p>ascii</p></body>";
    let server = Server::new(InMemorySite::new().with_file("sjis.html", page));

    // when
    let response = server.send("GET", "/sjis.html", Some("document")).await;

    // then
    assert_eq!(header(&response, "content-type"), Some("text/html"));
    assert_eq!(body(response).await, page);
    let warnings = server.warnings.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].0, "html-not-utf8");
}

#[tokio::test]
async fn should_serve_unchanged_and_warn_when_html_is_latin1() {
    // given
    let page = b"<!doctype html><body><p>caf\xe9</p></body>";
    let server = Server::new(InMemorySite::new().with_file("old.html", page));

    // when
    let response = server.send("GET", "/old.html", None).await;

    // then
    assert_eq!(header(&response, "content-type"), Some("text/html"));
    assert_eq!(body(response).await, page);
    assert_eq!(server.warnings.warnings()[0].0, "html-not-utf8");
}

#[tokio::test]
async fn should_serve_original_and_warn_once_when_rewrite_fails() {
    // given: the document ends inside a script, where an appended tag would become source
    let page = b"<html><body><script>var a = 1;";
    let server = Server::new(InMemorySite::new().with_file("broken.html", page));

    // when
    let first = server.send("GET", "/broken.html", Some("document")).await;
    let first_type = header(&first, "content-type").map(str::to_owned);
    let first_body = body(first).await;
    let second = body(server.send("GET", "/broken.html", None).await).await;

    // then
    assert_eq!(first_type.as_deref(), Some("text/html; charset=utf-8"));
    assert_eq!(first_body, page);
    assert_eq!(second, page);
    let warnings = server.warnings.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].0, "html-rewrite-failed");
    assert!(warnings[0].1.starts_with("/broken.html: "), "{warnings:?}");
}

#[tokio::test]
async fn should_preserve_meta_csp_bytes_when_injecting() {
    // given: a restrictive policy, with odd spacing and case that must survive untouched
    let meta = "<meta  HTTP-EQUIV='Content-Security-Policy' content=\"default-src 'none';  img-src 'self'\" >";
    let page = format!("<!doctype html><html><head>{meta}</head><body>hi</body></html>");
    let server = Server::new(InMemorySite::new().with_file("csp.html", page.as_bytes()));
    let tag = server.tag(&server.reload.current());

    // when
    let response = server.send("GET", "/csp.html", Some("document")).await;
    let csp_header = header(&response, "content-security-policy").map(str::to_owned);
    let served = body(response).await;

    // then: the meta element is byte-identical, no header was added, only the tag was inserted
    assert_eq!(csp_header, None);
    assert_eq!(
        String::from_utf8(served).expect("utf-8"),
        injected(&page, &tag)
    );
    assert_eq!(server.source("/csp.html"), page.as_bytes());
}

#[tokio::test]
async fn should_warn_once_per_path_when_meta_csp_blocks_reload() {
    // given
    let page = "<html><head><meta http-equiv=\"content-security-policy\" \
                content=\"default-src 'none'\"></head><body>hi</body></html>";
    let server = Server::new(InMemorySite::new().with_file("csp.html", page.as_bytes()));

    // when
    for _ in 0..2 {
        body(server.send("GET", "/csp.html", Some("document")).await).await;
    }
    body(server.send("HEAD", "/csp.html", Some("document")).await).await;

    // then
    let warnings = server.warnings.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].0, "csp-blocks-live-reload");
    assert!(warnings[0].1.starts_with("/csp.html: "), "{warnings:?}");
    assert!(warnings[0].1.contains("default-src"), "{warnings:?}");
}

#[tokio::test]
async fn should_not_warn_when_meta_csp_allows_self_or_is_not_a_head_policy() {
    // given: self allowed; a policy-looking meta in the body (browsers ignore it); no policy
    let allowed = "<html><head><meta http-equiv=\"Content-Security-Policy\" \
                   content=\"script-src 'self'; connect-src 'self'\"></head><body>a</body></html>";
    let in_body = "<html><head></head><body><meta http-equiv=\"Content-Security-Policy\" \
                   content=\"default-src 'none'\">b</body></html>";
    let other = "<html><head><meta http-equiv=\"refresh\" content=\"default-src 'none'\">\
                 </head><body>c</body></html>";
    let server = Server::new(
        InMemorySite::new()
            .with_file("allowed.html", allowed.as_bytes())
            .with_file("body.html", in_body.as_bytes())
            .with_file("other.html", other.as_bytes()),
    );

    // when
    for path in ["/allowed.html", "/body.html", "/other.html"] {
        body(server.send("GET", path, Some("document")).await).await;
    }

    // then
    assert!(server.warnings.warnings().is_empty());
}

#[tokio::test]
async fn should_hold_budget_until_body_is_consumed_when_html_served() {
    // given: a budget with room for one page of this size at a time
    let page = "<body>".to_owned() + &"x".repeat(256 * 1024) + "</body>";
    let budget = HtmlBudget::new(2 * page.len() as u64 + 64 * 1024);
    let full = budget.available_bytes();
    let server = Server::new(InMemorySite::new().with_file("index.html", page.as_bytes()))
        .with_budget(budget.clone());
    let held = server.send("GET", "/", None).await;
    assert_eq!(
        budget.available_bytes(),
        0,
        "an unread body keeps its reservation"
    );
    let app = server.app();
    let waiting = tokio::spawn(async move {
        let request = Request::builder()
            .header("host", "127.0.0.1")
            .uri("/")
            .body(Body::empty())
            .expect("request");
        app.oneshot(request).await.expect("a response")
    });

    // when: the first client goes away without reading
    drop(held);
    let second = waiting.await.expect("the request task");

    // then: the waiting request was served in full, and every reservation is back
    let sent = String::from_utf8(body(second).await).expect("UTF-8");
    assert!(sent.contains("<script defer src=\"/__site_preview/client.js\""));
    assert_eq!(
        sent.len(),
        page.len() + server.tag(&server.reload.current()).len()
    );
    assert_eq!(budget.available_bytes(), full);
}
