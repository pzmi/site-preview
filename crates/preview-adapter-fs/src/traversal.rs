//! The set of directories the watcher registers (plan.md section 8.1 steps 1 and 2, ADR 0004,
//! ADR 0012).
//!
//! [`eligible_dirs`] walks the site from the root capability and never from a path: each
//! directory is opened with one `openat` per component (`O_DIRECTORY | O_NOFOLLOW`) and listed
//! through its own descriptor. The rules that keep it inside what serving would serve:
//!
//! - **One predicate.** Every directory entry is judged by
//!   [`preview_domain::eligibility::component`], the function serving uses, and judged
//!   *before* it is opened, so an ineligible directory (`.git`, `node_modules`, `vendor`, the
//!   reserved root name, a name HFS+ would fold onto one of those) is never descended into and
//!   none of its descendants is seen. Entries directly in the root get `Level::Root`.
//! - **No links.** Only an entry the system reports as a real directory is a candidate. A
//!   symbolic link (to a directory or anywhere else) is skipped, and the descent opens with
//!   `O_NOFOLLOW`, so a link swapped in after the listing is refused as well.
//! - **Names.** A name that is not UTF-8 or not a valid [`Component`] cannot be requested, so
//!   it is skipped, as the root check does.
//! - **Bounded.** Nesting beyond [`MAX_DEPTH`] components is not descended into and is
//!   reported, which also ends a walk through a bind-mount cycle that links cannot cause.
//! - **Partial results.** A directory that cannot be opened or listed is reported in
//!   [`Traversal::errors`] and the walk goes on. Such a directory is not in the set: it cannot
//!   be watched. Only a start directory that cannot be opened fails the call.
//!
//! The walk is iterative with a work list of relative paths. No descriptor is held across
//! directories (a wide tree cannot exhaust them), and each directory is reopened from the
//! root capability, never from an absolute path.

use core::fmt;
use std::collections::BTreeSet;
use std::io;

use preview_application::{ContentRefusal, ForbiddenReason};
use preview_domain::eligibility::{self, ComponentKind, Eligibility, Level};
use preview_domain::{Component, RelPath};
use rustix::fs::{AtFlags, Dir, FileType};

use crate::capability::{OpenedDir, RootDir};

/// The deepest directory the walk enters, counted in components below the root. Far beyond any
/// site, and well inside `PATH_MAX`; it bounds the work done on a cyclic mount layout.
pub const MAX_DEPTH: usize = 128;

/// What [`eligible_dirs`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Traversal {
    /// The eligible, readable directories, the start directory included.
    pub dirs: BTreeSet<RelPath>,
    /// Directories the walk could not enter or list; the rest of the tree is unaffected.
    pub errors: Vec<TraversalError>,
}

/// Why a directory could not be walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraversalFault {
    /// The system denied opening or listing the directory.
    PermissionDenied,
    /// The directory lies deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Any other operating-system failure, by kind.
    Io(io::ErrorKind),
}

/// A directory the walk could not enter, named relative to the root; never an absolute path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TraversalError {
    /// The directory, relative to the site root (empty for the root itself).
    pub dir: RelPath,
    /// Why.
    pub fault: TraversalFault,
}

impl fmt::Display for TraversalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self
            .dir
            .components()
            .iter()
            .map(Component::as_str)
            .collect();
        let dir = if names.is_empty() {
            ".".to_owned()
        } else {
            names.join("/")
        };
        match self.fault {
            TraversalFault::PermissionDenied => {
                write!(f, "cannot read directory `{dir}`: permission denied")
            }
            TraversalFault::TooDeep => {
                write!(
                    f,
                    "directory `{dir}` is nested more than {MAX_DEPTH} levels deep"
                )
            }
            TraversalFault::Io(kind) => write!(f, "cannot read directory `{dir}`: {kind}"),
        }
    }
}

impl core::error::Error for TraversalError {}

