//! The only production wiring root: constructs adapters and runs the server.
//!
//! [`run`] takes a [`ServeConfig`], opens the site root at the bootstrap boundary (the one
//! place in composition with ambient filesystem authority), builds the Tokio runtime, binds
//! the listener, wires the HTTP router to the root capability and the reload state, reports
//! readiness through a [`StartupReporter`], reports warnings through a [`WarningSink`] and
//! serves within [`ServerLimits`] until SIGINT or SIGTERM (ADR 0028, ADR 0045). The CLI only
//! parses, renders and maps a [`StartupError`] to an exit code.

mod bootstrap;
mod config;
mod connections;
mod server;

pub use bootstrap::RootRefusal;
pub use config::{BindRefusal, BindScope, ServeConfig, ServerLimits};
pub use connections::{StopSignal, Stopped};
pub use preview_adapter_http::WarningSink;
pub use preview_domain::{DebounceWindow, DebounceWindowError};
pub use server::{Ready, StartupError, StartupReporter, run};
