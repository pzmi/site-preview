//! Capability-scoped filesystem access and the file watcher behind the application ports.
//!
//! [`open_site_root`] turns the requested root into a [`SiteRoot`] once at start-up;
//! [`RootDir`] is the root capability (ADR 0004): every served path is opened beneath it one
//! component at a time, never following a symbolic link. `RootDir` implements the application's
//! `SiteContent` port, resolving a directory to its own `index.html`. [`NotifyChangeSource`]
//! implements its `ChangeSource` port, watching the eligible directories [`eligible_dirs`] finds,
//! one non-recursive watch each.

mod capability;
mod root;
mod site_content;
mod traversal;
mod watcher;

#[cfg(test)]
#[path = "watcher/os_watcher_tests.rs"]
mod os_watcher;

pub use capability::{OpenError, Opened, OpenedDir, OpenedFile, RootDir};
pub use root::{RootError, SiteRoot, open_site_root};
pub use traversal::{MAX_DEPTH, Traversal, TraversalError, TraversalFault, eligible_dirs};
pub use watcher::{
    DEFAULT_CAPACITY, NotifyChangeSource, ReconcileReport, RegistrationError, RuntimeError,
    StartReport, WatchFault,
};
