//! The bootstrap boundary: the only file in composition that names the filesystem adapter,
//! and so the only place a user-supplied path becomes a capability (ADR 0004, ADR 0035).
//! Everything after it holds the root as the application's `SiteContent` port.

use core::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use preview_adapter_fs::{RootError, open_site_root};
use preview_application::SiteContent;

/// The opened root, with the adapter type erased.
pub(crate) struct OpenedRoot {
    /// Every served file is opened beneath this capability.
    pub content: Arc<dyn SiteContent>,
    /// The resolved root, for the start-up report only.
    pub display_path: PathBuf,
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
    Ok(OpenedRoot {
        content: Arc::new(root.dir),
        display_path: root.display_path,
    })
}
