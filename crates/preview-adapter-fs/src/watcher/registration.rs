//! Registering the eligible directories with the native watcher (plan.md section 8.1 steps 3
//! to 6, section 8.4).
//!
//! - **One watch per eligible directory.** [`NotifyWatcher::start`] walks the site with
//!   [`eligible_dirs`] and calls `watch(dir, NonRecursive)` for each directory it returns, the
//!   root included, parents before children. A directory the walk prunes is never named to the
//!   operating system.
//! - **Absolute paths only here.** The path handed to `notify` is `display_root` joined with
//!   the directory's validated components; it is built in [`NotifyWatcher::absolute`] and goes
//!   nowhere else. Errors name the relative directory.
//! - **Partial failure.** A directory that cannot be walked or watched becomes a
//!   [`RegistrationError`] and the rest is still registered. Once the system's watch limit is
//!   reached no further watch is attempted (each would fail the same way); the remaining
//!   directories are reported with [`WatchFault::WatchLimitReached`].
//! - **No link registration.** `notify` registers by path and follows a symbolic link, so after
//!   each watch the directory is reopened through the root capability (`O_NOFOLLOW` per
//!   component). If it is no longer a real directory there (a link swapped in after the walk),
//!   the watch is removed and the directory reported. A swap in the instant between the watch
//!   and that check, then back, is not detected; ADR 0047 records the residual window.
//! - **Record.** Successful registrations are kept in a [`WatchSet`], the answer to "is this
//!   directory watched" for reconciliation.

use core::fmt;
use std::io;
use std::path::{Path, PathBuf};

use notify::{Config, EventHandler, RecommendedWatcher, RecursiveMode, Watcher};
use preview_application::{ContentRefusal, ForbiddenReason};
use preview_domain::{RelPath, WatchSet};

use super::display_rel;
use crate::capability::RootDir;
use crate::traversal::{Traversal, TraversalError, TraversalFault, eligible_dirs};

/// Why a directory is not watched. A category only: the OS error text and the absolute path
/// never leave the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchFault {
    /// The system denied listing or watching the directory.
    PermissionDenied,
    /// The directory vanished, or is no longer a real directory (a link or file now stands
    /// at its path).
    NotFound,
    /// The system's limit on watches is reached (Linux `fs.inotify.max_user_watches`).
    WatchLimitReached,
    /// The directory lies deeper than [`crate::MAX_DEPTH`].
    TooDeep,
    /// Any other operating-system failure, by kind.
    Io(io::ErrorKind),
    /// The native watcher failed without an operating-system error kind.
    Unavailable,
}

impl fmt::Display for WatchFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::NotFound => f.write_str("not found or no longer a directory"),
            Self::WatchLimitReached => f.write_str(
                "the system's file watch limit is reached (on Linux raise fs.inotify.max_user_watches)",
            ),
            Self::TooDeep => write!(f, "nested more than {} levels deep", crate::MAX_DEPTH),
            Self::Io(kind) => write!(f, "{kind}"),
            Self::Unavailable => f.write_str("the native file watcher is unavailable"),
        }
    }
}

impl From<TraversalFault> for WatchFault {
    fn from(fault: TraversalFault) -> Self {
        match fault {
            TraversalFault::PermissionDenied => Self::PermissionDenied,
            TraversalFault::TooDeep => Self::TooDeep,
            TraversalFault::Io(io::ErrorKind::NotFound) => Self::NotFound,
            TraversalFault::Io(kind) => Self::Io(kind),
        }
    }
}

impl From<&notify::Error> for WatchFault {
    fn from(error: &notify::Error) -> Self {
        match &error.kind {
            notify::ErrorKind::Io(e) => match e.kind() {
                io::ErrorKind::PermissionDenied => Self::PermissionDenied,
                io::ErrorKind::NotFound => Self::NotFound,
                kind => Self::Io(kind),
            },
            notify::ErrorKind::PathNotFound | notify::ErrorKind::WatchNotFound => Self::NotFound,
            notify::ErrorKind::MaxFilesWatch => Self::WatchLimitReached,
            notify::ErrorKind::Generic(_) | notify::ErrorKind::InvalidConfig(_) => {
                Self::Unavailable
            }
        }
    }
}

