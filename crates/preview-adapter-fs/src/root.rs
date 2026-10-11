//! Opening the site root at start-up (plan.md sections 3, 5.3 and 6, ADR 0004).
//!
//! [`open_site_root`] is the one place a user-supplied path becomes a [`RootDir`]. It resolves
//! the requested path once (the root itself may be given through a symbolic link), opens the
//! resolved directory, and refuses a root that already holds an entry named `__site_preview`,
//! the namespace of the internal endpoints, so that user content is never silently shadowed.
//! Everything below the root is then resolved by the capability alone.

use core::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs::AtFlags;
use rustix::io::Errno;

use crate::capability::RootDir;

/// The name reserved for the internal endpoints directly in the site root. It matches the
/// domain's eligibility rule, which denies this name at the root (pinned by
/// `should_be_denied_by_eligibility_when_name_is_reserved`).
const RESERVED_NAME: &str = "__site_preview";

/// The opened site root.
#[derive(Debug)]
pub struct SiteRoot {
    /// The root capability every served path is resolved under.
    pub dir: RootDir,
    /// The resolved root, for the start-up message only: never logged per request and never
    /// used to open anything again.
    pub display_path: PathBuf,
}

/// Why the requested root cannot be served. Each message says what to do; none carries the
/// path, which the caller already holds and may print beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootError {
    /// The path, or a component of it, does not exist.
    NotFound,
    /// The path exists but is not a directory.
    NotDirectory,
    /// The directory exists but this user may not list or search it.
    NotReadable,
    /// The root already contains an entry named `__site_preview` (of any type, a symbolic link
    /// included), which would collide with the internal endpoints.
    ReservedPathConflict,
    /// Any other operating-system failure, by kind (a symbolic-link loop, a name too long).
    Io(io::ErrorKind),
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str(
                "the site root does not exist; pass the path of an existing directory",
            ),
            Self::NotDirectory => f.write_str(
                "the site root is not a directory; pass the directory that contains index.html",
            ),
            Self::NotReadable => f.write_str(
                "the site root cannot be read; grant read and search permission on it, or pick another directory",
            ),
            Self::ReservedPathConflict => f.write_str(
                "the site root contains `__site_preview`, a name reserved for site-preview's internal endpoints; rename or remove it",
            ),
            Self::Io(kind) => write!(f, "the site root cannot be opened: {kind}"),
        }
    }
}

impl core::error::Error for RootError {}

/// Resolves `requested` once, opens it as the root capability and checks the reserved name.
///
/// A symbolic link to a directory is accepted as the root and resolved here; nothing below
/// the root is ever followed (ADR 0004).
///
/// # Errors
///
/// [`RootError`] as documented on its variants. Pinned by the tests of `tests/root.rs`.
pub fn open_site_root(requested: &Path) -> Result<SiteRoot, RootError> {
    let resolved = std::fs::canonicalize(requested).map_err(|e| from_kind(e.kind()))?;
    let dir = RootDir::open_root(&resolved).map_err(|e| from_kind(e.kind()))?;
    match rustix::fs::statat(dir.as_fd(), RESERVED_NAME, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => return Err(RootError::ReservedPathConflict),
        Err(Errno::NOENT) => {}
        Err(errno) => return Err(from_kind(io::Error::from(errno).kind())),
    }
    Ok(SiteRoot {
        dir,
        display_path: resolved,
    })
}

fn from_kind(kind: io::ErrorKind) -> RootError {
    match kind {
        io::ErrorKind::NotFound => RootError::NotFound,
        io::ErrorKind::NotADirectory => RootError::NotDirectory,
        io::ErrorKind::PermissionDenied => RootError::NotReadable,
        other => RootError::Io(other),
    }
}

#[cfg(test)]
mod tests {
    use preview_domain::Component;
    use preview_domain::eligibility::{self, ComponentKind, DenyReason, Eligibility, Level};

    use super::*;

    #[test]
    fn should_be_denied_by_eligibility_when_name_is_reserved() {
        // given
        let name = Component::new(RESERVED_NAME).expect("valid name");

        // when
        let verdict = eligibility::component(&name, ComponentKind::Dir, Level::Root);

        // then
        assert_eq!(verdict, Eligibility::Denied(DenyReason::Reserved));
    }

    #[test]
    fn should_give_actionable_message_without_path_when_root_error_is_displayed() {
        // given
        let errors = [
            RootError::NotFound,
            RootError::NotDirectory,
            RootError::NotReadable,
            RootError::ReservedPathConflict,
            RootError::Io(io::ErrorKind::InvalidInput),
        ];

        // when
        let texts = errors.map(|e| e.to_string());

        // then
        for text in texts {
            assert!(!text.contains('/'), "a root error rendered a path: {text}");
            assert!(
                text.starts_with("the site root"),
                "unexpected message: {text}"
            );
        }
    }
}
