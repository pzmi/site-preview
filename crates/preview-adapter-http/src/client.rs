//! The embedded live-reload client script and its route.

use axum::body::Body;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;

/// The script text, compiled into the binary. Its unit tests run under `node --test`
/// (`assets/client.test.mjs`, the `client-js` stage).
pub(crate) const CLIENT_JS: &str = include_str!("../assets/client.js");

/// Answers `GET /__site_preview/client.js` (HEAD is served by the router with an empty body).
///
/// `no-store` and `nosniff` come from the outermost `harden` layer. Pinned by
/// `should_serve_embedded_client_when_route_requested` in `tests/client.rs`.
pub(crate) async fn client_js() -> Response {
    let mut response = Response::new(Body::from(CLIENT_JS));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    response
}
