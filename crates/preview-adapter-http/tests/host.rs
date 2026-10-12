//! The Host authority gate, driven in process with `tower::ServiceExt::oneshot`.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::body::Body;
use axum::http::request::Builder;
use axum::http::{HeaderValue, Request, Response, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Shutdown, StreamLimit, build_router,
};
use preview_application::ReloadState;
use preview_domain::{BoundAddr, HostPolicy};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

const PORT: u16 = 49_152;

fn site() -> InMemorySite {
    InMemorySite::new().with_file("index.html", b"<p>hi</p>")
}

async fn send_to(
    site: &InMemorySite,
    bound: IpAddr,
    allow_remote: bool,
    request: Request<Body>,
) -> (Response<Body>, String) {
    let mut ids = FixedInstanceIds::new(1, 1);
    let app = build_router(AppState {
        content: Arc::new(site.clone()),
        reload: ReloadState::new(ids.next_id()),
        host: HostPolicy::new(BoundAddr::new(bound, PORT), allow_remote),
        shutdown: Shutdown::new().signal(),
        keep_alive: DEFAULT_KEEP_ALIVE,
        streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
    });
    let response = app.oneshot(request).await.expect("response");
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.expect("body").to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (Response::from_parts(parts, Body::empty()), text)
}

async fn send(site: &InMemorySite, request: Request<Body>) -> (Response<Body>, String) {
    send_to(site, IpAddr::V4(Ipv4Addr::LOCALHOST), false, request).await
}

fn get(uri: &str) -> Builder {
    Request::builder().method("GET").uri(uri)
}

fn with_host(host: &str) -> Request<Body> {
    get("/")
        .header("host", host)
        .body(Body::empty())
        .expect("request")
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

fn assert_refused(response: &Response<Body>, body: &str, case: &str) {
    assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST, "{case}");
    assert_eq!(body, "Host not allowed", "{case}");
    assert_eq!(
        header(response, "content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        header(response, "cache-control"),
        Some("no-store"),
        "{case}"
    );
    assert_eq!(header(response, "x-content-type-options"), Some("nosniff"));
}

fn has_cors_header(response: &Response<Body>) -> bool {
    response
        .headers()
        .keys()
        .any(|k| k.as_str().starts_with("access-control-"))
}

#[tokio::test]
async fn should_reject_with_421_when_host_is_rebinding_name() {
    for host in [
        "evil.example",
        &format!("evil.example:{PORT}"),
        &format!("127.0.0.1.nip.io:{PORT}"),
        &format!("localhost.:{PORT}"),
        &format!("app.localhost:{PORT}"),
        &format!("127.1:{PORT}"),
    ] {
        // given
        let site = site();

        // when
        let (response, body) = send(&site, with_host(host)).await;

        // then
        assert_refused(&response, &body, host);
        assert!(!body.contains("evil"), "the header is never echoed");
        assert_eq!(site.open_count(), 0, "{host}");
    }
}

#[tokio::test]
async fn should_reject_when_host_port_differs() {
    for host in [
        "127.0.0.1".to_owned(),
        format!("127.0.0.1:{}", PORT + 1),
        format!("localhost:{}", PORT - 1),
        format!("127.0.0.1:0{PORT}"),
    ] {
        // given
        let site = site();

        // when
        let (response, body) = send(&site, with_host(&host)).await;

        // then
        assert_refused(&response, &body, &host);
        assert_eq!(site.open_count(), 0, "{host}");
    }
}

#[tokio::test]
async fn should_accept_when_host_is_bound_ip() {
    // given
    let site = site();

    // when
    let (response, body) = send(&site, with_host(&format!("127.0.0.1:{PORT}"))).await;

    // then
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body, "<p>hi</p>");
}

#[tokio::test]
async fn should_accept_when_host_is_localhost_alias_on_loopback() {
    for host in [format!("localhost:{PORT}"), format!("LocalHost:{PORT}")] {
        // given
        let site = site();

        // when
        let (response, _) = send(&site, with_host(&host)).await;

        // then
        assert_eq!(response.status(), StatusCode::OK, "{host}");
    }
}

