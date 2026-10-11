//! Capability-scoped filesystem access and the file watcher behind the application ports.
//!
//! [`RootDir`] is the root capability (ADR 0004): every served path is opened beneath it one
//! component at a time, never following a symbolic link.

mod capability;

pub use capability::{OpenError, Opened, OpenedDir, OpenedFile, RootDir};