/// A directory that is not watched, and why (plan.md section 8.1 step 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegistrationError {
    /// The directory, relative to the site root (empty for the root itself).
    pub dir: RelPath,
    /// Why.
    pub fault: WatchFault,
}

impl fmt::Display for RegistrationError {
    /// `cannot watch directory `<relative dir>`: <reason>`; pinned by
    /// `should_report_directory_and_reason_when_registration_fails`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot watch directory `{}`: {}",
            display_rel(&self.dir),
            self.fault
        )
    }
}

impl core::error::Error for RegistrationError {}

impl From<TraversalError> for RegistrationError {
    fn from(error: TraversalError) -> Self {
        Self {
            dir: error.dir,
            fault: error.fault.into(),
        }
    }
}

/// The native watcher and the directories registered with it. Dropping it removes every watch.
pub struct NotifyWatcher {
    inner: RecommendedWatcher,
    display_root: PathBuf,
    watched: WatchSet,
}

impl fmt::Debug for NotifyWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotifyWatcher")
            .field("watched", &self.watched.len())
            .finish_non_exhaustive()
    }
}

impl NotifyWatcher {
    /// Creates the native watcher, delivering its notifications to `handler`, and registers
    /// every eligible directory of the site.
    ///
    /// `display_root` is the resolved root the capability `root` was opened from
    /// ([`crate::SiteRoot::display_path`]). The returned errors list each directory that could
    /// not be walked or watched; the others are watched regardless.
    ///
    /// # Errors
    ///
    /// The [`WatchFault`] of the native watcher itself failing to start (on Linux, the
    /// `inotify` instance limit); nothing is watched then.
    pub fn start<H: EventHandler>(
        root: &RootDir,
        display_root: &Path,
        handler: H,
    ) -> Result<(Self, Vec<RegistrationError>), WatchFault> {
        let mut watcher = Self::new(display_root, handler)?;
        let errors = match eligible_dirs(root, &RelPath::root()) {
            Ok(traversal) => watcher.register(root, traversal),
            Err(error) => vec![error.into()],
        };
        Ok((watcher, errors))
    }

    /// The native watcher with nothing registered.
    ///
    /// On macOS `notify` 8.2 panics registering a path that is not UTF-8, so such a root is
    /// refused as [`WatchFault::Unavailable`] up front (ADR 0047).
    pub(crate) fn new<H: EventHandler>(
        display_root: &Path,
        handler: H,
    ) -> Result<Self, WatchFault> {
        if cfg!(target_os = "macos") && display_root.to_str().is_none() {
            return Err(WatchFault::Unavailable);
        }
        let inner = RecommendedWatcher::new(handler, Config::default())
            .map_err(|error| WatchFault::from(&error))?;
        Ok(Self {
            inner,
            display_root: display_root.to_path_buf(),
            watched: WatchSet::new(),
        })
    }

    /// Registers the directories of `traversal` not yet watched and reports its walk errors
    /// beside the registration failures, in that order.
    pub(crate) fn register(
        &mut self,
        root: &RootDir,
        traversal: Traversal,
    ) -> Vec<RegistrationError> {
        let mut errors: Vec<RegistrationError> = traversal
            .errors
            .into_iter()
            .map(RegistrationError::from)
            .collect();
        let mut added = Vec::new();
        let mut limit_reached = false;
        for dir in traversal.dirs {
            if self.watched.contains(&dir) {
                continue;
            }
            if limit_reached {
                errors.push(RegistrationError {
                    dir,
                    fault: WatchFault::WatchLimitReached,
                });
                continue;
            }
            match self.watch_one(root, &dir) {
                Ok(()) => added.push(dir),
                Err(fault) => {
                    limit_reached = fault == WatchFault::WatchLimitReached;
                    errors.push(RegistrationError { dir, fault });
                }
            }
        }
        let diff = preview_domain::WatchDiff {
            add: added.into_iter().collect(),
            remove: Default::default(),
        };
        self.watched.apply(&diff);
        errors
    }

