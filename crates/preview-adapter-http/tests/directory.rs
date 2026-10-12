//! Directory tests: the slash redirect, nested indexes and the absence of listings, driven in
//! process with `tower::ServiceExt::oneshot`.

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
use preview_application::ReloadState;
use preview_domain::{BoundAddr, HostPolicy};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

fn app(site: &InMemorySite) -> Router {
    let mut ids = FixedInstanceIds::new(1, 1);
    build_router(AppState {
        content: Arc::new(site.clone()),
        reload: ReloadState::new(ids.next_id()),
        host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
        shutdown: Shutdown::new().signal(),
        keep_alive: DEFAULT_KEEP_ALIVE,
        streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
        injection: Injection::default(),
    })
}

async fn send(site: &InMemorySite, method: &str, uri: &str) -> Response<Body> {
    let request = Request::builder()
        .header("host", "127.0.0.1")
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    app(site).oneshot(request).await.expect("a response")
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

#[tokio::test]
async fn should_redirect_with_307_when_directory_lacks_trailing_slash() {
    // given
    let site = InMemorySite::new()
        .with_file("guide/index.html", b"<h1>Guide</h1>")
        .with_file("docs/api/index.html", b"<h1>API</h1>");

    for (method, uri, location) in [
        ("GET", "/guide", "/guide/"),
        ("HEAD", "/guide", "/guide/"),
        ("GET", "/docs/api", "/docs/api/"),
    ] {
        // when
        let response = send(&site, method, uri).await;

        // then
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT, "{uri}");
        assert_eq!(header(&response, "location"), Some(location), "{uri}");
        assert_eq!(header(&response, "cache-control"), Some("no-store"));
        assert!(body(response).await.is_empty(), "{uri}");
    }
}

#[tokio::test]
async fn should_preserve_query_when_redirecting_to_slash() {
    // given
    let site = InMemorySite::new().with_file("guide/index.html", b"<h1>Guide</h1>");

    // when
    let response = send(&site, "GET", "/guide?step=2&q=a%20b").await;

    // then
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        header(&response, "location"),
        Some("/guide/?step=2&q=a%20b")
    );
}

#[tokio::test]
async fn should_encode_location_when_directory_name_has_space() {
    // given
    let site = InMemorySite::new()
        .with_file("my dir/index.html", b"<p>space</p>")
        .with_file("caf\u{e9}/index.html", b"<p>accent</p>");

    for (uri, location) in [("/my%20dir", "/my%20dir/"), ("/caf%C3%A9", "/caf%C3%A9/")] {
        // when
        let response = send(&site, "GET", uri).await;

        // then
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT, "{uri}");
        assert_eq!(header(&response, "location"), Some(location), "{uri}");
    }
}

#[tokio::test]
async fn should_build_location_from_components_when_request_spells_name_encoded() {
    // given: `%67` is `g`; the raw spelling never reaches the header
    let site = InMemorySite::new().with_file("guide/index.html", b"<h1>Guide</h1>");

    // when
    let response = send(&site, "GET", "/%67uide").await;

    // then
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header(&response, "location"), Some("/guide/"));
}

#[tokio::test]
async fn should_serve_nested_index_when_trailing_slash() {
    // given
    let site = InMemorySite::new()
        .with_file("index.html", b"<h1>Home</h1>")
        .with_file("docs/api/index.html", b"<h1>API</h1>");

    // when
    let response = send(&site, "GET", "/docs/api/").await;

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        Some("text/html; charset=utf-8")
    );
    // The page is injected (no Sec-Fetch-Dest), so only its start is the file's bytes.
    assert!(body(response).await.starts_with(b"<h1>API</h1><script "));
}

#[tokio::test]
async fn should_return_404_without_listing_when_index_missing() {
    // given
    let site = InMemorySite::new()
        .with_file("files/secret-name.txt", b"x")
        .with_dir("files/sub");

    // when
    let response = send(&site, "GET", "/files/").await;

    // then
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let text = String::from_utf8(body(response).await).expect("a text body");
    assert_eq!(text, "404 Not Found");
}
