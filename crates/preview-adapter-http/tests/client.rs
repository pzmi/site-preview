//! The embedded client script route.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Shutdown, StreamLimit, build_router,
};
use preview_application::ReloadState;
use preview_domain::{BoundAddr, HostPolicy};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

async fn request(method: &str, host: &str) -> (axum::http::response::Parts, String) {
    let site = InMemorySite::new().with_file("index.html", b"x");
    let mut ids = FixedInstanceIds::new(1, 1);
    let app = build_router(AppState {
        content: Arc::new(site.clone()),
        reload: ReloadState::new(ids.next_id()),
        host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
        shutdown: Shutdown::new().signal(),
        keep_alive: DEFAULT_KEEP_ALIVE,
        streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
    });
    let request = Request::builder()
        .header("host", host)
        .method(method)
        .uri("/__site_preview/client.js")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.expect("body").to_bytes();
    (parts, String::from_utf8_lossy(&bytes).into_owned())
}

fn header<'a>(parts: &'a axum::http::response::Parts, name: &str) -> Option<&'a str> {
    parts.headers.get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn should_serve_embedded_client_when_route_requested() {
    // given
    let embedded = include_str!("../assets/client.js");

    // when
    let (parts, body) = request("GET", "127.0.0.1").await;

    // then
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(
        header(&parts, "content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(header(&parts, "cache-control"), Some("no-store"));
    assert_eq!(header(&parts, "x-content-type-options"), Some("nosniff"));
    assert_eq!(body, embedded);
}

#[tokio::test]
async fn should_serve_headers_without_body_when_head_requested() {
    // given
    // when
    let (parts, body) = request("HEAD", "127.0.0.1").await;

    // then
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(
        header(&parts, "content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(body, "");
}

#[tokio::test]
async fn should_reject_client_route_when_host_not_allowed() {
    // given
    // when
    let (parts, _) = request("GET", "evil.example").await;

    // then
    assert_eq!(parts.status, StatusCode::MISDIRECTED_REQUEST);
}

#[tokio::test]
async fn should_reject_client_route_when_method_is_post() {
    // given
    // when
    let (parts, _) = request("POST", "127.0.0.1").await;

    // then
    assert_eq!(parts.status, StatusCode::METHOD_NOT_ALLOWED);
}
