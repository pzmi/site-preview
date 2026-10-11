//! Opening the site root over a real filesystem (plan.md sections 3 and 6): the reserved
//! name in every entry type and letter case, a file or missing root, a root given through a
//! link, and an unreadable root.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;

use preview_adapter_fs::{Opened, RootError, open_site_root};
use preview_domain::decode_request_path;
use tempfile::TempDir;

fn tempdir() -> TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn site(base: &TempDir) -> std::path::PathBuf {
    let root = base.path().join("site");
    fs::create_dir(&root).expect("create root");
    fs::write(root.join("index.html"), b"home").expect("write index");
    root
}

fn error_of(requested: &Path) -> RootError {
    open_site_root(requested).expect_err("root must be refused")
}

/// The effective uid of this process: a file it creates is owned by it.
fn effective_uid(base: &TempDir) -> u32 {
    let probe = base.path().join("uid-probe");
    fs::write(&probe, b"").expect("write probe");
    fs::metadata(&probe).expect("probe metadata").uid()
}

#[test]
fn should_fail_with_reserved_conflict_when_site_preview_dir_exists() {
    // given
    let base = tempdir();
    let root = site(&base);
    fs::create_dir(root.join("__site_preview")).expect("create reserved dir");

    // when
    let error = error_of(&root);

    // then
    assert_eq!(error, RootError::ReservedPathConflict);
}

#[test]
fn should_fail_with_reserved_conflict_when_site_preview_is_file_or_dangling_link() {
    // given
    let base = tempdir();
    let with_file = site(&base);
    fs::write(with_file.join("__site_preview"), b"x").expect("create reserved file");
    let with_link = base.path().join("linked");
    fs::create_dir(&with_link).expect("create second root");
    symlink("nowhere", with_link.join("__site_preview")).expect("create dangling link");

    // when
    let errors = [error_of(&with_file), error_of(&with_link)];

    // then
    assert_eq!(errors, [RootError::ReservedPathConflict; 2]);
}

#[test]
fn should_fail_with_reserved_conflict_when_name_differs_only_in_case() {
    // given
    let base = tempdir();
    let names = ["__Site_Preview", "__SITE_PREVIEW", "__s\u{0131}te_preview"];
    let roots: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let root = base.path().join(format!("site-{index}"));
            fs::create_dir_all(root.join(name)).expect("create reserved dir");
            fs::write(root.join(name).join("page.html"), b"shadowed").expect("write page");
            root
        })
        .collect();

    // when
    let errors: Vec<_> = roots.iter().map(|root| error_of(root)).collect();

    // then
    assert_eq!(errors, [RootError::ReservedPathConflict; 3]);
}

#[test]
fn should_open_when_root_holds_names_close_to_reserved_one() {
    // given
    let base = tempdir();
    let root = site(&base);
    fs::create_dir(root.join("site_preview")).expect("create near-miss dir");
    fs::write(root.join("__site_preview.html"), b"x").expect("create near-miss file");

    // when
    let opened = open_site_root(&root);

    // then
    assert!(
        opened.is_ok(),
        "a near-miss name is not reserved: {opened:?}"
    );
}

#[test]
fn should_open_when_reserved_name_is_only_nested() {
    // given
    let base = tempdir();
    let root = site(&base);
    fs::create_dir_all(root.join("docs/__site_preview")).expect("create nested dir");

    // when
    let opened = open_site_root(&root);

    // then
    assert!(
        opened.is_ok(),
        "a nested __site_preview is not reserved: {opened:?}"
    );
}

#[test]
fn should_fail_when_root_is_file() {
    // given
    let base = tempdir();
    let root = site(&base);

    // when
    let error = error_of(&root.join("index.html"));

    // then
    assert_eq!(error, RootError::NotDirectory);
}

#[test]
fn should_fail_when_root_is_missing() {
    // given
    let base = tempdir();

    // when
    let error = error_of(&base.path().join("absent"));

    // then
    assert_eq!(error, RootError::NotFound);
}

#[test]
fn should_open_when_root_given_via_symlink() {
    // given
    let base = tempdir();
    let root = site(&base);
    let link = base.path().join("link-to-site");
    symlink(&root, &link).expect("create root link");

    // when
    let opened = open_site_root(&link).expect("a linked root opens");

    // then
    let canonical = fs::canonicalize(&root).expect("canonical root");
    assert_eq!(opened.display_path, canonical);
    let index = opened
        .dir
        .open(&decode_request_path("/index.html").expect("valid path"))
        .expect("index opens beneath the linked root");
    assert!(matches!(index, Opened::File(file) if file.len() == 4));
}

#[test]
fn should_fail_when_root_unreadable() {
    // given
    let base = tempdir();
    let root = site(&base);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    let uid = effective_uid(&base);
    // Only uid 0 holding its DAC capabilities (a container or CI sandbox) reads a mode-000
    // directory; there the refusal cannot be provoked, so the test asserts that this is the
    // reason and that the root then opens, instead of skipping.
    let os_allows = fs::read_dir(&root).is_ok();

    // when
    let opened = open_site_root(&root);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).expect("restore mode");

    // then
    if os_allows {
        assert_eq!(uid, 0, "only the superuser may read a mode-000 directory");
        assert!(
            opened.is_ok(),
            "the OS allowed reading, so the root opens: {opened:?}"
        );
    } else {
        assert_eq!(opened.map(|_| ()), Err(RootError::NotReadable));
    }
}
