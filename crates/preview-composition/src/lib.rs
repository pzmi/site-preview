//! The only production wiring root: constructs adapters and runs the server.
//!
//! [`Server`] drives the domain lifecycle machine through start-up and shutdown: it opens the
//! site root at the bootstrap boundary (the one place in composition with ambient filesystem
//! authority), builds the Tokio runtime, binds the listener, wires the HTTP router to the root
//! capability and the reload state, starts the file watcher and its debounce pipeline task,
//! reports readiness through a [`StartupReporter`], reports warnings through a [`WarningSink`]
//! and serves within [`ServerLimits`]; [`run`] does so until SIGINT or SIGTERM (ADR 0028,
//! ADR 0045, ADR 0049, ADR 0051). The CLI only parses, renders and maps a [`StartupError`] to
//! an exit code.

mod bootstrap;
mod config;
mod connections;
mod lifecycle;
mod server;
mod watch_pipeline;

pub use bootstrap::RootRefusal;
pub use config::{BindRefusal, BindScope, ServeConfig, ServerLimits};
pub use connections::{StopSignal, Stopped};
pub use lifecycle::Server;
pub use preview_adapter_http::WarningSink;
pub use preview_domain::{DebounceWindow, DebounceWindowError};
pub use server::{Ready, StartupError, StartupReporter, WatchStatus, run};
