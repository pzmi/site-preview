//! Eligible-directory traversal over a real filesystem (plan.md section 8.1 steps 1 and 2):
//! the shared eligibility predicate applied before descent, links never followed, partial
//! failures reported, and the walk bounded.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use preview_adapter_fs::{
    MAX_DEPTH, RootDir, Traversal, TraversalFault, eligible_dirs, open_site_root,
};
use preview_domain::{Component, RelPath};
use tempfile::TempDir;

fn rel(text: &str) -> RelPath {
    RelPath::new(
        text.split('/')
            .filter(|part| !part.is_empty())
            .map(|part| Component::new(part).expect("valid component"))
            .collect(),
    )
}

fn site() -> (TempDir, std::path::PathBuf) {
    let base = tempfile::tempdir().expect("tempdir");
    let root = base.path().join("site");
    fs::create_dir(&root).expect("create root");
    fs::write(root.join("index.html"), b"home").expect("write index");
    (base, root)
}

fn walk(root: &Path, start: &str) -> Traversal {
    let site = open_site_root(root).expect("open root");
    eligible_dirs(&site.dir, &rel(start)).expect("traverse")
}

fn names(found: &Traversal) -> Vec<String> {
    found
        .dirs
        .iter()
        .map(|dir| {
            dir.components()
                .iter()
                .map(Component::as_str)
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

#[test]
fn should_include_root_when_traversing() {
    // given
    let (_base, root) = site();

    // when
    let found = walk(&root, "");

    // then
    assert_eq!(names(&found), [""]);
    assert!(found.errors.is_empty());
}

#[test]
fn should_include_nested_eligible_directories_when_traversing() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("css/fonts")).expect("create nested");
    fs::create_dir_all(root.join("my-vendor-artwork")).expect("create look-alike");

    // when
    let found = walk(&root, "");

    // then
    assert_eq!(names(&found), ["", "css", "css/fonts", "my-vendor-artwork"]);
}

#[test]
fn should_exclude_node_modules_subtree_when_traversing() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("node_modules/pkg/lib")).expect("create node_modules");
    fs::create_dir_all(root.join("docs/vendor/x")).expect("create nested vendor");
    fs::create_dir_all(root.join("Node_Modules/y")).expect("create folded name");
    fs::create_dir_all(root.join("docs/guide")).expect("create docs");

    // when
    let found = walk(&root, "");

    // then
    assert_eq!(names(&found), ["", "docs", "docs/guide"]);
}

#[test]
fn should_exclude_hidden_and_reserved_directories_when_traversing() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join(".git/objects")).expect("create .git");
    fs::create_dir_all(root.join("a/.cache/b")).expect("create nested hidden");
    fs::create_dir_all(root.join("__Site_Preview/x")).expect("create reserved");
    fs::create_dir_all(root.join("a/__site_preview")).expect("create nested look-alike");
    // the reserved name is refused at the root by `open_site_root`, so walk past that check
    let root_dir = RootDir::open_root(&root).expect("open root capability");

    // when
    let found = eligible_dirs(&root_dir, &RelPath::root()).expect("traverse");

    // then
    assert_eq!(names(&found), ["", "a", "a/__site_preview"]);
}

#[test]
fn should_not_follow_symlinked_directory_when_traversing() {
    // given
    let (base, root) = site();
    let outside = base.path().join("outside");
    fs::create_dir_all(outside.join("secret")).expect("create outside");
    fs::create_dir_all(root.join("real")).expect("create real");
    symlink(&outside, root.join("link")).expect("link outside");
    symlink("real", root.join("alias")).expect("link inside");
    symlink("..", root.join("real/up")).expect("link to parent");
    symlink("nowhere", root.join("dangling")).expect("dangling link");

    // when
    let found = walk(&root, "");

    // then
    assert_eq!(names(&found), ["", "real"]);
    assert!(found.errors.is_empty());
}

#[test]
fn should_refuse_start_when_start_is_symlink_or_missing() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join("real")).expect("create real");
    symlink("real", root.join("alias")).expect("link inside");
    let site = open_site_root(&root).expect("open root");

    // when
    let linked = eligible_dirs(&site.dir, &rel("alias"));
    let missing = eligible_dirs(&site.dir, &rel("absent"));

    // then
    assert!(linked.is_err());
    assert!(missing.is_err());
}

#[test]
fn should_start_below_root_when_start_is_nested() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("a/b/c")).expect("create nested");
    fs::create_dir_all(root.join("a/node_modules")).expect("create excluded");
    fs::create_dir(root.join("z")).expect("create sibling");

    // when
    let found = walk(&root, "a");

    // then
    assert_eq!(names(&found), ["a", "a/b", "a/b/c"]);
}

#[test]
fn should_return_empty_set_when_start_is_ineligible() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join(".git/hooks")).expect("create .git");
    fs::create_dir_all(root.join("node_modules/pkg")).expect("create node_modules");
    let root_dir = RootDir::open_root(&root).expect("open root capability");

    // when
    let hidden = eligible_dirs(&root_dir, &rel(".git")).expect("traverse");
    let excluded = eligible_dirs(&root_dir, &rel("node_modules/pkg")).expect("traverse");

    // then
    assert!(hidden.dirs.is_empty() && excluded.dirs.is_empty());
}

#[test]
fn should_skip_directory_when_name_is_not_utf8() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join(OsStr::from_bytes(b"bad-\xff"))).expect("create non-utf8 dir");
    fs::create_dir(root.join("good")).expect("create good");

    // when
    let found = walk(&root, "");

    // then
    assert_eq!(names(&found), ["", "good"]);
    assert!(found.errors.is_empty());
}

#[test]
fn should_report_unreadable_directory_when_permission_denied() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("open/inner")).expect("create readable");
    fs::create_dir_all(root.join("locked/inner")).expect("create locked");
    fs::create_dir(root.join("zzz")).expect("create later sibling");
    fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).expect("chmod 000");
    // Only uid 0 holding its DAC capabilities reads a mode-000 directory; there the denial
    // cannot be provoked, so the test asserts that this is the reason and that the whole
    // tree is then walked, instead of skipping (as tests/root.rs does).
    let os_allows = fs::read_dir(root.join("locked")).is_ok();

    // when
    let found = walk(&root, "");
    fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755))
        .expect("restore mode");

    // then
    if os_allows {
        assert!(found.errors.is_empty());
        assert_eq!(
            names(&found),
            ["", "locked", "locked/inner", "open", "open/inner", "zzz"]
        );
    } else {
        assert_eq!(names(&found), ["", "open", "open/inner", "zzz"]);
        assert_eq!(found.errors.len(), 1);
        assert_eq!(found.errors[0].dir, rel("locked"));
        assert_eq!(found.errors[0].fault, TraversalFault::PermissionDenied);
        assert_eq!(
            found.errors[0].to_string(),
            "cannot read directory `locked`: permission denied"
        );
    }
}

#[test]
fn should_stop_descending_when_nesting_exceeds_bound() {
    // given
    let (_base, root) = site();
    // a path this long is built in steps: the OS limits one path, not the tree
    let mut at = root.clone();
    for _ in 0..MAX_DEPTH + 3 {
        at = at.join("d");
        fs::create_dir(&at).expect("create level");
    }

    // when
    let found = walk(&root, "");

    // then
    assert_eq!(found.dirs.len(), MAX_DEPTH + 1);
    assert_eq!(found.errors.len(), 1);
    assert_eq!(found.errors[0].fault, TraversalFault::TooDeep);
    assert_eq!(found.errors[0].dir.components().len(), MAX_DEPTH + 1);
}
