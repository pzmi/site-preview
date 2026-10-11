//! Pure policies, transitions and invariants of the preview server; performs no I/O.

mod debounce;
mod events;
mod site_state;
mod time;

pub use debounce::{Deadline, Debouncer, Poll, Settled};
pub use events::{DegradeReason, DomainEvent};
pub use site_state::{InstanceId, InstanceIdError, Revision, RevisionExhausted, SiteState};

pub mod request_policy;

pub use request_policy::{
    Authority, AuthorityHost, BoundAddr, HostDecision, HostPolicy, HostReject, OriginDecision,
    OriginPolicy, OriginReject,
};
pub mod eligibility;
pub mod path;

pub use path::{Component, PathRefusal, SitePath, decode_request_path};
pub use time::{DebounceWindow, DebounceWindowError, Tick};
