//! The root capability over a real filesystem (plan.md section 5.3, ADR 0004): symbolic links
//! in every position, non-regular objects and missing entries, each in its own temporary
//! directory.

use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;

use preview_adapter_fs::{OpenError, Opened, RootDir};
use preview_application::{ContentRefusal, ForbiddenReason};
use preview_domain::{Component, SitePath, decode_request_path};
use tempfile::TempDir;

const SYMLINK: OpenError = ContentRefusal::Forbidden(ForbiddenReason::Symlink);
const NOT_REGULAR: OpenError = ContentRefusal::Forbidden(ForbiddenReason::NotRegular);

/// A site root and a sibling directory outside it, both temporary.
struct Fixture {
    _base: TempDir,
    root: std::path::PathBuf,
    outside: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().expect("tempdir");
        let root = base.path().join("site");
        let outside = base.path().join("outside");
        fs::create_dir(&root).expect("create root");
        fs::create_dir(&outside).expect("create outside");
        fs::write(outside.join("secret.txt"), b"outside secret").expect("write secret");
        Self {
            _base: base,
            root,
            outside,
        }
    }

    fn write(&self, rel: &str, body: &[u8]) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parents");
        }
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

    fn capability(&self) -> RootDir {
        RootDir::open_root(&self.root).expect("open root")
    }
}

fn site_path(raw: &str) -> SitePath {
    decode_request_path(raw).expect("valid request path")
}

fn read_file(opened: Opened) -> Vec<u8> {
    let Opened::File(file) = opened else {
        panic!("expected a file, got {opened:?}");
    };
    let len = file.len();
    let mut body = Vec::new();
    fs::File::from(file.into_fd())
        .read_to_end(&mut body)
        .expect("read opened file");
    assert_eq!(u64::try_from(body.len()).expect("small file"), len);
    body
}

#[test]
fn should_open_when_path_is_regular_file() {
    // given
    let site = Fixture::new();
    site.write("docs/guide/page.html", b"<p>hello</p>");
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/docs/guide/page.html"));

    // then
    let body = read_file(opened.expect("regular file opens"));
    assert_eq!(body, b"<p>hello</p>");
}

#[test]
fn should_report_zero_length_when_file_is_empty() {
    // given
    let site = Fixture::new();
    site.write("empty.txt", b"");
    let root = site.capability();

    // when
    let opened = root
        .open(&site_path("/empty.txt"))
        .expect("empty file opens");

    // then
    let Opened::File(file) = opened else {
        panic!("expected a file, got {opened:?}");
    };
    assert!(file.is_empty());
}

