//! Fixed, minimal error responses.
//!
//! Every body is a constant: nothing from the request (path, query, header) and nothing from
//! the filesystem reaches it, so an error reveals neither what exists nor what was asked.
//! Pinned by `should_not_echo_request_when_error_response` in `tests/router.rs`.

use axum::body::Body;
use axum::http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

/// A plain-text response whose body is the status line, e.g. `404 Not Found`; a 421 says
/// `Host not allowed` (plan.md §5.4, HTTP-05).
pub(crate) fn plain(status: StatusCode) -> Response {
    let body = match status {
        StatusCode::BAD_REQUEST => "400 Bad Request",
        StatusCode::FORBIDDEN => "403 Forbidden",
        StatusCode::NOT_FOUND => "404 Not Found",
        StatusCode::METHOD_NOT_ALLOWED => "405 Method Not Allowed",
        StatusCode::MISDIRECTED_REQUEST => "Host not allowed",
        StatusCode::SERVICE_UNAVAILABLE => "503 Service Unavailable",
        _ => "500 Internal Server Error",
    };
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = if body.starts_with("500") {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        status
    };
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// Rejects every method but GET and HEAD with `405` and `Allow: GET, HEAD` (plan.md §5.1).
pub(crate) async fn method_gate(request: axum::extract::Request, next: Next) -> Response {
    if matches!(*request.method(), Method::GET | Method::HEAD) {
        return next.run(request).await;
    }
    let mut response = plain(StatusCode::METHOD_NOT_ALLOWED);
    response
        .headers_mut()
        .insert(ALLOW, HeaderValue::from_static("GET, HEAD"));
    response
}

/// Sets `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every response.
///
/// Outermost layer, so the 405 from the method gate and every handler's response carry them.
/// It adds no CORS header: a page on another origin gets no cross-origin read access.
pub(crate) async fn harden(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}
