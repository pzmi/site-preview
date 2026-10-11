//! An in-memory [`SiteContent`] with the real adapter's refusal semantics.

use std::collections::BTreeMap;
use std::io::{Cursor, ErrorKind, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use preview_application::{ContentFile, ContentRefusal, ForbiddenReason, Opened, SiteContent};
use preview_domain::SitePath;

const INDEX: &str = "index.html";

#[derive(Clone)]
enum Node {
    Dir(BTreeMap<String, Node>),
    File(Arc<[u8]>),
    /// A symbolic link; where it points is irrelevant, it is never followed.
    Symlink,
    /// A FIFO, socket or device.
    Special,
    /// Access denied by the operating system.
    Denied,
    /// An entry whose open fails with this kind.
    Broken(ErrorKind),
}

type Hook = Box<dyn Fn() + Send + Sync>;

struct Inner {
    root: Mutex<BTreeMap<String, Node>>,
    opens: AtomicUsize,
    on_open: Mutex<Option<Hook>>,
}

/// A site tree held in memory: directories, files, and markers for symbolic links, special
/// files, denied entries and failing entries.
///
/// It refuses exactly as the real store does (the `SiteContent` contract): a link at any
/// component is `Forbidden(Symlink)` and is never followed, an intermediate file is
/// `Forbidden(Symlink)` too, a directory opens its own `index.html` and returns that index's
/// refusal, and no eligibility rule applies, so `.git` opens like any other name. Handles own
/// a copy of the bytes at open time, so a later [`put_file`](Self::put_file) does not change
/// a file already opened.
///
/// Paths in the builders are `/`-separated and relative to the root; missing parent
/// directories are created. A builder panics when a parent is already a non-directory: that is
/// a mistake in the test, not a behaviour to model.
#[derive(Clone)]
pub struct InMemorySite {
    inner: Arc<Inner>,
}

impl Default for InMemorySite {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySite {
    /// An empty site: just the root directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                root: Mutex::new(BTreeMap::new()),
                opens: AtomicUsize::new(0),
                on_open: Mutex::new(None),
            }),
        }
    }

    /// Adds a file with `content`.
    #[must_use]
    pub fn with_file(self, path: &str, content: &[u8]) -> Self {
        self.put_file(path, content);
        self
    }

    /// Adds an empty directory.
    #[must_use]
    pub fn with_dir(self, path: &str) -> Self {
        self.insert(path, Node::Dir(BTreeMap::new()));
        self
    }

    /// Adds a symbolic-link marker.
    #[must_use]
    pub fn with_symlink(self, path: &str) -> Self {
        self.insert(path, Node::Symlink);
        self
    }

    /// Adds a special file (a FIFO, say): neither a regular file nor a directory.
    #[must_use]
    pub fn with_special(self, path: &str) -> Self {
        self.insert(path, Node::Special);
        self
    }

    /// Adds an entry the operating system denies access to.
    #[must_use]
    pub fn with_denied(self, path: &str) -> Self {
        self.insert(path, Node::Denied);
        self
    }

    /// Adds an entry whose open fails with an unclassified I/O error of `kind`.
    #[must_use]
    pub fn with_io_error(self, path: &str, kind: ErrorKind) -> Self {
        self.insert(path, Node::Broken(kind));
        self
    }

    /// Adds or replaces a file; usable while the site is shared.
    pub fn put_file(&self, path: &str, content: &[u8]) {
        self.insert(path, Node::File(Arc::from(content)));
    }

    /// Removes an entry and everything below it; a missing entry is ignored.
    pub fn remove(&self, path: &str) {
        let mut root = self.tree();
        let mut names: Vec<&str> = split(path);
        let Some(last) = names.pop() else { return };
        let mut dir = &mut *root;
        for name in names {
            match dir.get_mut(name) {
                Some(Node::Dir(children)) => dir = children,
                _ => return,
            }
        }
        dir.remove(last);
    }

    /// Runs `hook` at the start of every `open`, before the tree is read. A test uses it to
    /// change the world mid-request. The hook must not call `on_open` itself.
    pub fn on_open(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.inner.on_open.lock().expect("hook lock") = Some(Box::new(hook));
    }

    /// How many times `open` has been called, including refused calls.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.inner.opens.load(Ordering::SeqCst)
    }

    fn tree(&self) -> MutexGuard<'_, BTreeMap<String, Node>> {
        self.inner.root.lock().expect("site tree lock")
    }

    fn insert(&self, path: &str, node: Node) {
        let mut root = self.tree();
        let mut names = split(path);
        let last = names.pop().expect("a site entry needs a non-empty path");
        let mut dir = &mut *root;
        for name in names {
            let entry = dir
                .entry(name.to_owned())
                .or_insert_with(|| Node::Dir(BTreeMap::new()));
            match entry {
                Node::Dir(children) => dir = children,
                _ => panic!("parent `{name}` of `{path}` is not a directory"),
            }
        }
        dir.insert(last.to_owned(), node);
    }
}