#[test]
fn should_refuse_when_final_component_is_symlink_to_outside() {
    // given
    let site = Fixture::new();
    site.link("leak.txt", &site.outside.join("secret.txt"));
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/leak.txt"));

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_middle_component_is_symlink() {
    // given
    let site = Fixture::new();
    site.link("escape", &site.outside);
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/escape/secret.txt"));

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_symlink_points_inside_root() {
    // given
    let site = Fixture::new();
    site.write("real/page.html", b"inside");
    site.link("alias.html", &site.root.join("real/page.html"));
    site.link("alias-dir", Path::new("real"));
    let root = site.capability();

    // when
    let final_link = root.open(&site_path("/alias.html"));
    let middle_link = root.open(&site_path("/alias-dir/page.html"));

    // then
    assert_eq!(final_link.map(|_| ()), Err(SYMLINK));
    assert_eq!(middle_link.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_final_component_is_symlink_to_directory() {
    // given
    let site = Fixture::new();
    site.link("out", &site.outside);
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/out/"));

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_symlink_is_dangling() {
    // given
    let site = Fixture::new();
    site.link("gone.html", &site.root.join("missing.html"));
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/gone.html"));

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_target_is_fifo() {
    // given
    let site = Fixture::new();
    site.fifo("pipe");
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/pipe"));

    // then
    assert_eq!(opened.map(|_| ()), Err(NOT_REGULAR));
}

#[test]
fn should_refuse_when_target_is_socket() {
    // given
    let site = Fixture::new();
    let _listener = UnixListener::bind(site.root.join("sock")).expect("bind socket");
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/sock"));

    // then
    assert_eq!(opened.map(|_| ()), Err(NOT_REGULAR));
}

#[test]
fn should_refuse_when_middle_component_is_regular_file() {
    // given
    let site = Fixture::new();
    site.write("file.txt", b"not a directory");
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/file.txt/child"));

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_refuse_when_middle_component_is_fifo() {
    // given
    let site = Fixture::new();
    site.fifo("pipe");
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/pipe/child"));

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_return_not_found_when_component_is_missing() {
    // given
    let site = Fixture::new();
    site.mkdir("present");
    let root = site.capability();

    // when
    let missing_final = root.open(&site_path("/present/absent.html"));
    let missing_middle = root.open(&site_path("/absent/page.html"));

    // then
    assert_eq!(missing_final.map(|_| ()), Err(ContentRefusal::NotFound));
    assert_eq!(missing_middle.map(|_| ()), Err(ContentRefusal::NotFound));
}

#[test]
fn should_open_directory_when_path_names_directory() {
    // given
    let site = Fixture::new();
    site.write("blog/index.html", b"blog");
    let root = site.capability();

    // when
    let with_slash = root.open(&site_path("/blog/"));
    let without_slash = root.open(&site_path("/blog"));

    // then
    assert!(matches!(with_slash, Ok(Opened::Directory(_))));
    assert!(matches!(without_slash, Ok(Opened::Directory(_))));
}

#[test]
fn should_open_index_through_directory_when_root_path_requested() {
    // given
    let site = Fixture::new();
    site.write("index.html", b"home");
    let root = site.capability();
    let index = Component::new("index.html").expect("valid name");

    // when
    let opened = root.open(&site_path("/")).expect("root opens");

    // then
    let Opened::Directory(dir) = opened else {
        panic!("expected the root directory, got {opened:?}");
    };
    let body = read_file(dir.open_child(&index).expect("index opens"));
    assert_eq!(body, b"home");
}

#[test]
fn should_refuse_when_child_of_opened_directory_is_symlink() {
    // given
    let site = Fixture::new();
    site.mkdir("blog");
    site.link("blog/index.html", &site.outside.join("secret.txt"));
    let root = site.capability();
    let index = Component::new("index.html").expect("valid name");
    let Ok(Opened::Directory(dir)) = root.open(&site_path("/blog/")) else {
        panic!("expected the blog directory");
    };

    // when
    let opened = dir.open_child(&index);

    // then
    assert_eq!(opened.map(|_| ()), Err(SYMLINK));
}

#[test]
fn should_keep_resolving_under_original_root_when_root_path_is_replaced_by_symlink() {
    // given
    let site = Fixture::new();
    site.write("page.html", b"original");
    let root = site.capability();
    let moved = site.root.with_file_name("moved");
    fs::rename(&site.root, &moved).expect("move root away");
    symlink(&site.outside, &site.root).expect("put a link where the root was");

    // when
    let original = root.open(&site_path("/page.html"));
    let outside = root.open(&site_path("/secret.txt"));

    // then
    assert_eq!(read_file(original.expect("still opens")), b"original");
    assert_eq!(outside.map(|_| ()), Err(ContentRefusal::NotFound));
}

#[test]
fn should_follow_os_decision_when_file_permissions_deny_reading() {
    // given
    let site = Fixture::new();
    site.write("locked.html", b"locked");
    let locked = site.root.join("locked.html");
    let mut perms = fs::metadata(&locked).expect("metadata").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
    fs::set_permissions(&locked, perms).expect("chmod 000");
    // A privileged user (root, or CAP_DAC_OVERRIDE in a container) may read a mode-000
    // file; the capability must agree with the operating system either way.
    let os_allows = fs::File::open(&locked).is_ok();
    let root = site.capability();

    // when
    let opened = root.open(&site_path("/locked.html"));

    // then
    if os_allows {
        assert_eq!(read_file(opened.expect("privileged open")), b"locked");
    } else {
        assert_eq!(
            opened.map(|_| ()),
            Err(ContentRefusal::Forbidden(ForbiddenReason::PermissionDenied))
        );
    }
}

#[test]
fn should_fail_when_root_is_not_a_directory() {
    // given
    let site = Fixture::new();
    site.write("file.txt", b"x");

    // when
    let opened = RootDir::open_root(&site.root.join("file.txt"));

    // then
    assert!(opened.is_err());
}
