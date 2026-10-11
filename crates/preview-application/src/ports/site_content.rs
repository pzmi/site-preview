//! The [`SiteContent`] port: open a validated site path, confined beneath the site root.

use core::fmt;
use std::io::{ErrorKind, Read};

use preview_domain::SitePath;

/// Read access to the site's files, confined beneath the root the store was built for.
///
/// # Contract
///
/// Every implementation, real or fake, honours all of the following; the `SiteContent`
/// contract harness checks them against each one.
///
/// - **Confinement.** `open` resolves `path` one component at a time beneath the root and
///   never follows a symbolic link at or below the root, not even one that points back
///   inside it (plan.md section 5.3, ADR 0004). A link at any component is
///   [`ForbiddenReason::Symlink`].
/// - **Open, then classify.** The handle returned is the object that was opened; nothing is
///   checked by path and reopened afterwards. A regular file is [`Opened::File`]; a
///   directory is [`Opened::Directory`]; anything else (a FIFO, a socket, a device) is
///   [`ForbiddenReason::NotRegular`].
/// - **Index resolution.** For a directory, `open` also opens `index.html` relative to that
///   same directory, with the same no-follow rule. A missing `index.html` gives
///   `Directory { index: None }`. An `index.html` that exists but is refused (a link, not a
///   regular file, unreadable) makes the whole call return that refusal: a directory whose
///   index cannot be served is never presented as one without an index. Only `index.html`
///   counts; there is no `index.htm` and never a listing.
/// - **No eligibility.** `open` does not apply the eligibility predicate (hidden names,
///   `node_modules`, editor files). The caller checks
///   `preview_domain::eligibility::path` before calling, so serving and watching share one
///   predicate (ADR 0012). An ineligible but confined path opens like any other.
/// - **Trailing slash ignored.** `a/b` and `a/b/` open the same object; deciding whether a
///   directory request needs a redirect is the caller's job. The root path opens the root
///   directory.
/// - **Refusals carry no location.** A [`ContentRefusal`] holds a category only, never a
///   path, a file name or OS error text, so it may be logged or turned into a response as is.
/// - **Independent calls.** `open` keeps no state between calls, may be called from many
///   threads at once (`Send + Sync`), and reflects the filesystem at the moment of the call.
///   The handles it returns own their resources and outlive the borrow of the store.
/// - **Blocking.** `open` and reads from a [`ContentFile`] may block on disk I/O; an
///   asynchronous caller runs them on a blocking-capable thread.
///
/// The trait is synchronous so that it is object safe: queries take `&dyn SiteContent`.
pub trait SiteContent: Send + Sync {
    /// Opens `path` beneath the root.
    ///
    /// # Errors
    ///
    /// [`ContentRefusal::NotFound`] when a component does not exist,
    /// [`ContentRefusal::Forbidden`] when confinement or the file type refuses it, and
    /// [`ContentRefusal::Io`] for any other failure; see the trait's contract.
    fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal>;
}

/// An opened regular file, read from its start.
///
/// The handle is the opened object itself, so a rename or a link swapped in after `open`
/// cannot redirect the bytes it yields.
pub trait ContentFile: Read + Send {
    /// The file's size in bytes when it was opened.
    ///
    /// A writer may change the file afterwards, so a reader that promises a length to its
    /// client (a `Content-Length`) must stop at this many bytes and handle a short read.
    fn len(&self) -> u64;

    /// Whether [`len`](Self::len) is zero.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What a successful [`SiteContent::open`] found.
pub enum Opened {
    /// A regular file.
    File(Box<dyn ContentFile>),
    /// A directory, with its `index.html` when it has one.
    Directory {
        /// The directory's `index.html`, already opened; `None` when there is none.
        index: Option<Box<dyn ContentFile>>,
    },
}

impl fmt::Debug for Opened {
    /// Shows the shape and sizes only; a handle has nothing else worth printing.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(file) => f.debug_tuple("File").field(&file.len()).finish(),
            Self::Directory { index } => f
                .debug_struct("Directory")
                .field("index", &index.as_ref().map(|file| file.len()))
                .finish(),
        }
    }
}

