//! A response body read from a blocking file handle, one chunk per poll.

use std::future::Future;
use std::io::{self, ErrorKind, Read};
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use preview_application::ContentFile;
use tokio::task::JoinHandle;
use tokio_stream::Stream;

/// Bytes read per blocking call. Small enough to bound memory per connection, large enough
/// to keep the per-chunk overhead negligible.
const CHUNK: usize = 64 * 1024;

/// A body of exactly `len` bytes read from `file`.
///
/// Each chunk is one short `spawn_blocking` read, started only when the connection polls the
/// body for more, so a blocking thread is busy for one read at a time and never waits on a
/// client: a slow or stalled reader holds one buffered chunk and no thread, and cannot starve
/// the blocking pool that request resolution also uses. This holds for every body built here:
/// every non-HTML file and HTML over 8 MiB. HTML of at most 8 MiB is instead held whole in
/// memory so it can be checked and rewritten, and is bounded by the HTML budget (ADR 0046).
/// No task outlives the body: dropping it (a client gone, or the server shutting down) stops
/// the reading, and a read already in flight finishes on its own and drops the handle
/// (plan.md §9). Pinned by
/// `should_complete_other_request_when_client_stalls_body` and
/// `should_stop_reading_when_body_dropped` in `tests/stream.rs`.
///
/// `len` is the size at open time and is promised to the client as `Content-Length`, so no
/// more than `len` bytes are sent even if the file grew, and if it shrank (end of file before
/// `len`) the stream fails with `UnexpectedEof`. That aborts the connection instead of letting
/// the client take a truncated body for a complete one. Pinned by
/// `should_abort_body_when_file_shorter_than_length` and
/// `should_not_exceed_length_when_file_grew` in `tests/files.rs`.
pub(crate) fn stream_body(file: Box<dyn ContentFile>, len: u64) -> Body {
    if len == 0 {
        return Body::empty();
    }
    Body::from_stream(FileStream {
        state: State::Idle(file),
        remaining: len,
    })
}

type ReadResult = (Box<dyn ContentFile>, io::Result<Vec<u8>>);

enum State {
    /// Waiting to be polled; nothing is being read.
    Idle(Box<dyn ContentFile>),
    /// One chunk is being read on a blocking thread.
    Reading(JoinHandle<ReadResult>),
    /// Finished, failed or exhausted; the handle is dropped.
    Done,
}

struct FileStream {
    state: State,
    remaining: u64,
}

impl Stream for FileStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        loop {
            match std::mem::replace(&mut this.state, State::Done) {
                State::Done => return Poll::Ready(None),
                State::Idle(_) if this.remaining == 0 => return Poll::Ready(None),
                State::Idle(file) => {
                    let want = usize::try_from(this.remaining).map_or(CHUNK, |r| r.min(CHUNK));
                    this.state =
                        State::Reading(tokio::task::spawn_blocking(move || read_chunk(file, want)));
                }
                State::Reading(mut reading) => {
                    let Poll::Ready(joined) = Pin::new(&mut reading).poll(cx) else {
                        this.state = State::Reading(reading);
                        return Poll::Pending;
                    };
                    let chunk = match joined {
                        Ok((_, Ok(chunk))) if chunk.is_empty() => {
                            Err(ErrorKind::UnexpectedEof.into())
                        }
                        Ok((file, Ok(chunk))) => {
                            this.remaining -= chunk.len() as u64;
                            this.state = State::Idle(file);
                            Ok(Bytes::from(chunk))
                        }
                        Ok((_, Err(e))) => Err(e),
                        Err(join) => Err(io::Error::other(join)),
                    };
                    return Poll::Ready(Some(chunk));
                }
            }
        }
    }
}

/// Up to `want` bytes; empty only at end of file.
fn read_chunk(mut file: Box<dyn ContentFile>, want: usize) -> ReadResult {
    let mut buf = vec![0_u8; want];
    loop {
        match file.read(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return (file, Ok(buf));
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return (file, Err(e)),
        }
    }
}