fn split(path: &str) -> Vec<&str> {
    path.split('/').filter(|part| !part.is_empty()).collect()
}

/// A file handle over the bytes captured at open time.
struct MemoryFile(Cursor<Arc<[u8]>>);

impl Read for MemoryFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl ContentFile for MemoryFile {
    fn len(&self) -> u64 {
        self.0.get_ref().len() as u64
    }
}

/// A directory opens its own `index.html` with the same no-follow rule and returns that
/// index's refusal rather than pretending the directory has none.
fn open_dir(children: &BTreeMap<String, Node>) -> Result<Opened, ContentRefusal> {
    let index = match children.get(INDEX) {
        None => None,
        Some(Node::Dir(_) | Node::Special) => {
            return Err(ContentRefusal::Forbidden(ForbiddenReason::NotRegular));
        }
        Some(other) => match open_leaf(other)? {
            Opened::File(file) => Some(file),
            Opened::Directory { .. } => unreachable!("directories are refused above"),
        },
    };
    Ok(Opened::Directory { index })
}

/// What the final object of a walk is, classified the way an open-then-inspect adapter does.
fn open_leaf(node: &Node) -> Result<Opened, ContentRefusal> {
    match node {
        Node::File(bytes) => Ok(Opened::File(Box::new(MemoryFile(Cursor::new(Arc::clone(
            bytes,
        )))))),
        Node::Dir(children) => open_dir(children),
        Node::Symlink => Err(ContentRefusal::Forbidden(ForbiddenReason::Symlink)),
        Node::Special => Err(ContentRefusal::Forbidden(ForbiddenReason::NotRegular)),
        Node::Denied => Err(ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied)),
        Node::Broken(kind) => Err(ContentRefusal::Io(*kind)),
    }
}

impl SiteContent for InMemorySite {
    fn open(&self, path: &SitePath) -> Result<Opened, ContentRefusal> {
        self.inner.opens.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.inner.on_open.lock().expect("hook lock").as_ref() {
            hook();
        }
        let root = self.tree();
        let names = path.components();
        let Some((last, parents)) = names.split_last() else {
            return open_dir(&root);
        };
        let mut dir = &*root;
        for name in parents {
            match dir.get(name.as_str()) {
                None => return Err(ContentRefusal::NotFound),
                Some(Node::Dir(children)) => dir = children,
                Some(Node::Denied) => {
                    return Err(ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied));
                }
                Some(Node::Broken(kind)) => return Err(ContentRefusal::Io(*kind)),
                // A link, or a file or device with more path below it: the no-follow open
                // cannot tell them apart and both are refused as `Symlink`.
                Some(_) => return Err(ContentRefusal::Forbidden(ForbiddenReason::Symlink)),
            }
        }
        match dir.get(last.as_str()) {
            None => Err(ContentRefusal::NotFound),
            Some(node) => open_leaf(node),
        }
    }
}

#[cfg(test)]
mod tests {
    use preview_domain::decode_request_path;

    use super::*;

    fn site_path(raw: &str) -> SitePath {
        decode_request_path(raw).expect("test path decodes")
    }

