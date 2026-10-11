//! The root capability: every served path is opened one component at a time beneath an open
//! directory descriptor of the site root (plan.md section 5.3, ADR 0004).
//!
//! [`RootDir::open_root`] is the only function here that takes a [`Path`]: it uses ambient
//! authority once, at start-up, to open the root the user selected. After that, a
//! [`SitePath`] is resolved with one `openat` per [`Component`], each relative to the
//! descriptor the previous step opened, and each with `O_NOFOLLOW`:
//!
//! - an intermediate component is opened with
//!   `O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`, so a symbolic link, a
//!   regular file or anything else that is not a real directory stops the walk. When that
//!   open fails with `ENOTDIR` (which either system may answer for a link as well as for a
//!   file), an `fstatat(AT_SYMLINK_NOFOLLOW)` of the same name classifies the refusal only: a
//!   link is `Forbidden(Symlink)`, anything else that is not a directory is `NotFound`, as a
//!   path through a file names nothing. It never opens anything and its answer never lets a
//!   walk continue, so a swap between the two calls changes only which refusal is reported;
//! - the final component is first looked at with `fstatat(AT_SYMLINK_NOFOLLOW)`, so a device,
//!   FIFO or socket is refused before `open` could run a driver's open routine; then it is
//!   opened with `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK | O_NOCTTY`, and `fstat` on
//!   the descriptor that was opened classifies it as a regular file, a directory or other.
//!   The `fstat` stays the authoritative check: an object swapped in after the pre-filter is
//!   still refused, the pre-filter only spares the common case the open.
//!
//! Nothing is checked by path and then trusted: the descriptor that was classified is the one
//! handed out, so a link swapped in after the open cannot redirect it. A component name is
//! passed to `openat` exactly as [`Component::as_str`] gives it, never decoded or joined
//! again; the [`Component`] invariant (no `/`, no `.` or `..`, no control characters) is what
//! makes it a single entry name.
//!
//! `O_NONBLOCK` is the one flag beyond ADR 0004's list: opening a FIFO for reading without it
//! blocks until a writer appears, which would hang the request before `fstat` could refuse
//! it. It is cleared again on a regular file. `O_NOCTTY` keeps a terminal device from becoming
//! the process's controlling terminal before it is refused.
//!
//! Limits, by design: this is HTTP-serving confinement, not a sandbox against other processes
//! of the same user. A hard link or a mount point inside the root is a real entry and is
//! served. A directory that may be searched but not read is refused, because each directory
//! is opened for reading.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use preview_application::{ContentRefusal, ForbiddenReason};
use preview_domain::{Component, SitePath};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;

/// Why [`RootDir::open`] or [`OpenedDir::open_child`] refused a path.
///
/// The same category-only refusal as the `SiteContent` port, so the adapter adds no second
/// error vocabulary and never carries a path or OS error text.
pub type OpenError = ContentRefusal;

/// Flags for every intermediate component: a directory, never followed, never inherited.
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

/// Flags for the final component, whose type `fstat` decides after the open.
const FINAL_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOCTTY);

/// An open descriptor of the site root: the capability every served path is resolved under.
///
/// It is built only by [`RootDir::open_root`]. It keeps naming the directory it opened even if
/// that directory is later renamed or replaced by a link.
#[derive(Debug)]
pub struct RootDir {
    fd: OwnedFd,
}

impl RootDir {
    /// Opens the selected site root: the single use of ambient filesystem authority.
    ///
    /// `canonical` is the root as resolved once at start-up; it may itself have been given
    /// through a symbolic link, so this one open follows links. Nothing below it does.
    ///
    /// # Errors
    ///
    /// The operating system's error when `canonical` cannot be opened as a directory.
    pub fn open_root(canonical: &Path) -> io::Result<Self> {
        let flags = OFlags::RDONLY
            .union(OFlags::DIRECTORY)
            .union(OFlags::CLOEXEC)
            .union(OFlags::NONBLOCK);
        let fd = retry_on_interrupt(|| rustix::fs::open(canonical, flags, Mode::empty()))?;
        Ok(Self { fd })
    }

