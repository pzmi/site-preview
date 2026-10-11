//! Capability-scoped filesystem access and the file watcher behind the application ports.
//!
//! [`open_site_root`] turns the requested root into a [`SiteRoot`] once at start-up;
//! [`RootDir`] is the root capability (ADR 0004): every served path is opened beneath it one
//! component at a time, never following a symbolic link.

mod capability;
mod root;

pub use capability::{OpenError, Opened, OpenedDir, OpenedFile, RootDir};
pub use root::{RootError, SiteRoot, open_site_root};
