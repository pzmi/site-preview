//! File streaming tests, driven in process with `tower::ServiceExt::oneshot`.

use std::io::{self, Cursor, Read};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Injection, Shutdown, StreamLimit,
    build_router,
};
use preview_application::{ContentFile, ContentRefusal, Opened, ReloadState, SiteContent};
use preview_domain::{BoundAddr, HostPolicy, SitePath};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

fn app(content: Arc<dyn SiteContent>) -> Router {
    let mut ids = FixedInstanceIds::new(1, 1);
    build_router(AppState {
        content,
        reload: ReloadState::new(ids.next_id()),
        host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
        shutdown: Shutdown::new().signal(),
        keep_alive: DEFAULT_KEEP_ALIVE,
        streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
        injection: Injection::default(),
    })
}

fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .header("host", "127.0.0.1")
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// Deterministic pseudo-random bytes (xorshift), so the test needs no rng crate.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn should_stream_bytes_unchanged_when_get_file() {
    // given
    let data = noise(5 * 1024 * 1024);
    let site = InMemorySite::new().with_file("big.bin", &data);

    // when
    let response = app(Arc::new(site))
        .oneshot(request("GET", "/big.bin"))
        .await
        .unwrap();

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-length"), Some("5242880"));
    assert_eq!(header(&response, "cache-control"), Some("no-store"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        body.as_ref() == data.as_slice(),
        "body differs from the file"
    );
}

#[tokio::test]
async fn should_return_headers_without_body_when_head() {
    // given
    let site = InMemorySite::new().with_file("a.bin", &noise(100_000));

    // when
    let response = app(Arc::new(site))
        .oneshot(request("HEAD", "/a.bin"))
        .await
        .unwrap();

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-length"), Some("100000"));
    assert_eq!(header(&response, "cache-control"), Some("no-store"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty());
}

#[tokio::test]
async fn should_return_full_200_when_range_requested() {
    // given
    let data = noise(10_000);
    let site = InMemorySite::new().with_file("a.bin", &data);
    let req = Request::builder()
        .header("host", "127.0.0.1")
        .uri("/a.bin")
        .header("range", "bytes=10-19")
        .body(Body::empty())
        .unwrap();

    // when
    let response = app(Arc::new(site)).oneshot(req).await.unwrap();

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("content-range").is_none());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), data.as_slice());
}

#[tokio::test]
async fn should_omit_accept_ranges_when_serving() {
    // given
    let site = InMemorySite::new().with_file("a.txt", b"hello");

    for method in ["GET", "HEAD"] {
        // when
        let response = app(Arc::new(site.clone()))
            .oneshot(request(method, "/a.txt"))
            .await
            .unwrap();

        // then
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers().get("accept-ranges").is_none(),
            "{method}"
        );
    }
}

#[tokio::test]
async fn should_serve_empty_body_when_file_is_empty() {
    // given
    let site = InMemorySite::new().with_file("e.txt", b"");

    // when
    let response = app(Arc::new(site))
        .oneshot(request("GET", "/e.txt"))
        .await
        .unwrap();

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-length"), Some("0"));
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

/// A site whose single file reports `claimed` bytes but holds `data`.
struct Lying {
    data: Vec<u8>,
    claimed: u64,
}

struct LyingFile {
    cursor: Cursor<Vec<u8>>,
    claimed: u64,
}

impl Read for LyingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.cursor.read(buf)
    }
}

impl ContentFile for LyingFile {
    fn len(&self) -> u64 {
        self.claimed
    }
}

impl SiteContent for Lying {
    fn open(&self, _path: &SitePath) -> Result<Opened, ContentRefusal> {
        Ok(Opened::File(Box::new(LyingFile {
            cursor: Cursor::new(self.data.clone()),
            claimed: self.claimed,
        })))
    }
}

#[tokio::test]
async fn should_abort_body_when_file_shorter_than_length() {
    // given: the file shrank after fstat
    let content = Lying {
        data: noise(1000),
        claimed: 5000,
    };

    // when
    let response = app(Arc::new(content))
        .oneshot(request("GET", "/x.bin"))
        .await
        .unwrap();
    let result = response.into_body().collect().await;

    // then: an error, never a short body taken for a complete one
    assert!(result.is_err());
}

