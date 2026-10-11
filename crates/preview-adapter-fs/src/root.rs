//! Opening the site root at start-up (plan.md sections 3, 5.3 and 6, ADR 0004).
//!
//! [`open_site_root`] is the one place a user-supplied path becomes a [`RootDir`]. It resolves
//! the requested path once (the root itself may be given through a symbolic link), opens the
//! resolved directory, and refuses a root that already holds an entry the eligibility rule
//! reserves (`__site_preview` in any letter case, the namespace of the internal endpoints), so
//! that user content is never silently shadowed. The check lists the root's entries and asks
//! `preview_domain::eligibility::component` about each one, the same rule serving applies to
//! every request (ADR 0012): an exact-name lookup would miss `__Site_Preview` on a
//! case-sensitive filesystem, which serving then refuses as reserved.
//! Everything below the root is then resolved by the capability alone.

use core::fmt;
use std::io;
use std::path::{Path, PathBuf};

use preview_domain::Component;
use preview_domain::eligibility::{self, ComponentKind, DenyReason, Eligibility, Level};
use rustix::fs::{Dir, FileType};

use crate::capability::RootDir;

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
    /// The root already contains an entry named `__site_preview` in any letter case (of any
    /// type, a symbolic link included), which would collide with the internal endpoints.
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
                "the site root contains `__site_preview` (in some letter case), a name reserved for site-preview's internal endpoints; rename or remove it",
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
    if holds_reserved_entry(&dir)? {
        return Err(RootError::ReservedPathConflict);
    }
    Ok(SiteRoot {
        dir,
        display_path: resolved,
    })
}

/// Whether any entry of the root is one the eligibility rule reserves at the root.
///
/// The entries are listed through a fresh descriptor of the root (`Dir::read_from` opens `.`
/// beneath it), so the root capability's own directory offset is never moved. A name that is
/// not UTF-8, or not a valid [`Component`], cannot be requested and cannot be reserved, so it
/// is skipped. The verdict is the domain's, so this check and serving never disagree about
/// which names collide (pinned by
/// `should_fail_with_reserved_conflict_when_name_differs_only_in_case`).
fn holds_reserved_entry(dir: &RootDir) -> Result<bool, RootError> {
    let entries = Dir::read_from(dir.as_fd()).map_err(errno_error)?;
    for entry in entries {
        let entry = entry.map_err(errno_error)?;
        let Ok(text) = entry.file_name().to_str() else {
            continue;
        };
        let Ok(name) = Component::new(text) else {
            continue;
        };
        let kind = if entry.file_type() == FileType::Directory {
            ComponentKind::Dir
        } else {
            ComponentKind::File
        };
        if is_reserved(&name, kind) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_reserved(name: &Component, kind: ComponentKind) -> bool {
    eligibility::component(name, kind, Level::Root) == Eligibility::Denied(DenyReason::Reserved)
}

fn errno_error(errno: rustix::io::Errno) -> RootError {
    from_kind(io::Error::from(errno).kind())
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
    use super::*;

    #[test]
    fn should_treat_name_as_reserved_when_eligibility_reserves_it_for_either_kind() {
        // given
        let names = ["__site_preview", "__SITE_PREVIEW", "__s\u{0131}te_preview"]
            .map(|text| Component::new(text).expect("valid name"));
        let ordinary = Component::new("site_preview").expect("valid name");

        // when
        let verdicts = names.map(|name| {
            [ComponentKind::Dir, ComponentKind::File].map(|kind| is_reserved(&name, kind))
        });
        let ordinary_verdict = is_reserved(&ordinary, ComponentKind::Dir);

        // then
        assert_eq!(verdicts, [[true; 2]; 3]);
        assert!(!ordinary_verdict);
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
