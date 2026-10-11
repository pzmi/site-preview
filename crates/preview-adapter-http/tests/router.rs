//! In-process tests of the router, driven with `tower::ServiceExt::oneshot`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{AppState, build_router};
use preview_application::ReloadState;
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

fn app(site: &InMemorySite) -> Router {
    let mut ids = FixedInstanceIds::new(1, 1);
    build_router(AppState {
        content: Arc::new(site.clone()),
        reload: ReloadState::new(ids.next_id()),
    })
}

async fn send(site: &InMemorySite, method: &str, uri: &str) -> (Response<Body>, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let response = app(site).oneshot(request).await.expect("response");
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.expect("body").to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (Response::from_parts(parts, Body::empty()), text)
}

fn header<'a>(response: &'a Response<Body>, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn should_return_405_with_allow_when_method_is_post() {
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        // given
        let site = InMemorySite::new().with_file("index.html", b"x");

        // when
        let (response, body) = send(&site, method, "/").await;

        // then
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
        assert_eq!(header(&response, "allow"), Some("GET, HEAD"), "{method}");
        assert_eq!(body, "405 Method Not Allowed");
        assert_eq!(site.open_count(), 0, "{method}");
    }
}

#[tokio::test]
async fn should_return_405_when_method_is_post_on_internal_path() {
    // given
    let site = InMemorySite::new();

    // when
    let (response, _) = send(&site, "POST", "/__site_preview/events").await;

    // then
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header(&response, "allow"), Some("GET, HEAD"));
}

#[tokio::test]
async fn should_not_touch_disk_when_internal_path_requested() {
    // given: files on disk that share the reserved name must stay unreachable
    let site = InMemorySite::new()
        .with_file("__site_preview/client.js", b"secret")
        .with_file("__site_preview", b"secret");

    for method in ["GET", "HEAD"] {
        for uri in [
            "/__site_preview",
            "/__site_preview/",
            "/__site_preview/client.js",
            "/__site_preview/a/b/c?x=1",
        ] {
            // when
            let (response, body) = send(&site, method, uri).await;

            // then
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
            assert_eq!(body, if method == "GET" { "404 Not Found" } else { "" });
        }
    }
    assert_eq!(site.open_count(), 0);
}

#[tokio::test]
async fn should_not_touch_disk_when_reserved_name_is_percent_encoded() {
    // given
    let site = InMemorySite::new().with_file("__site_preview/client.js", b"secret");

    for uri in ["/%5F%5Fsite_preview/client.js", "/__SITE_PREVIEW/client.js"] {
        // when
        let (response, body) = send(&site, "GET", uri).await;

        // then
        assert!(
            matches!(
                response.status(),
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ),
            "{uri}"
        );
        assert!(!body.contains("secret"));
    }
    assert_eq!(site.open_count(), 0);
}

#[tokio::test]
async fn should_set_no_store_when_error_response() {
    // given
    let site = InMemorySite::new().with_io_error("broken.txt", std::io::ErrorKind::Other);
    let cases = [
        ("GET", "/missing.txt", StatusCode::NOT_FOUND),
        ("GET", "/.env", StatusCode::FORBIDDEN),
        ("GET", "/bad%zz", StatusCode::BAD_REQUEST),
        ("GET", "/broken.txt", StatusCode::INTERNAL_SERVER_ERROR),
        ("GET", "/__site_preview/x", StatusCode::NOT_FOUND),
        ("POST", "/", StatusCode::METHOD_NOT_ALLOWED),
    ];

    for (method, uri, status) in cases {
        // when
        let (response, body) = send(&site, method, uri).await;

        // then
        assert_eq!(response.status(), status, "{uri}");
        assert_eq!(
            header(&response, "cache-control"),
            Some("no-store"),
            "{uri}"
        );
        assert_eq!(header(&response, "x-content-type-options"), Some("nosniff"));
        assert_eq!(
            header(&response, "content-type"),
            Some("text/plain; charset=utf-8")
        );
        assert!(
            response
                .headers()
                .keys()
                .all(|k| !k.as_str().starts_with("access-control-"))
        );
        assert!(body.starts_with(&status.as_u16().to_string()), "{body}");
    }
}

#[tokio::test]
async fn should_not_echo_request_when_error_response() {
    // given
    let site = InMemorySite::new();

    for uri in [
        "/unique-missing-name?token=hunter2",
        "/.git/unique-secret?token=hunter2",
        "/bad%zzunique?token=hunter2",
    ] {
        // when
        let (_, body) = send(&site, "GET", uri).await;

        // then
        assert!(!body.contains("unique"), "{body}");
        assert!(!body.contains("hunter2"), "{body}");
        assert!(body.len() < 40, "{body}");
    }
}

#[tokio::test]
async fn should_not_pass_query_to_resolution_when_path_has_query() {
    // given
    let site = InMemorySite::new().with_file("page.html", b"x");

    // when: a query that would be an invalid path if it were not stripped
    let (response, _) = send(&site, "GET", "/page.html?a=%zz/../..").await;

    // then: the file resolves (stub 501 until streaming lands), not 400/404
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn should_return_404_when_file_missing_on_head() {
    // given
    let site = InMemorySite::new();

    // when
    let (response, body) = send(&site, "HEAD", "/nope").await;

    // then
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body, "");
}
