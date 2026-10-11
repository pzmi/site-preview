//! Capability-scoped filesystem access and the file watcher behind the application ports.
//!
//! [`open_site_root`] turns the requested root into a [`SiteRoot`] once at start-up;
//! [`RootDir`] is the root capability (ADR 0004): every served path is opened beneath it one
//! component at a time, never following a symbolic link. `RootDir` implements the application's
//! `SiteContent` port, resolving a directory to its own `index.html`.

mod capability;
mod root;
mod site_content;

pub use capability::{OpenError, Opened, OpenedDir, OpenedFile, RootDir};
pub use root::{RootError, SiteRoot, open_site_root};