    fn body(opened: Opened) -> Vec<u8> {
        let (Opened::File(mut file)
        | Opened::Directory {
            index: Some(mut file),
        }) = opened
        else {
            panic!("expected a file, got {opened:?}");
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).expect("in-memory read");
        assert_eq!(file.len() as usize, bytes.len());
        bytes
    }

    fn refusal(site: &InMemorySite, raw: &str) -> ContentRefusal {
        site.open(&site_path(raw)).expect_err("expected a refusal")
    }

    #[test]
    fn should_open_file_when_path_names_a_regular_file() {
        // given
        let site = InMemorySite::new().with_file("css/site.css", b"body{}");
        // when
        let opened = site.open(&site_path("/css/site.css")).expect("opens");
        // then
        assert_eq!(body(opened), b"body{}");
    }

    #[test]
    fn should_return_not_found_when_component_is_missing() {
        // given
        let site = InMemorySite::new().with_file("a/b.html", b"");
        // when
        let leaf = refusal(&site, "/a/missing.html");
        let parent = refusal(&site, "/nope/b.html");
        // then
        assert_eq!(leaf, ContentRefusal::NotFound);
        assert_eq!(parent, ContentRefusal::NotFound);
    }

    #[test]
    fn should_refuse_when_path_crosses_symlink_marker() {
        // given
        let site = InMemorySite::new()
            .with_file("real/secret.html", b"s")
            .with_symlink("link");
        // when
        let below = refusal(&site, "/link/secret.html");
        let at = refusal(&site, "/link");
        // then
        let symlink = ContentRefusal::Forbidden(ForbiddenReason::Symlink);
        assert_eq!(below, symlink);
        assert_eq!(at, symlink);
    }

    #[test]
    fn should_refuse_as_symlink_when_intermediate_component_is_a_file() {
        // given
        let site = InMemorySite::new().with_file("a.html", b"x");
        // when
        let result = refusal(&site, "/a.html/b");
        // then
        assert_eq!(result, ContentRefusal::Forbidden(ForbiddenReason::Symlink));
    }

