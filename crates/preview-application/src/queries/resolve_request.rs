//! The [`ResolveRequest`] query: from a raw request path to the file to serve, with the site
//! state to embed in it (plan.md §5.1, §5.2, §7.3; ADR 0012, ADR 0039).

use core::fmt;
use std::io::ErrorKind;

use preview_domain::eligibility::{self, ComponentKind, DenyReason, Eligibility, Level};
use preview_domain::{Component, PathRefusal, SitePath, SiteState, decode_request_path};

use crate::ports::{ContentFile, ContentRefusal, ForbiddenReason, Opened, SiteContent};

const INDEX: &str = "index.html";

/// Resolve one request path against the site.
///
/// The order is fixed and is the security boundary of serving:
///
/// 1. decode the path once ([`decode_request_path`]);
/// 2. apply the eligibility predicate ([`eligibility::path`]); a refused path is never opened;
/// 3. take the [`SiteState`] snapshot;
/// 4. open through [`SiteContent`] and classify what was opened.
///
/// Taking the snapshot before the open means a save that lands during or after the open
/// advances the server past the embedded revision, so the page reloads once instead of
/// keeping stale content (plan.md §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolveRequest<'a> {
    /// The request path as it arrived: starting with `/`, without query or fragment.
    pub raw_path: &'a str,
}

/// What a [`ResolveRequest`] decided. No variant carries text taken from the raw request.
pub enum Resolved {
    /// Serve this file; `snapshot` is the state read just before it was opened.
    File {
        /// The opened file: the requested file, or a directory's `index.html`.
        file: Box<dyn ContentFile>,
        /// The site state to embed in an HTML response, read before the open.
        snapshot: SiteState,
        /// The served file's own name (`index.html` for a directory), for its media type.
        name: Component,
    },
    /// The path names an eligible directory but lacks its trailing slash. `path` is the
    /// decoded request path, so the `Location` can be encoded from validated components
    /// rather than echoed from the raw input.
    RedirectToSlash {
        /// The decoded path, without its trailing slash.
        path: SitePath,
    },
    /// Nothing may be served here: missing, a directory without an index, a file requested
    /// with a trailing slash, or a refusal that must not reveal what exists.
    NotFound,
    /// The path may not be served, for a reason that does not depend on what exists on disk
    /// beyond what any request could learn.
    Forbidden(ForbiddenCause),
    /// The path is not a well-formed request path.
    BadRequest,
    /// Opening failed with an unclassified I/O error; nothing was served.
    Unavailable(ErrorKind),
}

impl fmt::Debug for Resolved {
    /// Shows the variant and sizes; never file contents.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File {
                file,
                snapshot,
                name,
            } => f
                .debug_struct("File")
                .field("len", &file.len())
                .field("snapshot", snapshot)
                .field("name", name)
                .finish(),
            Self::RedirectToSlash { path } => f
                .debug_struct("RedirectToSlash")
                .field("path", path)
                .finish(),
            Self::NotFound => f.write_str("NotFound"),
            Self::Forbidden(cause) => f.debug_tuple("Forbidden").field(cause).finish(),
            Self::BadRequest => f.write_str("BadRequest"),
            Self::Unavailable(kind) => f.debug_tuple("Unavailable").field(kind).finish(),
        }
    }
}

/// Why a path is [`Resolved::Forbidden`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForbiddenCause {
    /// The eligibility predicate denied a component; decided before anything was opened.
    Ineligible(DenyReason),
    /// Confinement or the file type refused the open.
    Content(ForbiddenReason),
}

impl ResolveRequest<'_> {
    /// Runs the query against `content`, calling `snapshot` at most once, after the path is
    /// found eligible and immediately before the open.
    ///
    /// `snapshot` is the latest-state read; in production `|| reload.current()` over the
    /// shared [`ReloadState`](crate::ReloadState). It is not called for a malformed or
    /// ineligible path.
    ///
    /// Pinned by the component tests in `tests/resolve_request.rs`, among them
    /// `should_snapshot_state_before_open_when_resolving_file`,
    /// `should_not_open_when_path_is_ineligible` and
    /// `should_return_not_found_when_excluded_directory_requested_without_slash`.
    pub fn execute(
        &self,
        content: &dyn SiteContent,
        snapshot: impl FnOnce() -> SiteState,
    ) -> Resolved {
        let path = match decode_request_path(self.raw_path) {
            Ok(path) => path,
            Err(PathRefusal::Malformed) => return Resolved::BadRequest,
            // `.`, `..`, encoded separators and `a//b` name nothing that can exist.
            Err(PathRefusal::Traversal) => return Resolved::NotFound,
        };
        if let Eligibility::Denied(reason) = eligibility::path(&path) {
            return Resolved::Forbidden(ForbiddenCause::Ineligible(reason));
        }
        // A final name denied only as a directory (`node_modules`, `vendor`) was checked as a
        // file above. If it is a directory, every outcome but a plain file would tell the
        // client that the directory exists, so all of them become `NotFound`.
        let denied_as_dir = denied_as_directory(&path);
        let state = snapshot();
        let opened = match content.open(&path) {
            Ok(opened) => opened,
            Err(_) if denied_as_dir => return Resolved::NotFound,
            Err(ContentRefusal::NotFound) => return Resolved::NotFound,
            Err(ContentRefusal::Forbidden(reason)) => {
                return Resolved::Forbidden(ForbiddenCause::Content(reason));
            }
            Err(ContentRefusal::Io(kind)) => return Resolved::Unavailable(kind),
        };
        match opened {
            Opened::File(file) => match path.components().last() {
                Some(name) if !path.has_trailing_slash() => Resolved::File {
                    file,
                    snapshot: state,
                    name: name.clone(),
                },
                // `/page.html/`: a file asked for as a directory; its relative links would
                // resolve against the wrong base.
                _ => Resolved::NotFound,
            },
            Opened::Directory { .. } if denied_as_dir => Resolved::NotFound,
            Opened::Directory { .. } if !path.has_trailing_slash() => {
                Resolved::RedirectToSlash { path }
            }
            Opened::Directory { index: Some(file) } => Resolved::File {
                file,
                snapshot: state,
                name: index_name(),
            },
            Opened::Directory { index: None } => Resolved::NotFound,
        }
    }
}

/// Whether the last component, requested without a trailing slash, would be denied if it
/// turned out to be a directory.
fn denied_as_directory(path: &SitePath) -> bool {
    if path.has_trailing_slash() {
        return false;
    }
    let names = path.components();
    let Some(last) = names.last() else {
        return false;
    };
    let level = if names.len() == 1 {
        Level::Root
    } else {
        Level::Nested
    };
    eligibility::component(last, ComponentKind::Dir, level) != Eligibility::Eligible
}

fn index_name() -> Component {
    Component::new(INDEX).expect("`index.html` is a valid component")
}