#[tokio::test]
async fn should_not_exceed_length_when_file_grew() {
    // given: the file grew after fstat
    let data = noise(5000);
    let content = Lying {
        data: data.clone(),
        claimed: 1000,
    };

    // when
    let response = app(Arc::new(content))
        .oneshot(request("GET", "/x.bin"))
        .await
        .unwrap();

    // then
    assert_eq!(header(&response, "content-length"), Some("1000"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), &data[..1000]);
}

#[tokio::test]
async fn should_omit_utf8_charset_when_html_is_latin1() {
    // given: "café" in ISO-8859-1, not valid UTF-8
    let page = b"<!doctype html><meta charset=iso-8859-1><p>caf\xe9</p>";
    let site = InMemorySite::new().with_file("old.html", page);

    // when
    let response = app(Arc::new(site))
        .oneshot(request("GET", "/old.html"))
        .await
        .unwrap();

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), Some("text/html"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), page.as_slice());
}

#[tokio::test]
async fn should_label_utf8_charset_when_html_is_utf8() {
    // given
    let page = "<!doctype html><p>café</p>".as_bytes();
    let site = InMemorySite::new().with_file("index.html", page);
    let mut fetch = request("GET", "/");
    // A script fetch, so the page is not injected and the label is checked on its own.
    fetch
        .headers_mut()
        .insert("sec-fetch-dest", "empty".parse().expect("header value"));

    // when
    let response = app(Arc::new(site)).oneshot(fetch).await.unwrap();

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(header(&response, "content-length"), Some("27"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), page);
}

#[tokio::test]
async fn should_label_same_content_type_when_head_of_html() {
    // given
    let site = InMemorySite::new()
        .with_file("utf8.html", "<p>ok ✓</p>".as_bytes())
        .with_file("latin1.html", b"<p>caf\xe9</p>");

    for (path, expected) in [
        ("/utf8.html", "text/html; charset=utf-8"),
        ("/latin1.html", "text/html"),
    ] {
        // when
        let response = app(Arc::new(site.clone()))
            .oneshot(request("HEAD", path))
            .await
            .unwrap();

        // then
        assert_eq!(header(&response, "content-type"), Some(expected), "{path}");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn should_omit_utf8_charset_when_html_exceeds_eight_mebibytes() {
    // given: valid UTF-8, but too large to check, so its encoding is unknown
    let page = vec![b'a'; 8 * 1024 * 1024 + 1];
    let site = InMemorySite::new().with_file("huge.html", &page);

    // when
    let response = app(Arc::new(site))
        .oneshot(request("GET", "/huge.html"))
        .await
        .unwrap();

    // then
    assert_eq!(header(&response, "content-type"), Some("text/html"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.len(), page.len());
}

#[tokio::test]
async fn should_label_content_type_and_nosniff_when_serving_by_extension() {
    // given
    let site = InMemorySite::new()
        .with_file("styles.css", b"p{}")
        .with_file("routes.gpx", b"<gpx/>")
        .with_file("blob.unknownext", b"?");

    for (path, expected) in [
        ("/styles.css", "text/css; charset=utf-8"),
        ("/routes.gpx", "application/gpx+xml"),
        ("/blob.unknownext", "application/octet-stream"),
    ] {
        // when
        let response = app(Arc::new(site.clone()))
            .oneshot(request("GET", path))
            .await
            .unwrap();

        // then
        assert_eq!(header(&response, "content-type"), Some(expected), "{path}");
        assert_eq!(
            header(&response, "x-content-type-options"),
            Some("nosniff"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn should_answer_500_when_html_shorter_than_length() {
    // given: an HTML file that shrank after fstat
    let content = Lying {
        data: b"<p>short</p>".to_vec(),
        claimed: 5000,
    };

    // when
    let response = app(Arc::new(content))
        .oneshot(request("GET", "/page.html"))
        .await
        .unwrap();

    // then
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("short"));
}
