//! HTTP handlers, HTML injection and live-reload transport behind the application ports.
//!
//! This crate translates between HTTP and the application's queries: it parses nothing the
//! domain owns and decides no path policy. [`build_router`] is the single entry point; the
//! composition root supplies an [`AppState`] and serves the returned router.

mod client;
mod directory;
mod errors;
mod files;
mod host;
pub mod inject;
mod mime;
mod router;
mod shutdown;
mod sse;
mod sse_guard;
mod state;
mod stream;

pub use router::build_router;
pub use shutdown::{Shutdown, ShutdownSignal};
pub use sse::DEFAULT_KEEP_ALIVE;
pub use sse_guard::{DEFAULT_MAX_STREAMS, StreamLimit};
pub use state::AppState;
