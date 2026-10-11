//! The file fallback: resolve a request path through the application and map the outcome.

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::Response;
use preview_application::{ResolveRequest, Resolved};

use crate::errors::plain;
use crate::state::AppState;

/// Resolves `uri.path()` (never the query) on a blocking thread and maps the outcome.
///
/// Streaming (HTTP-02) and the directory redirect (HTTP-04) are not here yet: a resolved file
/// or redirect answers a fixed 501 and the opened handle is dropped unread.
pub(crate) async fn serve(State(state): State<AppState>, uri: Uri) -> Response {
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
        Resolved::BadRequest => plain(StatusCode::BAD_REQUEST),
        Resolved::NotFound => plain(StatusCode::NOT_FOUND),
        Resolved::Forbidden(_) => plain(StatusCode::FORBIDDEN),
        Resolved::Unavailable(_) => plain(StatusCode::INTERNAL_SERVER_ERROR),
        Resolved::File { .. } | Resolved::RedirectToSlash { .. } => {
            plain(StatusCode::NOT_IMPLEMENTED)
        }
    }
}
