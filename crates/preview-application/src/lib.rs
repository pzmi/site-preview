//! Commands, queries, use cases and port traits that orchestrate the domain.
//!
//! The crate keeps four concepts apart (foundation.md section 2.4):
//!
//! - [`ports`]: the traits the application needs from the outside world, implemented by the
//!   adapters and by the testkit fakes. A trait exists only at a nondeterministic or
//!   replaceable boundary (foundation.md section 2.3): file content, change notifications and
//!   time.
//! - [`commands`]: use cases that change state and return outcomes, events or refusals.
//! - [`queries`]: use cases that observe state without changing a domain decision.
//! - [`events`]: the domain events the commands report, distinct from the watcher's change
//!   events and from the HTTP notifications sent to pages.
//!
//! The application performs no I/O itself and names no adapter technology: no HTTP, watcher
//! or file-descriptor type appears in a signature here (ADR 0038). Security policies stay pure
//! functions in `preview-domain` (ADR 0012); the application orders them and the ports.

pub mod commands;
pub mod events;
pub mod ports;
pub mod queries;
pub mod reload_state;

pub use ports::{
    ChangeBatch, ChangeSource, Clock, ContentFile, ContentRefusal, ForbiddenReason, Opened,
    SiteContent,
};
pub use reload_state::{Published, ReloadClosed, ReloadState, StateSubscription};