/// Lists the eligible directories at and below `start`.
///
/// An ineligible `start` (any of its components denied) yields an empty set. Results are
/// ordered, a directory immediately before its descendants.
///
/// # Errors
///
/// A [`TraversalError`] when `start` itself cannot be opened (missing, not a directory, a
/// link, unreadable). Failures below it are in [`Traversal::errors`] instead. Pinned by the
/// tests of `tests/traversal.rs`.
pub fn eligible_dirs(root: &RootDir, start: &RelPath) -> Result<Traversal, TraversalError> {
    let mut found = Traversal::default();
    if eligibility::components(start.components(), ComponentKind::Dir) != Eligibility::Eligible {
        return Ok(found);
    }
    let first = open_dir(root, start).map_err(|fault| TraversalError {
        dir: start.clone(),
        fault,
    })?;
    found.dirs.insert(start.clone());
    let mut pending = Vec::new();
    list_children(&first, start, &mut pending, &mut found.errors);
    drop(first);
    while let Some(dir) = pending.pop() {
        if dir.components().len() > MAX_DEPTH {
            found.errors.push(TraversalError {
                dir,
                fault: TraversalFault::TooDeep,
            });
            continue;
        }
        match open_dir(root, &dir) {
            Ok(opened) => {
                list_children(&opened, &dir, &mut pending, &mut found.errors);
                found.dirs.insert(dir);
            }
            // Removed or replaced since the listing: not a directory to watch any more.
            Err(TraversalFault::Io(io::ErrorKind::NotFound)) => {}
            Err(fault) => found.errors.push(TraversalError { dir, fault }),
        }
    }
    Ok(found)
}

/// Opens `dir` beneath the root. A link or file found where the listing saw a directory is
/// reported as `NotFound`: it is gone as a directory, which is not an error of the tree.
fn open_dir(root: &RootDir, dir: &RelPath) -> Result<OpenedDir, TraversalFault> {
    root.open_dir(dir).map_err(|refusal| match refusal {
        ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied) => {
            TraversalFault::PermissionDenied
        }
        ContentRefusal::NotFound | ContentRefusal::Forbidden(_) => {
            TraversalFault::Io(io::ErrorKind::NotFound)
        }
        ContentRefusal::Io(kind) => TraversalFault::Io(kind),
    })
}

/// Pushes the eligible real subdirectories of `opened` (which is `dir`) onto `pending`.
fn list_children(
    opened: &OpenedDir,
    dir: &RelPath,
    pending: &mut Vec<RelPath>,
    errors: &mut Vec<TraversalError>,
) {
    let level = if dir.is_root() {
        Level::Root
    } else {
        Level::Nested
    };
    let fail = |errors: &mut Vec<TraversalError>, errno: rustix::io::Errno| {
        let fault = if matches!(errno, rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) {
            TraversalFault::PermissionDenied
        } else {
            TraversalFault::Io(io::Error::from(errno).kind())
        };
        errors.push(TraversalError {
            dir: dir.clone(),
            fault,
        });
    };
    // `read_from` lists through a fresh descriptor of this directory, not a path.
    let entries = match Dir::read_from(opened.as_fd()) {
        Ok(entries) => entries,
        Err(errno) => return fail(errors, errno),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(errno) => return fail(errors, errno),
        };
        let Ok(text) = entry.file_name().to_str() else {
            continue;
        };
        let Ok(name) = Component::new(text) else {
            continue;
        };
        let file_type = match entry.file_type() {
            FileType::Unknown => {
                match rustix::fs::statat(
                    opened.as_fd(),
                    entry.file_name(),
                    AtFlags::SYMLINK_NOFOLLOW,
                ) {
                    Ok(stat) => FileType::from_raw_mode(stat.st_mode),
                    Err(_) => continue,
                }
            }
            known => known,
        };
        if file_type != FileType::Directory {
            continue;
        }
        if eligibility::component(&name, ComponentKind::Dir, level) == Eligibility::Eligible {
            pending.push(dir.join(name));
        }
    }
}
