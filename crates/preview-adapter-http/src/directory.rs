//! The redirect from a directory path to its slash form (plan.md §5.1, row 3).

use axum::body::Body;
use axum::http::header::LOCATION;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use preview_domain::{ChangedPath, RelPath, SitePath};

use crate::errors::plain;

/// `307 Temporary Redirect` from `/guide` to `/guide/`, so the page's relative URLs resolve
/// against the directory.
///
/// The `Location` path is built from `path`'s validated components, each percent-encoded as
/// the reload protocol encodes a changed file's URL ([`ChangedPath`]), never copied from the
/// raw request; it always starts with a single `/` followed by a non-empty component, so it
/// can never be read as a scheme-relative `//host` URL. The request's query, when present,
/// is re-attached unchanged. 307 rather than 301: nothing is cached and the method is kept.
/// Pinned by `should_redirect_with_307_when_directory_lacks_trailing_slash`,
/// `should_encode_location_when_directory_name_has_space` and
/// `should_preserve_query_when_redirecting_to_slash` in `tests/directory.rs`.
pub(crate) fn redirect_to_slash(path: &SitePath, query: Option<&str>) -> Response {
    let encoded = ChangedPath::new(&RelPath::new(path.components().to_vec()));
    let mut location = encoded.url_path().to_owned();
    if !location.ends_with('/') {
        location.push('/');
    }
    if let Some(query) = query {
        location.push('?');
        location.push_str(query);
    }
    let Ok(location) = HeaderValue::from_str(&location) else {
        return plain(StatusCode::BAD_REQUEST);
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    response.headers_mut().insert(LOCATION, location);
    response
}
