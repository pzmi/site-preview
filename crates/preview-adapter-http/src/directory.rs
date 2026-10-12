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
/// is re-attached unchanged except that bytes outside printable ASCII (controls and
/// non-ASCII) are percent-encoded, so no raw non-ASCII byte reaches the header. 307 rather than 301: nothing is cached and the method is kept.
/// Pinned by `should_redirect_with_307_when_directory_lacks_trailing_slash`,
/// `should_encode_location_when_directory_name_has_space` and
/// `should_preserve_query_when_redirecting_to_slash` in `tests/directory.rs`; the query escape by `should_percent_encode_non_ascii_when_query_has_raw_bytes` (unit, below).
pub(crate) fn redirect_to_slash(path: &SitePath, query: Option<&str>) -> Response {
    let encoded = ChangedPath::new(&RelPath::new(path.components().to_vec()));
    let mut location = encoded.url_path().to_owned();
    if !location.ends_with('/') {
        location.push('/');
    }
    if let Some(query) = query {
        location.push('?');
        push_query(&mut location, query);
    }
    let Ok(location) = HeaderValue::from_str(&location) else {
        return plain(StatusCode::BAD_REQUEST);
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    response.headers_mut().insert(LOCATION, location);
    response
}

/// Appends `query`, percent-encoding every byte that is a control or not ASCII.
fn push_query(location: &mut String, query: &str) {
    for byte in query.bytes() {
        if byte.is_ascii_graphic() {
            location.push(char::from(byte));
        } else {
            location.push_str(&format!("%{byte:02X}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::push_query;

    #[test]
    fn should_percent_encode_non_ascii_when_query_has_raw_bytes() {
        // given
        let query = "q=caf\u{e9}&a=%20b\u{1}\u{7f} c";
        let mut location = String::from("/guide/?");

        // when
        push_query(&mut location, query);

        // then
        assert_eq!(location, "/guide/?q=caf%C3%A9&a=%20b%01%7F%20c");
    }

    #[test]
    fn should_keep_query_unchanged_when_it_is_printable_ascii() {
        // given
        let query = "step=2&q=a%20b&x=/?:@!$'()*+,;=-._~";
        let mut location = String::new();

        // when
        push_query(&mut location, query);

        // then
        assert_eq!(location, query);
    }
}
