//! HTTP handlers, HTML injection and live-reload transport behind the application ports.
//!
//! This crate translates between HTTP and the application's queries: it parses nothing the
//! domain owns and decides no path policy. [`build_router`] is the single entry point; the
//! composition root supplies an [`AppState`] and serves the returned router.

mod directory;
mod errors;
mod files;
mod mime;
mod router;
mod state;

pub use router::build_router;
pub use state::AppState;
