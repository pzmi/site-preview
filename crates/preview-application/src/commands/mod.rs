//! Commands: use cases that change application state.
//!
//! A command returns its outcome together with the [`DomainEvent`](crate::events::DomainEvent)s
//! it produced, as a value; there is no command or event bus (foundation.md section 2.4), so
//! the caller decides where each event goes. Every command has a test entry point
//! (foundation.md section 5.2).