    #[test]
    fn should_classify_leaf_when_entry_is_special_or_denied_or_broken() {
        // given
        let site = InMemorySite::new()
            .with_special("fifo")
            .with_denied("locked")
            .with_io_error("bad", ErrorKind::InvalidData);
        // when
        let fifo = refusal(&site, "/fifo");
        let locked = refusal(&site, "/locked");
        let bad = refusal(&site, "/bad");
        // then
        assert_eq!(fifo, ContentRefusal::Forbidden(ForbiddenReason::NotRegular));
        assert_eq!(
            locked,
            ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied)
        );
        assert_eq!(bad, ContentRefusal::Io(ErrorKind::InvalidData));
    }

    #[test]
    fn should_refuse_when_denied_directory_is_crossed() {
        // given
        let site = InMemorySite::new().with_denied("locked");
        // when
        let result = refusal(&site, "/locked/a.html");
        // then
        assert_eq!(
            result,
            ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied)
        );
    }

    #[test]
    fn should_open_index_itself_when_path_is_a_directory() {
        // given
        let site = InMemorySite::new().with_file("docs/index.html", b"<p>docs</p>");
        // when
        let with_slash = site.open(&site_path("/docs/")).expect("opens");
        let without = site.open(&site_path("/docs")).expect("opens");
        // then
        assert_eq!(body(with_slash), b"<p>docs</p>");
        assert_eq!(body(without), b"<p>docs</p>");
    }

    #[test]
    fn should_open_root_directory_when_path_is_root() {
        // given
        let site = InMemorySite::new().with_file("index.html", b"home");
        let bare = InMemorySite::new();
        // when
        let opened = site.open(&site_path("/")).expect("opens");
        let no_index = bare.open(&site_path("/")).expect("opens");
        // then
        assert_eq!(body(opened), b"home");
        assert!(matches!(no_index, Opened::Directory { index: None }));
    }

    #[test]
    fn should_return_directory_without_index_when_index_is_missing() {
        // given
        let site = InMemorySite::new()
            .with_dir("empty")
            .with_file("empty/a.txt", b"");
        // when
        let opened = site.open(&site_path("/empty")).expect("opens");
        // then
        assert!(matches!(opened, Opened::Directory { index: None }));
    }

    #[test]
    fn should_return_index_refusal_when_directory_index_is_refused() {
        // given
        let site = InMemorySite::new()
            .with_symlink("linked/index.html")
            .with_special("fifo/index.html")
            .with_dir("dir/index.html")
            .with_denied("locked/index.html")
            .with_io_error("bad/index.html", ErrorKind::InvalidData);
        // when
        let [linked, fifo, dir, locked, bad] =
            ["/linked", "/fifo", "/dir", "/locked", "/bad"].map(|raw| refusal(&site, raw));
        // then
        assert_eq!(linked, ContentRefusal::Forbidden(ForbiddenReason::Symlink));
        assert_eq!(fifo, ContentRefusal::Forbidden(ForbiddenReason::NotRegular));
        assert_eq!(dir, ContentRefusal::Forbidden(ForbiddenReason::NotRegular));
        assert_eq!(
            locked,
            ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied)
        );
        assert_eq!(bad, ContentRefusal::Io(ErrorKind::InvalidData));
    }

    #[test]
    fn should_only_count_index_html_when_directory_has_other_index_names() {
        // given
        let site = InMemorySite::new().with_file("d/index.htm", b"x");
        // when
        let opened = site.open(&site_path("/d")).expect("opens");
        // then
        assert!(matches!(opened, Opened::Directory { index: None }));
    }

    #[test]
    fn should_not_apply_eligibility_when_path_is_hidden() {
        // given
        let site = InMemorySite::new()
            .with_file(".git/config", b"c")
            .with_file("node_modules/x.js", b"j");
        // when
        let hidden = site.open(&site_path("/.git/config")).expect("opens");
        let excluded = site.open(&site_path("/node_modules/x.js")).expect("opens");
        // then
        assert_eq!(body(hidden), b"c");
        assert_eq!(body(excluded), b"j");
    }

    #[test]
    fn should_keep_opened_bytes_when_file_is_replaced_afterwards() {
        // given
        let site = InMemorySite::new().with_file("a.txt", b"old");
        let opened = site.open(&site_path("/a.txt")).expect("opens");
        // when
        site.put_file("a.txt", b"new content");
        // then
        assert_eq!(body(opened), b"old");
        assert_eq!(
            body(site.open(&site_path("/a.txt")).expect("opens")),
            b"new content"
        );
    }

    #[test]
    fn should_run_hook_and_count_when_open_is_called() {
        // given
        let site = InMemorySite::new().with_file("a.txt", b"old");
        let writer = site.clone();
        site.on_open(move || writer.put_file("a.txt", b"mid-request"));
        // when
        let opened = site.open(&site_path("/a.txt"));
        let missing = site.open(&site_path("/missing"));
        // then
        assert_eq!(body(opened.expect("opens")), b"mid-request");
        assert_eq!(missing.unwrap_err(), ContentRefusal::NotFound);
        assert_eq!(site.open_count(), 2);
    }

    #[test]
    fn should_forget_subtree_when_entry_is_removed() {
        // given
        let site = InMemorySite::new().with_file("d/a.txt", b"a");
        // when
        site.remove("d");
        site.remove("never/existed");
        // then
        assert_eq!(refusal(&site, "/d/a.txt"), ContentRefusal::NotFound);
    }

    #[test]
    fn should_open_through_trait_object_when_site_is_shared_across_threads() {
        // given
        let site: Arc<dyn SiteContent> = Arc::new(InMemorySite::new().with_file("a", b"1"));
        // when
        let thread_site = Arc::clone(&site);
        let opened = std::thread::spawn(move || thread_site.open(&site_path("/a")))
            .join()
            .expect("thread ran");
        // then
        assert_eq!(body(opened.expect("opens")), b"1");
    }
}