#[tokio::test]
async fn should_accept_when_host_is_bound_ipv6_ip() {
    // given
    let site = site();
    let request = with_host(&format!("[::1]:{PORT}"));

    // when
    let (response, _) = send_to(
        &site,
        IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        false,
        request,
    )
    .await;

    // then
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn should_reject_with_421_when_host_missing() {
    // given
    let site = site();
    let request = get("/").body(Body::empty()).expect("request");

    // when
    let (response, body) = send(&site, request).await;

    // then
    assert_refused(&response, &body, "missing");
}

#[tokio::test]
async fn should_reject_with_421_when_host_header_is_duplicated() {
    let good = format!("127.0.0.1:{PORT}");
    for second in [good.as_str(), "evil.example"] {
        // given: the first value alone would be accepted
        let site = site();
        let request = get("/")
            .header("host", good.as_str())
            .header("host", second)
            .body(Body::empty())
            .expect("request");

        // when
        let (response, body) = send(&site, request).await;

        // then
        assert_refused(&response, &body, second);
    }
}

#[tokio::test]
async fn should_reject_with_421_when_host_is_not_utf8() {
    // given
    let site = site();
    let mut raw = b"127.0.0.1:".to_vec();
    raw.extend_from_slice(PORT.to_string().as_bytes());
    raw.push(0xFF);
    let request = get("/")
        .header(
            "host",
            HeaderValue::from_bytes(&raw).expect("obs-text is a valid value"),
        )
        .body(Body::empty())
        .expect("request");

    // when
    let (response, body) = send(&site, request).await;

    // then
    assert_refused(&response, &body, "non-UTF-8");
}

#[tokio::test]
async fn should_reject_with_421_when_absolute_target_disagrees_with_host() {
    let good = format!("127.0.0.1:{PORT}");
    for target in [
        "http://evil.example/".to_owned(),
        format!("http://localhost:{PORT}/"),
        format!("http://127.0.0.1:{}/", PORT + 1),
        format!("http://user@127.0.0.1:{PORT}/"),
    ] {
        // given: a valid Host, but the request line names another authority
        let site = site();
        let request = get(&target)
            .header("host", good.as_str())
            .body(Body::empty())
            .expect("request");

        // when
        let (response, body) = send(&site, request).await;

        // then
        assert_refused(&response, &body, &target);
        assert_eq!(site.open_count(), 0, "{target}");
    }
}

#[tokio::test]
async fn should_reject_with_421_when_absolute_target_scheme_is_not_http() {
    let good = format!("127.0.0.1:{PORT}");
    for scheme in ["https", "HTTPS", "ftp", "ws", "file"] {
        // given: the authorities agree, but the request line names another protocol
        let site = site();
        let request = get(&format!("{scheme}://{good}/"))
            .header("host", good.as_str())
            .body(Body::empty())
            .expect("request");

        // when
        let (response, body) = send(&site, request).await;

        // then
        assert_refused(&response, &body, scheme);
        assert_eq!(site.open_count(), 0, "{scheme}");
    }
}

#[tokio::test]
async fn should_accept_when_absolute_target_scheme_is_uppercase_http() {
    // given
    let site = site();
    let request = get(&format!("HTTP://127.0.0.1:{PORT}/"))
        .header("host", format!("127.0.0.1:{PORT}"))
        .body(Body::empty())
        .expect("request");

    // when
    let (response, _) = send(&site, request).await;

    // then
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn should_accept_when_absolute_target_matches_host() {
    // given
    let site = site();
    let request = get(&format!("http://127.0.0.1:{PORT}/"))
        .header("host", format!("127.0.0.1:{PORT}"))
        .body(Body::empty())
        .expect("request");

    // when
    let (response, _) = send(&site, request).await;

    // then
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn should_reject_with_421_when_internal_route_has_bad_host() {
    for uri in [
        "/__site_preview",
        "/__site_preview/events",
        "/__site_preview/client.js",
    ] {
        // given
        let site = site();
        let request = get(uri)
            .header("host", "evil.example")
            .body(Body::empty())
            .expect("request");

        // when
        let (response, body) = send(&site, request).await;

        // then
        assert_refused(&response, &body, uri);
    }
}

#[tokio::test]
async fn should_reject_with_421_before_405_when_post_has_bad_host() {
    // given
    let site = site();
    let request = Request::builder()
        .method("POST")
        .uri("/")
        .header("host", "evil.example")
        .body(Body::empty())
        .expect("request");

    // when
    let (response, body) = send(&site, request).await;

    // then
    assert_refused(&response, &body, "POST");
    assert_eq!(header(&response, "allow"), None);
}

#[tokio::test]
async fn should_reject_localhost_when_wildcard_listener_with_allow_remote() {
    // given
    let site = site();
    let wildcard = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

    // when
    let (refused, body) = send_to(
        &site,
        wildcard,
        true,
        with_host(&format!("localhost:{PORT}")),
    )
    .await;
    let (lan, _) = send_to(
        &site,
        wildcard,
        true,
        with_host(&format!("192.168.1.20:{PORT}")),
    )
    .await;

    // then
    assert_refused(&refused, &body, "localhost on wildcard");
    assert_eq!(lan.status(), StatusCode::OK);
}

#[tokio::test]
async fn should_not_emit_cors_headers_when_origin_sent() {
    let good = format!("127.0.0.1:{PORT}");
    for (method, host) in [
        ("GET", good.as_str()),
        ("OPTIONS", good.as_str()),
        ("GET", "evil.example"),
        ("OPTIONS", "evil.example"),
    ] {
        // given: a cross-origin request, preflight included
        let site = site();
        let request = Request::builder()
            .method(method)
            .uri("/")
            .header("host", host)
            .header("origin", "http://evil.example")
            .header("access-control-request-method", "GET")
            .body(Body::empty())
            .expect("request");

        // when
        let (response, _) = send(&site, request).await;

        // then
        assert!(!has_cors_header(&response), "{method} {host}");
    }
}