    /// The root descriptor, for checks made relative to the root inside this crate (the
    /// reserved-name listing of `root.rs`). Never exported: outside the crate, the root is
    /// reachable only through [`RootDir::open`].
    pub(crate) fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Opens `path` beneath the root, one component at a time, never following a link.
    ///
    /// The root path opens a new descriptor of the root directory itself.
    ///
    /// # Errors
    ///
    /// - [`ContentRefusal::NotFound`] when a component does not exist, its name is longer than
    ///   the system allows, or an intermediate component is a file or other non-directory that
    ///   is not a link;
    /// - [`ForbiddenReason::Symlink`] when a component is a symbolic link (even one pointing
    ///   inside the root);
    /// - [`ForbiddenReason::NotRegular`] when the final object is neither a regular file nor a
    ///   directory;
    /// - [`ForbiddenReason::PermissionDenied`] when the system denies access;
    /// - [`ContentRefusal::Io`] for anything else.
    ///
    /// Pinned by the `should_refuse_*` and `should_open_*` tests of `tests/capability.rs`.
    pub fn open(&self, path: &SitePath) -> Result<Opened, OpenError> {
        let Some((last, parents)) = path.components().split_last() else {
            return self.reopen_root();
        };
        // Each step's descriptor is dropped (closed) when the next replaces it, or when an
        // error returns early: no path through this loop leaks one.
        let mut held: Option<OwnedFd> = None;
        for name in parents {
            let parent = held.as_ref().map_or(self.fd.as_fd(), AsFd::as_fd);
            let next = open_name(parent, name, DIR_FLAGS)
                .map_err(|errno| intermediate_refusal(parent, name, errno))?;
            held = Some(next);
        }
        let parent = held.as_ref().map_or(self.fd.as_fd(), AsFd::as_fd);
        open_final(parent, last)
    }

    /// A fresh descriptor of the root, with its own directory offset (a duplicate would share
    /// one, so two traversals of the root could disturb each other). `.` is a constant here,
    /// never request text.
    fn reopen_root(&self) -> Result<Opened, OpenError> {
        let fd = retry_on_interrupt(|| rustix::fs::openat(&self.fd, ".", DIR_FLAGS, Mode::empty()))
            .map_err(refusal)?;
        Ok(Opened::Directory(OpenedDir { fd }))
    }
}

/// What [`RootDir::open`] found: the descriptor that was opened and classified.
#[derive(Debug)]
pub enum Opened {
    /// A regular file.
    File(OpenedFile),
    /// A directory.
    Directory(OpenedDir),
}

/// An open regular file and its size when it was opened.
#[derive(Debug)]
pub struct OpenedFile {
    fd: OwnedFd,
    len: u64,
}

impl OpenedFile {
    /// The file's size in bytes, from `fstat` on the opened descriptor.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the file was empty when it was opened.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The opened descriptor, ready to read from its start.
    #[must_use]
    pub fn into_fd(self) -> OwnedFd {
        self.fd
    }
}

/// An open directory beneath the root, from which a child can be opened by name.
#[derive(Debug)]
pub struct OpenedDir {
    fd: OwnedFd,
}

impl OpenedDir {
    /// Opens the entry `name` of this directory with the final-component rules of
    /// [`RootDir::open`]: never following a link, then classified by `fstat`. This is how a
    /// directory's `index.html` is opened without reopening anything by path.
    ///
    /// # Errors
    ///
    /// As [`RootDir::open`] for its final component. Pinned by
    /// `should_refuse_when_child_of_opened_directory_is_symlink`.
    pub fn open_child(&self, name: &Component) -> Result<Opened, OpenError> {
        open_final(self.fd.as_fd(), name)
    }

    /// The opened directory descriptor.
    #[must_use]
    pub fn into_fd(self) -> OwnedFd {
        self.fd
    }
}