    /// Watches one directory, then confirms through the capability that a real directory
    /// still stands at its path; otherwise the watch is withdrawn.
    fn watch_one(&mut self, root: &RootDir, dir: &RelPath) -> Result<(), WatchFault> {
        let path = self.absolute(dir);
        self.inner
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|error| WatchFault::from(&error))?;
        let Err(refusal) = root.open_dir(dir) else {
            return Ok(());
        };
        // The watch may already be gone with its inode; nothing else to undo.
        let _ = self.inner.unwatch(&path);
        Err(match refusal {
            ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied) => {
                WatchFault::PermissionDenied
            }
            ContentRefusal::Io(kind) => WatchFault::Io(kind),
            ContentRefusal::NotFound | ContentRefusal::Forbidden(_) => WatchFault::NotFound,
        })
    }

    /// The absolute path of `dir`: the only place one is built.
    fn absolute(&self, dir: &RelPath) -> PathBuf {
        let mut path = self.display_root.clone();
        path.extend(dir.components().iter().map(|name| name.as_str()));
        path
    }

    /// The directories currently watched, root-relative.
    #[must_use]
    pub fn watched(&self) -> &WatchSet {
        &self.watched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use preview_domain::Component;

    fn rel(text: &str) -> RelPath {
        RelPath::new(
            text.split('/')
                .filter(|part| !part.is_empty())
                .map(|part| Component::new(part).expect("valid component"))
                .collect(),
        )
    }

    #[test]
    fn should_name_relative_directory_and_reason_when_error_is_displayed() {
        // given
        let errors = [
            RegistrationError {
                dir: rel("docs/guide"),
                fault: WatchFault::PermissionDenied,
            },
            RegistrationError {
                dir: RelPath::root(),
                fault: WatchFault::NotFound,
            },
        ];

        // when
        let texts = errors.map(|e| e.to_string());

        // then
        assert_eq!(
            texts,
            [
                "cannot watch directory `docs/guide`: permission denied",
                "cannot watch directory `.`: not found or no longer a directory",
            ]
        );
    }

    #[test]
    fn should_map_notify_errors_to_categories_when_converted() {
        // given
        let errors = [
            notify::Error::new(notify::ErrorKind::MaxFilesWatch),
            notify::Error::path_not_found(),
            notify::Error::io(io::Error::from(io::ErrorKind::PermissionDenied)),
            notify::Error::io(io::Error::from(io::ErrorKind::NotFound)),
            notify::Error::io(io::Error::from(io::ErrorKind::OutOfMemory)),
            notify::Error::generic("text that must not leak"),
        ];

        // when
        let faults = errors.each_ref().map(WatchFault::from);

        // then
        assert_eq!(
            faults,
            [
                WatchFault::WatchLimitReached,
                WatchFault::NotFound,
                WatchFault::PermissionDenied,
                WatchFault::NotFound,
                WatchFault::Io(io::ErrorKind::OutOfMemory),
                WatchFault::Unavailable,
            ]
        );
    }

    #[test]
    fn should_map_traversal_faults_when_converted() {
        // given
        let faults = [
            TraversalFault::PermissionDenied,
            TraversalFault::TooDeep,
            TraversalFault::Io(io::ErrorKind::NotFound),
            TraversalFault::Io(io::ErrorKind::Interrupted),
        ];

        // when
        let mapped = faults.map(WatchFault::from);

        // then
        assert_eq!(
            mapped,
            [
                WatchFault::PermissionDenied,
                WatchFault::TooDeep,
                WatchFault::NotFound,
                WatchFault::Io(io::ErrorKind::Interrupted),
            ]
        );
    }
}
