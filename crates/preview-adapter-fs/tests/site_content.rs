//! `RootDir` as the application's `SiteContent` port over a real filesystem (plan.md section
//! 5.1, ADR 0038): directory index resolution, its refusals, and refusals that carry no path.

use std::fs;
use std::io::{ErrorKind, Read};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use preview_adapter_fs::open_site_root;
use preview_application::{ContentFile, ContentRefusal, ForbiddenReason, Opened, SiteContent};
use preview_domain::decode_request_path;
use tempfile::TempDir;

struct Site {
    _base: TempDir,
    root: PathBuf,
    outside: PathBuf,
}

impl Site {
    fn new() -> Self {
        let base = tempfile::tempdir().expect("tempdir");
        let root = base.path().join("site");
        let outside = base.path().join("outside");
        fs::create_dir(&root).expect("create root");
        fs::create_dir(&outside).expect("create outside");
        fs::write(outside.join("index.html"), b"outside secret").expect("write secret");
        Self {
            _base: base,
            root,
            outside,
        }
    }

    fn write(&self, rel: &str, body: &[u8]) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().expect("has parent")).expect("create parents");
        fs::write(path, body).expect("write file");
    }

    fn mkdir(&self, rel: &str) {
        fs::create_dir_all(self.root.join(rel)).expect("create dir");
    }

    fn link(&self, rel: &str, target: &Path) {
        symlink(target, self.root.join(rel)).expect("create symlink");
    }

    fn fifo(&self, rel: &str) {
        let status = Command::new("mkfifo")
            .arg(self.root.join(rel))
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed");
    }

    fn content(&self) -> Box<dyn SiteContent> {
        Box::new(open_site_root(&self.root).expect("open root").dir)
    }
}

fn open(content: &dyn SiteContent, raw: &str) -> Result<Opened, ContentRefusal> {
    content.open(&decode_request_path(raw).expect("valid request path"))
}

fn read_all(mut file: Box<dyn ContentFile>) -> Vec<u8> {
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("read file");
    assert_eq!(u64::try_from(body.len()).expect("small file"), file.len());
    body
}

fn index_body(opened: Opened) -> Vec<u8> {
    let Opened::Directory { index: Some(file) } = opened else {
        panic!("expected a directory with an index, got {opened:?}");
    };
    read_all(file)
}

const SYMLINK: ContentRefusal = ContentRefusal::Forbidden(ForbiddenReason::Symlink);
const NOT_REGULAR: ContentRefusal = ContentRefusal::Forbidden(ForbiddenReason::NotRegular);

#[test]
fn should_return_directory_with_index_when_index_present() {
    // given
    let site = Site::new();
    site.write("index.html", b"home");
    site.write("docs/index.html", b"<p>docs</p>");
    let content = site.content();

    // when
    let root = open(&*content, "/");
    let docs = open(&*content, "/docs/");

    // then
    assert_eq!(index_body(root.expect("root opens")), b"home");
    assert_eq!(index_body(docs.expect("docs opens")), b"<p>docs</p>");
}

#[test]
fn should_open_same_object_when_trailing_slash_differs() {
    // given
    let site = Site::new();
    site.write("docs/index.html", b"docs");
    site.write("page.html", b"page");
    let content = site.content();

    // when
    let dir = open(&*content, "/docs");
    let file = open(&*content, "/page.html/");

    // then
    assert_eq!(index_body(dir.expect("directory opens")), b"docs");
    let Ok(Opened::File(file)) = file else {
        panic!("expected the file, got {file:?}");
    };
    assert_eq!(read_all(file), b"page");
}

#[test]
fn should_return_file_when_path_is_regular_file() {
    // given
    let site = Site::new();
    site.write("css/site.css", b"body{}");
    let content = site.content();

    // when
    let opened = open(&*content, "/css/site.css");

    // then
    let Ok(Opened::File(file)) = opened else {
        panic!("expected a file, got {opened:?}");
    };
    assert_eq!(read_all(file), b"body{}");
}

#[test]
fn should_return_directory_without_index_when_index_missing() {
    // given
    let site = Site::new();
    site.write("assets/index.htm", b"not the index");
    let content = site.content();

    // when
    let opened = open(&*content, "/assets/");

    // then
    assert!(
        matches!(opened, Ok(Opened::Directory { index: None })),
        "got {opened:?}"
    );
}

#[test]
fn should_refuse_when_index_html_is_symlink() {
    // given
    let site = Site::new();
    site.mkdir("outside-link");
    site.link("outside-link/index.html", &site.outside.join("index.html"));
    site.write("inside-link/real.html", b"inside");
    site.link("inside-link/index.html", Path::new("real.html"));
    let content = site.content();

    // when
    let outside = open(&*content, "/outside-link/");
    let inside = open(&*content, "/inside-link/");

    // then
    assert_eq!(outside.map(|_| ()), Err(SYMLINK));
    assert_eq!(inside.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_index_html_is_not_regular_file() {
    // given
    let site = Site::new();
    site.mkdir("dir-index/index.html");
    site.mkdir("fifo-index");
    site.fifo("fifo-index/index.html");
    let content = site.content();

    // when
    let dir_index = open(&*content, "/dir-index/");
    let fifo_index = open(&*content, "/fifo-index/");

    // then
    assert_eq!(dir_index.map(|_| ()), Err(NOT_REGULAR));
    assert_eq!(fifo_index.map(|_| ()), Err(NOT_REGULAR));
}

#[test]
fn should_refuse_when_directory_itself_is_symlink() {
    // given
    let site = Site::new();
    site.link("docs", &site.outside);
    let content = site.content();

    // when
    let opened = open(&*content, "/docs/");

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_not_leak_absolute_path_when_io_error() {
    // given
    let site = Site::new();
    site.link("link.html", &site.outside.join("index.html"));
    site.fifo("pipe");
    let content = site.content();
    let too_long = format!("/{}", "a".repeat(300));

    // when
    let refusals = [
        open(&*content, &too_long),
        open(&*content, "/missing.html"),
        open(&*content, "/link.html"),
        open(&*content, "/pipe"),
    ]
    .map(|opened| opened.expect_err("refused"));

    // then
    assert!(
        matches!(refusals[0], ContentRefusal::Io(kind) if kind != ErrorKind::NotFound),
        "a name over NAME_MAX is an I/O error: {:?}",
        refusals[0]
    );
    let base = site.root.parent().expect("root has a parent");
    for refusal in refusals {
        let text = format!("{refusal} {refusal:?}");
        assert!(!text.contains('/'), "a refusal rendered a path: {text}");
        assert!(
            !text.contains(&*base.to_string_lossy()),
            "a refusal leaked the temp dir: {text}"
        );
    }
}
