//! Real-OS watcher tests. They share the system's watch resources, so the module is named
//! `os_watcher` and nextest runs it in the `os-watcher` group. A wait for a native
//! notification is a bounded condition poll (`wait_until`): the OS delivers asynchronously and
//! no readiness signal exists; the deadline only limits how long a failing test hangs.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use preview_domain::{ChangeEvent, ChangeKind, Component, RelPath};
use preview_testkit::poll::wait_until;
use tempfile::TempDir;

use crate::{NotifyWatcher, SiteRoot, WatchFault, eligible_dirs, open_site_root, translate};

const DEADLINE: Duration = Duration::from_secs(10);

type Notifications = Receiver<notify::Result<notify::Event>>;

fn site() -> (TempDir, PathBuf) {
    let base = tempfile::tempdir().expect("create temp dir");
    let root = base.path().join("site");
    fs::create_dir(&root).expect("create root");
    fs::write(root.join("index.html"), b"home").expect("write index");
    (base, root)
}

fn rel(text: &str) -> RelPath {
    RelPath::new(
        text.split('/')
            .filter(|part| !part.is_empty())
            .map(|part| Component::new(part).expect("valid component"))
            .collect(),
    )
}

fn names(dirs: impl IntoIterator<Item = RelPath>) -> Vec<String> {
    dirs.into_iter()
        .map(|dir| {
            dir.components()
                .iter()
                .map(Component::as_str)
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

fn start(site: &SiteRoot) -> (NotifyWatcher, Vec<crate::RegistrationError>, Notifications) {
    let (sender, receiver) = channel();
    let (watcher, errors) =
        NotifyWatcher::start(&site.dir, &site.display_path, sender).expect("start watcher");
    (watcher, errors, receiver)
}

/// Waits until a notification names `target`, collecting every path seen on the way.
fn wait_for_path(notifications: &Notifications, target: &Path, seen: &mut Vec<PathBuf>) {
    let mut found = false;
    wait_until(DEADLINE, "a notification naming the target", || {
        while let Ok(result) = notifications.try_recv() {
            let event = result.expect("no watcher error");
            found |= event.paths.iter().any(|path| path == target);
            seen.extend(event.paths);
        }
        found
    })
    .expect("target notified");
}

#[test]
fn should_register_every_eligible_directory_when_started() {
    // given
    let (_base, root) = site();
    for dir in ["css/fonts", "docs/guide/deep", "images"] {
        fs::create_dir_all(root.join(dir)).expect("create dir");
    }
    let site = open_site_root(&root).expect("open root");
    let expected = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");

    // when
    let (watcher, errors, notifications) = start(&site);
    let target = site.display_path.join("docs/guide/deep/page.html");
    fs::write(&target, b"new").expect("write nested file");

    // then
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(watcher.watched().len(), expected.dirs.len());
    assert_eq!(
        names(watcher.watched().iter().cloned()),
        names(expected.dirs)
    );
    wait_for_path(&notifications, &target, &mut Vec::new());
}

#[test]
fn should_not_register_ignored_directory_when_started() {
    // given
    let (_base, root) = site();
    for dir in [
        "node_modules/pkg",
        ".git/objects",
        "docs/vendor",
        "docs/.cache",
    ] {
        fs::create_dir_all(root.join(dir)).expect("create dir");
    }
    let site = open_site_root(&root).expect("open root");

    // when
    let (watcher, errors, notifications) = start(&site);
    fs::write(site.display_path.join("node_modules/pkg/index.js"), b"x").expect("write ignored");
    fs::write(site.display_path.join(".git/objects/abc"), b"x").expect("write ignored");
    fs::write(site.display_path.join("docs/vendor/lib.js"), b"x").expect("write ignored");
    // A later change in a watched directory: a single native queue delivers in order, so once
    // it arrives any notification from an ignored directory would have arrived before it.
    let marker = site.display_path.join("docs/marker.html");
    fs::write(&marker, b"x").expect("write marker");

    // then
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(names(watcher.watched().iter().cloned()), ["", "docs"]);
    let mut seen = Vec::new();
    wait_for_path(&notifications, &marker, &mut seen);
    let leaked: Vec<_> = seen
        .iter()
        .filter(|path| {
            ["node_modules", ".git", "docs/vendor"]
                .iter()
                .any(|ignored| path.starts_with(site.display_path.join(ignored)))
        })
        .collect();
    assert!(leaked.is_empty(), "ignored paths notified: {leaked:?}");
}

#[test]
fn should_report_directory_and_reason_when_registration_fails() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("open/inner")).expect("create readable");
    fs::create_dir(root.join("locked")).expect("create locked");
    let site = open_site_root(&root).expect("open root");
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    let locked = site.display_path.join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    // Only uid 0 holding its DAC capabilities may watch a mode-000 directory; there the denial
    // cannot be provoked, so the test asserts that this is the reason and that the directory
    // is then watched (the vanished-directory test below covers the report on every uid).
    let os_allows = fs::read_dir(&locked).is_ok();
    let (sender, _notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");

    // when
    let errors = watcher.register(&site.dir, traversal);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore mode");

    // then
    if os_allows {
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert!(watcher.watched().contains(&rel("locked")));
    } else {
        assert_eq!(errors.len(), 1, "errors: {errors:?}");
        assert_eq!(errors[0].dir, rel("locked"));
        assert_eq!(errors[0].fault, WatchFault::PermissionDenied);
        assert_eq!(
            errors[0].to_string(),
            "cannot watch directory `locked`: permission denied"
        );
        assert!(!watcher.watched().contains(&rel("locked")));
    }
    assert_eq!(
        names(watcher.watched().iter().cloned())
            .into_iter()
            .filter(|name| name != "locked")
            .collect::<Vec<_>>(),
        ["", "open", "open/inner"]
    );
}

#[test]
fn should_report_directory_and_reason_when_directory_vanishes_before_registration() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("gone/inner")).expect("create doomed");
    fs::create_dir(root.join("kept")).expect("create kept");
    let site = open_site_root(&root).expect("open root");
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    fs::remove_dir_all(site.display_path.join("gone")).expect("remove after traversal");
    let (sender, _notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");

    // when
    let errors = watcher.register(&site.dir, traversal);

    // then
    assert_eq!(
        names(errors.iter().map(|e| e.dir.clone())),
        ["gone", "gone/inner"]
    );
    assert!(errors.iter().all(|e| e.fault == WatchFault::NotFound));
    assert_eq!(
        errors[0].to_string(),
        "cannot watch directory `gone`: not found or no longer a directory"
    );
    assert_eq!(names(watcher.watched().iter().cloned()), ["", "kept"]);
}

#[test]
fn should_withdraw_watch_when_link_replaces_directory_before_registration() {
    // given
    let (base, root) = site();
    fs::create_dir(root.join("assets")).expect("create assets");
    let outside = base.path().join("outside");
    fs::create_dir(&outside).expect("create outside");
    let site = open_site_root(&root).expect("open root");
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    let assets = site.display_path.join("assets");
    fs::remove_dir(&assets).expect("remove assets");
    std::os::unix::fs::symlink(&outside, &assets).expect("swap in a link");
    let (sender, _notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");

    // when
    let errors = watcher.register(&site.dir, traversal);

    // then
    assert_eq!(names(errors.iter().map(|e| e.dir.clone())), ["assets"]);
    assert_eq!(errors[0].fault, WatchFault::NotFound);
    assert_eq!(names(watcher.watched().iter().cloned()), [""]);
}

#[test]
fn should_not_register_again_when_directory_already_watched() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join("docs")).expect("create docs");
    let site = open_site_root(&root).expect("open root");
    let (mut watcher, _errors, _notifications) = start(&site);
    let again = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    fs::remove_dir(site.display_path.join("docs")).expect("remove docs");

    // when
    let errors = watcher.register(&site.dir, again);

    // then
    assert!(
        errors.is_empty(),
        "a watched directory was registered again: {errors:?}"
    );
    assert_eq!(names(watcher.watched().iter().cloned()), ["", "docs"]);
}

#[test]
fn should_translate_native_rename_to_both_relative_names_when_file_is_renamed() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join("docs")).expect("create docs");
    fs::write(root.join("docs/draft.tmp"), b"x").expect("write draft");
    let site = open_site_root(&root).expect("open root");
    let (_watcher, _errors, notifications) = start(&site);

    // when
    fs::rename(
        site.display_path.join("docs/draft.tmp"),
        site.display_path.join("docs/page.html"),
    )
    .expect("rename");

    // then
    let mut changes = Vec::new();
    let expected = [
        ChangeEvent {
            kind: ChangeKind::Rename,
            path: rel("docs/draft.tmp"),
        },
        ChangeEvent {
            kind: ChangeKind::Rename,
            path: rel("docs/page.html"),
        },
    ];
    wait_until(DEADLINE, "both sides of the rename translated", || {
        while let Ok(result) = notifications.try_recv() {
            let event = result.expect("no watcher error");
            changes.extend(translate(event, &site.display_path));
        }
        expected.iter().all(|wanted| changes.contains(wanted))
    })
    .expect("rename translated");
}
