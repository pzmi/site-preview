//! The file fallback: resolve a request path through the application and map the outcome.

use std::io::{self, ErrorKind, Read};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use preview_application::{ContentFile, ResolveRequest, Resolved};
use preview_domain::MAX_INJECTION_INPUT_BYTES;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::errors::plain;
use crate::mime::{MediaType, html_content_type, media_type};
use crate::state::AppState;

/// Bytes read per blocking call. Small enough to bound memory per connection, large enough
/// to keep the per-chunk overhead negligible.
const CHUNK: usize = 64 * 1024;

/// Chunks queued between the reader and the connection. Together with [`CHUNK`] this bounds
/// the memory a slow client can pin to a few chunks, whatever the file's size.
const QUEUE: usize = 2;

/// Resolves `uri.path()` (never the query) on a blocking thread and maps the outcome.
///
/// A file answers 200 with `Content-Length` and, for GET, a streamed body (see
/// [`stream_body`]); HEAD answers the same headers with an empty body and reads nothing. The
/// directory redirect (HTTP-04) is not here yet and answers a fixed 501. No `Accept-Ranges`
/// is sent and a `Range` request header is never read, so a range request gets the full 200
/// (plan.md §5.1). The `Content-Type` comes from the served file's own name (see
/// [`crate::mime`]); HTML is handled by [`html_response`].
pub(crate) async fn serve(State(state): State<AppState>, method: Method, uri: Uri) -> Response {
    let path = uri.path().to_owned();
    let resolved = tokio::task::spawn_blocking(move || {
        ResolveRequest { raw_path: &path }
            .execute(state.content.as_ref(), || state.reload.current())
    })
    .await;
    let Ok(resolved) = resolved else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR);
    };
    match resolved {
        Resolved::File { file, name, .. } => match media_type(name.as_str()) {
            MediaType::Fixed(content_type) => file_response(file, &method, content_type),
            MediaType::Html => html_response(file, &method).await,
        },
        Resolved::BadRequest => plain(StatusCode::BAD_REQUEST),
        Resolved::NotFound => plain(StatusCode::NOT_FOUND),
        Resolved::Forbidden(_) => plain(StatusCode::FORBIDDEN),
        Resolved::Unavailable(_) => plain(StatusCode::INTERNAL_SERVER_ERROR),
        Resolved::RedirectToSlash { .. } => plain(StatusCode::NOT_IMPLEMENTED),
    }
}

/// An HTML file, labeled UTF-8 only when its bytes are (ADR 0022).
///
/// A file of at most [`MAX_INJECTION_INPUT_BYTES`] is read whole on a blocking thread, for
/// HEAD too so that HEAD and GET carry the same `Content-Type`, and GET sends those same bytes.
/// A larger file is not read ahead: its encoding is unknown, so it is labeled plain
/// `text/html` and streamed. A file that ends before its open-time length answers 500, since
/// no byte has been sent yet. Pinned by `should_omit_utf8_charset_when_html_is_latin1`,
/// `should_label_utf8_charset_when_html_is_utf8`,
/// `should_label_same_content_type_when_head_of_html` and
/// `should_omit_utf8_charset_when_html_exceeds_eight_mebibytes` in `tests/files.rs`.
async fn html_response(file: Box<dyn ContentFile>, method: &Method) -> Response {
    let len = file.len();
    if len > MAX_INJECTION_INPUT_BYTES {
        return file_response(file, method, html_content_type(false));
    }
    let read = tokio::task::spawn_blocking(move || read_exactly(file, len)).await;
    let Ok(Ok(bytes)) = read else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let content_type = html_content_type(std::str::from_utf8(&bytes).is_ok());
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    with_headers(body, content_type, len)
}

/// Exactly `len` bytes of `file`: never more if it grew, `UnexpectedEof` if it shrank.
fn read_exactly(file: Box<dyn ContentFile>, len: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or_default());
    file.take(len).read_to_end(&mut bytes)?;
    if (bytes.len() as u64) < len {
        return Err(ErrorKind::UnexpectedEof.into());
    }
    Ok(bytes)
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

/// A body of exactly `len` bytes read from `file` on a blocking thread.
///
/// `len` is the size at open time and is promised to the client as `Content-Length`, so the
/// reader never sends more than `len` bytes even if the file grew, and if the file shrank
/// (end of file before `len`) the stream fails with `UnexpectedEof`. That aborts the
/// connection instead of letting the client take a truncated body for a complete one. A
/// client that disconnects drops the receiver, the next send fails and the reader stops.
/// Pinned by `should_abort_body_when_file_shorter_than_length` and
/// `should_not_exceed_length_when_file_grew` in `tests/files.rs`.
fn stream_body(mut file: Box<dyn ContentFile>, len: u64) -> Body {
    if len == 0 {
        return Body::empty();
    }
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(QUEUE);
    // Detached on purpose: the task ends when the file is read out or the receiver is gone.
    drop(tokio::task::spawn_blocking(move || {
        let mut remaining = len;
        let mut buf = vec![0_u8; CHUNK];
        while remaining > 0 {
            let want = usize::try_from(remaining).map_or(CHUNK, |r| r.min(CHUNK));
            match file.read(&mut buf[..want]) {
                Ok(0) => {
                    let _ = tx.blocking_send(Err(ErrorKind::UnexpectedEof.into()));
                    return;
                }
                Ok(n) => {
                    remaining -= n as u64;
                    if tx
                        .blocking_send(Ok(Bytes::copy_from_slice(&buf[..n])))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
            }
        }
    }));
    Body::from_stream(ReceiverStream::new(rx))
}
