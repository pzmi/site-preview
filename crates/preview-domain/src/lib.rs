//! Pure policies, transitions and invariants of the preview server; performs no I/O.

mod events;
mod site_state;

pub use events::{DegradeReason, DomainEvent};
pub use site_state::{InstanceId, InstanceIdError, Revision, RevisionExhausted, SiteState};
