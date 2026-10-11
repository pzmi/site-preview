//! Route table and layers.

use axum::Router;
use axum::http::StatusCode;
use axum::middleware::from_fn;
use axum::response::Response;
use axum::routing::any;

use crate::errors::{harden, method_gate, plain};
use crate::files::serve;
use crate::state::AppState;

/// Builds the router for one server instance.
///
/// - Layers, outermost first: [`harden`] (no-store, nosniff), then the method gate (405 for
///   anything but GET and HEAD, `Allow: GET, HEAD`).
/// - `/__site_preview`, `/__site_preview/` and everything beneath it are matched before the
///   file fallback and answered by internal handlers, never from disk. Until the later tasks
///   add the client script and the event stream, every internal path is a 404.
/// - Everything else goes to the file fallback.
///
/// Percent-encoded spellings of the reserved name are not matched by the routes; the
/// application's eligibility check refuses them with 403 before any open (ADR 0012).
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/__site_preview", any(internal_not_found))
        .route("/__site_preview/", any(internal_not_found))
        .route("/__site_preview/{*rest}", any(internal_not_found))
        .fallback(serve)
        .layer(from_fn(method_gate))
        .layer(from_fn(harden))
        .with_state(state)
}

async fn internal_not_found() -> Response {
    plain(StatusCode::NOT_FOUND)
}
