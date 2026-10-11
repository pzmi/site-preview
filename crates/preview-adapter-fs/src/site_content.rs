//! The [`SiteContent`] port over the root capability (plan.md section 5.1 rows 1 and 2,
//! section 5.3, ADR 0004, ADR 0038).
//!
//! [`RootDir::open`] does the confined walk; this module adds what the port promises on top:
//! a directory's `index.html` is opened relative to the directory descriptor that was just
//! opened (never reopened by path), and a regular file becomes a [`ContentFile`] over the
//! very descriptor that `fstat` classified. Every refusal is already the port's
//! category-only [`ContentRefusal`], so no path or OS text can reach the caller.

use std::fs::File;
use std::io::{self, Read};

use preview_application::{ContentFile, ContentRefusal, ForbiddenReason, Opened, SiteContent};
use preview_domain::{Component, SitePath};

use crate::capability::{self, OpenedDir, OpenedFile, RootDir};

/// The only name a directory is served by (plan.md section 5.1): no `index.htm`, no listing.
const INDEX: &str = "index.html";

impl SiteContent for RootDir {
    /// Opens `path` beneath the root with the port's contract; pinned by the tests of
    /// `tests/site_content.rs` and, from TEST-04, the shared contract harness.
    fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal> {
        match RootDir::open(self, path)? {
            capability::Opened::File(file) => Ok(Opened::File(Box::new(FsFile::from(file)))),
            capability::Opened::Directory(dir) => open_index(&dir),
        }
    }
}

/// Opens the directory's `index.html` through the directory's own descriptor.
///
/// A missing index is a directory without one; any other refusal of the index is the whole
/// call's refusal, and an `index.html` that is itself a directory is not a regular file.
fn open_index(dir: &OpenedDir) -> Result<Opened, ContentRefusal> {
    let name =
        Component::new(INDEX).map_err(|_| ContentRefusal::Io(io::ErrorKind::InvalidInput))?;
    let index = match dir.open_child(&name) {
        Ok(capability::Opened::File(file)) => Some(FsFile::from(file)),
        Ok(capability::Opened::Directory(_)) => {
            return Err(ContentRefusal::Forbidden(ForbiddenReason::NotRegular));
        }
        Err(ContentRefusal::NotFound) => None,
        Err(refusal) => return Err(refusal),
    };
    Ok(Opened::Directory {
        index: index.map(|file| Box::new(file) as Box<dyn ContentFile>),
    })
}

/// An opened regular file: the classified descriptor as a [`File`], and its size at open.
struct FsFile {
    file: File,
    len: u64,
}

impl From<OpenedFile> for FsFile {
    fn from(opened: OpenedFile) -> Self {
        let len = opened.len();
        Self {
            file: File::from(opened.into_fd()),
            len,
        }
    }
}

impl Read for FsFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl ContentFile for FsFile {
    fn len(&self) -> u64 {
        self.len
    }
}