/// Opens the final component and classifies the descriptor it got.
///
/// The `fstatat` first is a pre-filter, not the check: it keeps a device, FIFO or socket from
/// being opened at all (a device's open routine runs before any `fstat` could refuse it), but
/// the object can change between the two calls, so the `fstat` of the opened descriptor below
/// still decides. Pinned by `should_not_open_fifo_when_final_component_is_fifo`.
fn open_final(parent: BorrowedFd<'_>, name: &Component) -> Result<Opened, OpenError> {
    let seen =
        retry_on_interrupt(|| rustix::fs::statat(parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW))
            .map_err(refusal)?;
    match FileType::from_raw_mode(seen.st_mode) {
        FileType::RegularFile | FileType::Directory => {}
        FileType::Symlink => return Err(ContentRefusal::Forbidden(ForbiddenReason::Symlink)),
        _ => return Err(ContentRefusal::Forbidden(ForbiddenReason::NotRegular)),
    }
    let fd = open_name(parent, name, FINAL_FLAGS).map_err(|errno| {
        if refuses_non_regular(errno) {
            ContentRefusal::Forbidden(ForbiddenReason::NotRegular)
        } else {
            refusal(errno)
        }
    })?;
    let stat = rustix::fs::fstat(&fd).map_err(refusal)?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => {
            let len = u64::try_from(stat.st_size)
                .map_err(|_| ContentRefusal::Io(io::ErrorKind::InvalidData))?;
            // Reads of a regular file ignore O_NONBLOCK on the P0 systems; clearing it keeps a
            // network or FUSE filesystem from ever answering a read with EAGAIN.
            rustix::fs::fcntl_setfl(&fd, OFlags::empty()).map_err(refusal)?;
            Ok(Opened::File(OpenedFile { fd, len }))
        }
        FileType::Directory => Ok(Opened::Directory(OpenedDir { fd })),
        _ => Err(ContentRefusal::Forbidden(ForbiddenReason::NotRegular)),
    }
}

/// One `openat` of a single validated name relative to `parent`.
fn open_name(
    parent: BorrowedFd<'_>,
    name: &Component,
    flags: OFlags,
) -> rustix::io::Result<OwnedFd> {
    retry_on_interrupt(|| rustix::fs::openat(parent, name.as_str(), flags, Mode::empty()))
}

/// Retries a call the kernel interrupted with a signal (`EINTR`), which a slow filesystem
/// may return from `open` itself.
fn retry_on_interrupt<T>(mut call: impl FnMut() -> rustix::io::Result<T>) -> rustix::io::Result<T> {
    loop {
        match call() {
            Err(Errno::INTR) => {}
            other => return other,
        }
    }
}

/// Errors of the final open that mean the object exists but is not a file or directory:
/// Linux answers an open of a socket, or of a device without a driver, with `ENXIO` or
/// `ENODEV`; macOS answers a socket with `EOPNOTSUPP`. A regular file or directory never
/// produces these from a read-only open.
fn refuses_non_regular(errno: Errno) -> bool {
    matches!(errno, Errno::NXIO | Errno::NODEV | Errno::OPNOTSUPP)
}

/// The refusal for an intermediate component whose directory open failed.
///
/// With `O_DIRECTORY | O_NOFOLLOW`, Linux and macOS may answer a link as well as a regular
/// file with `ENOTDIR`. Only on that error is the name looked at again, without following
/// it, to tell the two apart: a link is refused as one, while a file or other non-directory is
/// `NotFound`, so `/index.html/x` does not reveal that `index.html` exists. A directory found
/// there now (swapped in after the open), or a failed look, stays `Symlink`, the refusal the
/// open itself would have given. The look opens nothing and never resumes the walk. Pinned
/// by `should_return_not_found_when_middle_component_is_regular_file` and
/// `should_refuse_when_middle_component_is_symlink`.
fn intermediate_refusal(parent: BorrowedFd<'_>, name: &Component, errno: Errno) -> ContentRefusal {
    if errno != Errno::NOTDIR {
        return refusal(errno);
    }
    let looked =
        retry_on_interrupt(|| rustix::fs::statat(parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW));
    match looked.map(|stat| FileType::from_raw_mode(stat.st_mode)) {
        Ok(FileType::Symlink | FileType::Directory) | Err(_) => {
            ContentRefusal::Forbidden(ForbiddenReason::Symlink)
        }
        Ok(_) => ContentRefusal::NotFound,
    }
}

/// The port's category for an errno.
///
/// `ELOOP` is what Linux and macOS return for `O_NOFOLLOW` on a link. `ENOTDIR` reaches here
/// only from an open that [`intermediate_refusal`] did not classify; it means "not a real
/// directory to walk through" and is refused like a link. A name longer than `NAME_MAX`
/// (`ENAMETOOLONG`) names nothing that can exist, so it is `NotFound` like any missing name,
/// not a server failure. (FreeBSD's `EMLINK` for a link is outside the P0 targets.)
fn refusal(errno: Errno) -> ContentRefusal {
    match errno {
        Errno::NOENT | Errno::NAMETOOLONG => ContentRefusal::NotFound,
        Errno::LOOP | Errno::NOTDIR => ContentRefusal::Forbidden(ForbiddenReason::Symlink),
        Errno::ACCESS | Errno::PERM => ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied),
        other => ContentRefusal::Io(io::Error::from(other).kind()),
    }
}
