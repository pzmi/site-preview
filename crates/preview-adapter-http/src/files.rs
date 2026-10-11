//! The file fallback: resolve a request path through the application and map the outcome.

use std::io::{self, ErrorKind, Read};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use preview_application::{ContentFile, ResolveRequest, Resolved};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::errors::plain;
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
/// (plan.md §5.1). The media type is a placeholder until HTTP-03.
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
        Resolved::File { file, .. } => file_response(file, &method),
        Resolved::BadRequest => plain(StatusCode::BAD_REQUEST),
        Resolved::NotFound => plain(StatusCode::NOT_FOUND),
        Resolved::Forbidden(_) => plain(StatusCode::FORBIDDEN),
        Resolved::Unavailable(_) => plain(StatusCode::INTERNAL_SERVER_ERROR),
        Resolved::RedirectToSlash { .. } => plain(StatusCode::NOT_IMPLEMENTED),
    }
}

fn file_response(file: Box<dyn ContentFile>, method: &Method) -> Response {
    let len = file.len();
    let body = if *method == Method::HEAD {
        // The handle is dropped unread: HEAD does no file I/O beyond the open.
        Body::empty()
    } else {
        stream_body(file, len)
    };
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
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
