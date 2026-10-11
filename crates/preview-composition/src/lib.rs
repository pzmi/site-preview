//! The only production wiring root: constructs adapters and runs the server.
//!
//! [`run`] takes a [`ServeConfig`], opens the site root at the bootstrap boundary (the one
//! place in composition with ambient filesystem authority), builds the Tokio runtime, binds
//! the listener, wires the HTTP router to the root capability and the reload state, reports
//! readiness through a [`StartupReporter`] and serves until Ctrl-C (ADR 0028). The CLI only
//! parses, renders and maps a [`StartupError`] to an exit code.

mod bootstrap;
mod config;
mod server;

pub use bootstrap::RootRefusal;
pub use config::{BindRefusal, BindScope, ServeConfig};
pub use preview_domain::{DebounceWindow, DebounceWindowError};
pub use server::{Ready, StartupError, StartupReporter, run};
