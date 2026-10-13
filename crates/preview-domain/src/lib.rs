//! Pure policies, transitions and invariants of the preview server; performs no I/O.

mod change;
mod change_set;
mod debounce;
mod events;
mod lifecycle;
mod rel_path;
mod site_state;
mod time;
mod watch_set;
mod watcher_health;

pub use change::{ChangeEvent, ChangeKind, Classification, IgnoreReason, Relevance, classify};
pub use change_set::{AssetKind, ChangeSet, ChangedPath, FullReason, MAX_CHANGES};
pub use debounce::{Deadline, Debouncer, Poll, Settled};
pub use events::{DegradeReason, DomainEvent};
pub use lifecycle::{
    InFlight, Lifecycle, LifecycleEffect, LifecycleInput, LifecycleRefusal, LifecycleState,
    StartPhase, Transition,
};
pub use site_state::{InstanceId, InstanceIdError, Revision, RevisionExhausted, SiteState};

pub mod request_policy;

pub use request_policy::{
    Authority, AuthorityHost, BoundAddr, HostDecision, HostPolicy, HostReject, OriginDecision,
    OriginPolicy, OriginReject,
};
pub mod eligibility;
pub mod injection;
pub mod path;
pub mod warning_code;

pub use injection::{
    FetchDest, InjectionDecision, InjectionInput, InjectionWarning, MAX_INJECTION_INPUT_BYTES,
    Media, RequestMethod, SkipReason,
};
pub use path::{Component, PathRefusal, SitePath, decode_request_path};
pub use rel_path::RelPath;
pub use time::{DebounceWindow, DebounceWindowError, Tick};
pub use watch_set::{WatchDiff, WatchSet};
pub use watcher_health::{HealthTransition, WatcherHealth, WatcherHealthRefusal, WatcherInput};
