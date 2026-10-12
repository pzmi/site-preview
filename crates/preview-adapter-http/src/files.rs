//! The file fallback: resolve a request path through the application and map the outcome.

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use preview_application::{ContentFile, ResolveRequest, Resolved};

use crate::directory::redirect_to_slash;
use crate::errors::plain;
use crate::inject::response::{HtmlRequest, html_response};
use crate::mime::{MediaType, media_type};
use crate::state::AppState;
use crate::stream::stream_body;

/// Resolves `uri.path()` (never the query) on a blocking thread and maps the outcome.
///
/// A file answers 200 with `Content-Length` and, for GET, a streamed body (see
/// [`stream_body`]); HEAD answers the same headers with an empty body and, except for HTML
/// (whose headers depend on its bytes), reads nothing. A
/// directory without its trailing slash is redirected (see [`redirect_to_slash`]), with the
/// request's query re-attached; a directory index is served like any file. No `Accept-Ranges`
/// is sent and a `Range` request header is never read, so a range request gets the full 200
/// (plan.md §5.1). The `Content-Type` comes from the served file's own name (see
/// [`crate::mime`]); HTML is handled by [`html_response`], with the snapshot the query read
/// before the open (plan.md §7.3).
pub(crate) async fn serve(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let path = uri.path().to_owned();
    let injection = state.injection.clone();
    let resolved = tokio::task::spawn_blocking(move || {
        ResolveRequest { raw_path: &path }
            .execute(state.content.as_ref(), || state.reload.current())
    })
    .await;
    let Ok(resolved) = resolved else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR);
    };
    match resolved {
        Resolved::File {
            file,
            snapshot,
            name,
        } => match media_type(name.as_str()) {
            MediaType::Fixed(content_type) => file_response(file, &method, content_type),
            MediaType::Html => {
                let request = HtmlRequest {
                    method: &method,
                    headers: &headers,
                    raw_path: uri.path(),
                };
                html_response(&injection, &request, file, snapshot).await
            }
        },
        Resolved::BadRequest => plain(StatusCode::BAD_REQUEST),
        Resolved::NotFound => plain(StatusCode::NOT_FOUND),
        Resolved::Forbidden(_) => plain(StatusCode::FORBIDDEN),
        Resolved::Unavailable(_) => plain(StatusCode::INTERNAL_SERVER_ERROR),
        Resolved::RedirectToSlash { path } => redirect_to_slash(&path, uri.query()),
    }
}

fn file_response(
    file: Box<dyn ContentFile>,
    method: &Method,
    content_type: &'static str,
) -> Response {
    let len = file.len();
    let body = if *method == Method::HEAD {
        // The handle is dropped unread: HEAD does no file I/O beyond the open.
        Body::empty()
    } else {
        stream_body(file, len)
    };
    with_headers(body, content_type, len)
}

fn with_headers(body: Body, content_type: &'static str, len: u64) -> Response {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(len));
    response
}
