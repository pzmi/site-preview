//! The bootstrap boundary: the only file in composition that names the filesystem adapter,
//! and so the only place a user-supplied path becomes a capability (ADR 0004, ADR 0035).
//! Everything after it holds the root as the application's `SiteContent` port, and the
//! watcher as a [`WatchControl`] (ADR 0049).

use core::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use preview_adapter_fs::{NotifyChangeSource, RootDir, RootError, open_site_root};
use preview_application::SiteContent;

use crate::watch_pipeline::{ReconcileScope, WatchControl};

/// How many directories that could not be watched are named one by one at start-up; the rest
/// are counted in one line, so a reached watch limit cannot flood stderr.
const NAMED_REGISTRATION_ERRORS: usize = 10;

/// The opened root, with the adapter type erased.
pub(crate) struct OpenedRoot {
    /// Every served file is opened beneath this capability.
    pub content: Arc<dyn SiteContent>,
    /// The resolved root, for the start-up report only.
    pub display_path: PathBuf,
    /// The same capability, for the watcher's walk and registration.
    dir: Arc<RootDir>,
}

/// What starting the watcher set up, in words composition can report without naming the
/// adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchStart {
    /// The number of directories watched.
    pub dirs: usize,
    /// One line per directory that could not be walked or watched (the first
    /// [`NAMED_REGISTRATION_ERRORS`]), then one line counting the rest. Relative paths only.
    pub problems: Vec<String>,
    /// Why nothing is watched: the native watcher could not be created, or not one
    /// directory could be registered.
    pub unavailable: Option<String>,
}

impl WatchStart {
    /// Whether every eligible directory is watched.
    pub(crate) fn is_complete(&self) -> bool {
        self.problems.is_empty() && self.unavailable.is_none()
    }
}

/// The requested root cannot be served: the path as the user gave it and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRefusal {
    requested: PathBuf,
    reason: RootError,
}

impl fmt::Display for RootRefusal {
    /// `cannot serve <path>: <reason>`; the adapter's reason carries no path, so the path
    /// appears once. Pinned by `should_exit_1_when_directory_does_not_exist`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot serve {}: {}",
            self.requested.display(),
            self.reason
        )
    }
}

impl core::error::Error for RootRefusal {}

/// Resolves and opens `requested` once (plan.md section 3: "Resolve the requested root once
/// at startup").
pub(crate) fn open_root(requested: &Path) -> Result<OpenedRoot, RootRefusal> {
    let root = open_site_root(requested).map_err(|reason| RootRefusal {
        requested: requested.to_path_buf(),
        reason,
    })?;
    let dir = Arc::new(root.dir);
    Ok(OpenedRoot {
        content: Arc::clone(&dir) as Arc<dyn SiteContent>,
        display_path: root.display_path,
        dir,
    })
}

/// Registers every eligible directory of `root` with the native watcher (plan.md section 8.1)
/// and returns the change source with the adapter type erased. Never fails: what could not be
/// watched is in the [`WatchStart`], and the source's first batch degrades the coordinator
/// (ADR 0047, ADR 0048).
pub(crate) fn start_watcher(root: &OpenedRoot) -> (impl WatchControl + use<>, WatchStart) {
    let (source, report) = NotifyChangeSource::start(&root.dir, &root.display_path);
    let mut problems: Vec<String> = report
        .errors
        .iter()
        .take(NAMED_REGISTRATION_ERRORS)
        .map(ToString::to_string)
        .collect();
    let unnamed = report
        .errors
        .len()
        .saturating_sub(NAMED_REGISTRATION_ERRORS);
    if unnamed > 0 {
        problems.push(format!(
            "{unnamed} more directories cannot be watched for the same kinds of reason"
        ));
    }
    let start = WatchStart {
        dirs: report.watching,
        problems,
        unavailable: match report.unavailable {
            Some(fault) => Some(fault.to_string()),
            None if report.watching == 0 => {
                Some("no eligible directory could be watched".to_owned())
            }
            None => None,
        },
    };
    (source, start)
}

impl WatchControl for NotifyChangeSource {
    /// The reconcile hook. The adapter cannot re-register yet, so the watch set and the
    /// health stay as they are; directory create and remove reconciliation (FS-09) fills it.
    fn reconcile(&mut self, _scope: ReconcileScope) -> Option<usize> {
        None
    }

    fn failure_detail(&self) -> Option<String> {
        self.last_failure().map(ToString::to_string)
    }

    fn close(&mut self) {
        Self::close(self);
    }
}
