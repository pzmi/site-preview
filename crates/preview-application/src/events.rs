//! The events commands report.
//!
//! Three kinds of event exist in the system and are kept apart (foundation.md section 2.4):
//!
//! - **Change events** come *in* from the watcher through
//!   [`ChangeSource`](crate::ports::ChangeSource); they say what the operating system saw.
//! - **Domain events**, re-exported here, are what the application decided: a revision was
//!   published, the watcher degraded or recovered. Commands return them as values.
//! - **Notifications** go *out* to connected pages; they are the HTTP adapter's
//!   representation of a published state and never appear in this crate.
//!
//! The domain events carry fixed categories only (no path, no error text), so a log line or a
//! response built from one cannot leak a filesystem location.

pub use preview_domain::{DegradeReason, DomainEvent};