/// Why confinement or the file type refused a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForbiddenReason {
    /// A component is a symbolic link, or an intermediate component is not a directory: a
    /// no-follow directory open cannot tell the two apart, and both are refused.
    Symlink,
    /// The final object is neither a regular file nor a directory.
    NotRegular,
    /// The operating system denied access to a component.
    PermissionDenied,
}

/// Why [`SiteContent::open`] did not open a path. A category only: never a path, a file name
/// or OS error text (pinned by `should_render_category_only_when_refusal_is_displayed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentRefusal {
    /// A component does not exist.
    NotFound,
    /// The path exists but may not be served.
    Forbidden(ForbiddenReason),
    /// Any other I/O failure, by kind. A missing entry, a refused link or non-directory
    /// component and a denied access have their own variants and never arrive here as
    /// `NotFound`, `NotADirectory` or `PermissionDenied`.
    Io(ErrorKind),
}

impl fmt::Display for ContentRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Forbidden(ForbiddenReason::Symlink) => {
                f.write_str("forbidden: symbolic link or non-directory component")
            }
            Self::Forbidden(ForbiddenReason::NotRegular) => {
                f.write_str("forbidden: not a regular file or directory")
            }
            Self::Forbidden(ForbiddenReason::PermissionDenied) => {
                f.write_str("forbidden: permission denied")
            }
            Self::Io(kind) => write!(f, "io error: {kind}"),
        }
    }
}

impl core::error::Error for ContentRefusal {}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use preview_domain::decode_request_path;

    use super::*;

    struct Bytes(Cursor<Vec<u8>>);

    impl Read for Bytes {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl ContentFile for Bytes {
        fn len(&self) -> u64 {
            self.0.get_ref().len() as u64
        }
    }

    /// A one-file store: `/` is a directory whose index is `body`; anything else is missing.
    struct OneIndex(&'static [u8]);

    impl SiteContent for OneIndex {
        fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal> {
            if !path.is_root() {
                return Err(ContentRefusal::NotFound);
            }
            let index: Box<dyn ContentFile> = Box::new(Bytes(Cursor::new(self.0.to_vec())));
            Ok(Opened::Directory { index: Some(index) })
        }
    }

    fn assert_send<T: Send + 'static>(_: &T) {}

    #[test]
    fn should_open_through_trait_object_when_store_is_dyn() {
        // given
        let store: Box<dyn SiteContent> = Box::new(OneIndex(b"<h1>hi</h1>"));
        let root = decode_request_path("/").expect("root decodes");
        // when
        let opened = store.open(&root).expect("root opens");
        drop(store);
        // then
        assert_send(&opened);
        let Opened::Directory {
            index: Some(mut file),
        } = opened
        else {
            panic!("expected a directory with an index, got {opened:?}");
        };
        let mut body = Vec::new();
        file.read_to_end(&mut body).expect("in-memory read");
        assert_eq!(body, b"<h1>hi</h1>");
        assert_eq!(file.len(), 11);
        assert!(!file.is_empty());
    }

    #[test]
    fn should_render_category_only_when_refusal_is_displayed() {
        // given
        let refusals = [
            ContentRefusal::NotFound,
            ContentRefusal::Forbidden(ForbiddenReason::Symlink),
            ContentRefusal::Forbidden(ForbiddenReason::NotRegular),
            ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied),
            ContentRefusal::Io(ErrorKind::InvalidData),
        ];
        // when
        let texts = refusals.map(|r| format!("{r} {r:?}"));
        // then
        for text in texts {
            assert!(!text.contains('/'), "a refusal rendered a path: {text}");
        }
    }

    #[test]
    fn should_show_sizes_only_when_opened_is_debug_printed() {
        // given
        let file: Box<dyn ContentFile> = Box::new(Bytes(Cursor::new(vec![0; 3])));
        let opened = [Opened::File(file), Opened::Directory { index: None }];
        // when
        let texts = opened.map(|o| format!("{o:?}"));
        // then
        assert_eq!(texts, ["File(3)", "Directory { index: None }"]);
    }
}
