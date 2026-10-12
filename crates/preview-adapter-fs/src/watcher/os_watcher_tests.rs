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

use crate::watcher::{NotifyWatcher, RETRY_BUDGET, ReconcileHint, translate};
use crate::{MAX_DEPTH, NotifyChangeSource, SiteRoot, WatchFault, eligible_dirs, open_site_root};
use preview_application::{ChangeBatch, ChangeSource, SourceFailure};

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

/// Polls the source once without a runtime; the source never needs one.
fn poll_next(source: &mut NotifyChangeSource) -> Option<ChangeBatch<ChangeEvent>> {
    let future = core::pin::pin!(source.next());
    match future.poll(&mut core::task::Context::from_waker(
        core::task::Waker::noop(),
    )) {
        core::task::Poll::Ready(batch) => Some(batch),
        core::task::Poll::Pending => None,
    }
}

#[test]
fn should_deliver_relative_change_when_file_written_under_change_source() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("docs/guide")).expect("create docs");
    fs::create_dir(root.join("node_modules")).expect("create ignored");
    let site = open_site_root(&root).expect("open root");
    let (mut source, report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    fs::write(site.display_path.join("node_modules/x.js"), b"x").expect("write ignored");
    fs::write(site.display_path.join("docs/guide/page.html"), b"x").expect("write page");

    // then
    assert!(report.is_complete(), "unexpected report: {report:?}");
    assert_eq!(report.watching, 3);
    let mut seen = Vec::new();
    wait_until(DEADLINE, "the page change delivered", || {
        while let Some(batch) = poll_next(&mut source) {
            match batch {
                ChangeBatch::Events(changes) => seen.extend(changes),
                other => panic!("unexpected batch: {other:?}"),
            }
        }
        seen.iter()
            .any(|change| change.path == rel("docs/guide/page.html"))
    })
    .expect("change delivered");
    assert!(
        seen.iter()
            .all(|change| !change.path.starts_with(&rel("node_modules"))),
        "ignored subtree delivered: {seen:?}"
    );
}

#[test]
fn should_report_registration_failure_first_when_a_directory_cannot_be_watched() {
    // given
    let (_base, root) = site();
    // a path this long is built in steps: the OS limits one path, not the tree
    let mut at = root.clone();
    for _ in 0..MAX_DEPTH + 2 {
        at = at.join("d");
        fs::create_dir(&at).expect("create level");
    }
    let site = open_site_root(&root).expect("open root");

    // when
    let (mut source, report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // then
    assert_eq!(report.watching, MAX_DEPTH + 1);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].fault, WatchFault::TooDeep);
    assert_eq!(
        poll_next(&mut source),
        Some(ChangeBatch::Failed(SourceFailure::RegistrationFailed))
    );
}

#[test]
fn should_return_closed_when_source_is_closed() {
    // given
    let (_base, root) = site();
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    source.close();
    fs::write(site.display_path.join("late.html"), b"x").expect("write after close");

    // then
    assert_eq!(poll_next(&mut source), Some(ChangeBatch::Closed));
    assert_eq!(poll_next(&mut source), Some(ChangeBatch::Closed));
}

/// Drains every batch available now, as the pipeline would: each batch of changes is
/// followed by a reconcile of what it touched. Returns the changes and the reconcile
/// failures seen.
fn pump(
    source: &mut NotifyChangeSource,
    site: &SiteRoot,
    changes: &mut Vec<ChangeEvent>,
    failures: &mut Vec<crate::RegistrationError>,
) {
    while let Some(batch) = poll_next(source) {
        match batch {
            ChangeBatch::Events(batch) => {
                changes.extend(batch);
                failures.extend(source.reconcile_changed(&site.dir).errors);
            }
            other => panic!("unexpected batch: {other:?}"),
        }
    }
}

/// Pumps until `done` holds; bounded, since native notifications arrive asynchronously.
fn pump_until(
    source: &mut NotifyChangeSource,
    site: &SiteRoot,
    what: &str,
    mut done: impl FnMut(&NotifyChangeSource, &[ChangeEvent]) -> bool,
) -> Vec<crate::RegistrationError> {
    let mut changes = Vec::new();
    let mut failures = Vec::new();
    wait_until(DEADLINE, what, || {
        pump(source, site, &mut changes, &mut failures);
        done(source, &changes)
    })
    .unwrap_or_else(|_| panic!("{what}: changes {changes:?}, failures {failures:?}"));
    failures
}

fn names_path(changes: &[ChangeEvent], path: &str) -> bool {
    changes.iter().any(|change| change.path == rel(path))
}

fn watching(source: &NotifyChangeSource, dirs: &[&str]) -> bool {
    let watched = source.watched();
    dirs.iter().all(|dir| watched.contains(&rel(dir)))
}

fn watching_none(source: &NotifyChangeSource, dirs: &[&str]) -> bool {
    let watched = source.watched();
    dirs.iter().all(|dir| !watched.contains(&rel(dir)))
}

#[test]
fn should_watch_new_nested_directory_with_descendants_when_created() {
    // given
    let (_base, root) = site();
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when: the inner directory exists before anything can watch the outer one
    fs::create_dir_all(site.display_path.join("a/b")).expect("mkdir -p");
    let failures = pump_until(&mut source, &site, "a and a/b watched", |source, _| {
        watching(source, &["a", "a/b"])
    });
    fs::write(site.display_path.join("a/b/page.css"), b"x").expect("edit inside");

    // then
    assert!(failures.is_empty(), "{failures:?}");
    pump_until(
        &mut source,
        &site,
        "the nested edit delivered",
        |_, changes| names_path(changes, "a/b/page.css"),
    );
}

/// Writes a new file in `dir` on every poll until one of them is delivered: a directory whose
/// watch is not (yet) in place reports nothing, so delivery shows the watch works. Bounded.
fn probe_until_delivered(source: &mut NotifyChangeSource, site: &SiteRoot, dir: &str) {
    let mut written = Vec::new();
    let mut changes = Vec::new();
    let mut failures = Vec::new();
    wait_until(DEADLINE, "a probe write delivered", || {
        let name = format!("{dir}/probe-{}.html", written.len());
        fs::write(site.display_path.join(&name), b"x").expect("write probe");
        written.push(rel(&name));
        pump(source, site, &mut changes, &mut failures);
        changes.iter().any(|change| written.contains(&change.path))
    })
    .unwrap_or_else(|_| panic!("no probe in `{dir}` delivered: {changes:?}"));
}

/// Writes a file in the root and waits for it: one native queue delivers in order, so every
/// notification of what happened before has been handled by then.
fn settle_with_marker(source: &mut NotifyChangeSource, site: &SiteRoot) {
    fs::write(site.display_path.join("marker.html"), b"x").expect("write marker");
    let mut changes = Vec::new();
    wait_until(DEADLINE, "the marker delivered", || {
        while let Some(batch) = poll_next(source) {
            if let ChangeBatch::Events(batch) = batch {
                changes.extend(batch);
            }
        }
        names_path(&changes, "marker.html")
    })
    .expect("marker delivered");
}

#[test]
fn should_rewatch_directory_when_deleted_and_recreated() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("docs/inner")).expect("create docs");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);
    let docs = site.display_path.join("docs");

    // when
    fs::remove_dir_all(&docs).expect("rm -r docs");
    fs::create_dir(&docs).expect("mkdir docs");

    // then
    probe_until_delivered(&mut source, &site, "docs");
    assert!(watching(&source, &["", "docs"]));
    assert!(watching_none(&source, &["docs/inner"]));
}

#[test]
fn should_drop_stale_watch_when_directory_removed() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("old/inner/deep")).expect("create old");
    fs::create_dir(root.join("kept")).expect("create kept");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    fs::remove_dir_all(site.display_path.join("old")).expect("rm -r old");

    // then
    pump_until(&mut source, &site, "old forgotten", |source, _| {
        watching_none(source, &["old", "old/inner", "old/inner/deep"])
    });
    assert_eq!(names(source.watched()), ["", "kept"]);
}

#[test]
fn should_watch_moved_in_tree_when_directory_is_renamed_into_the_site() {
    // given
    let (base, root) = site();
    let outside = base.path().join("incoming");
    fs::create_dir_all(outside.join("t/u")).expect("create a tree outside");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    fs::rename(outside.join("t"), site.display_path.join("t")).expect("move the tree in");
    pump_until(&mut source, &site, "the moved tree watched", |source, _| {
        watching(source, &["t", "t/u"])
    });
    fs::write(site.display_path.join("t/u/page.html"), b"x").expect("edit inside");

    // then
    pump_until(
        &mut source,
        &site,
        "the edit in the moved tree",
        |_, changes| names_path(changes, "t/u/page.html"),
    );
}

#[test]
fn should_move_watches_when_directory_is_renamed_away_and_back() {
    // given
    let (base, root) = site();
    fs::create_dir_all(root.join("blog/posts")).expect("create blog");
    let away = base.path().join("away");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);
    let blog = site.display_path.join("blog");

    // when: away, then back
    fs::rename(&blog, &away).expect("move blog away");
    pump_until(&mut source, &site, "blog forgotten", |source, _| {
        watching_none(source, &["blog", "blog/posts"])
    });
    fs::write(away.join("posts/elsewhere.html"), b"x").expect("edit while away");
    fs::rename(&away, &blog).expect("move blog back");
    pump_until(&mut source, &site, "blog watched again", |source, _| {
        watching(source, &["blog", "blog/posts"])
    });
    fs::write(blog.join("posts/back.html"), b"x").expect("edit after the return");

    // then
    let mut changes = Vec::new();
    pump_until(
        &mut source,
        &site,
        "the edit after the return",
        |_, seen| {
            changes = seen.to_vec();
            names_path(seen, "blog/posts/back.html")
        },
    );
    assert!(
        !changes.iter().any(
            |change| change.path.components().last().map(Component::as_str)
                == Some("elsewhere.html")
        ),
        "an edit outside the site was delivered: {changes:?}"
    );
}

#[test]
fn should_rename_watches_when_directory_is_renamed_inside_the_site() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("draft/img")).expect("create draft");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    fs::rename(
        site.display_path.join("draft"),
        site.display_path.join("final"),
    )
    .expect("rename draft");
    pump_until(&mut source, &site, "the watches renamed", |source, _| {
        watching(source, &["final", "final/img"]) && watching_none(source, &["draft", "draft/img"])
    });
    fs::write(site.display_path.join("final/img/a.svg"), b"x").expect("edit inside");

    // then
    pump_until(
        &mut source,
        &site,
        "the edit under the new name",
        |_, changes| names_path(changes, "final/img/a.svg"),
    );
}

#[test]
fn should_rewatch_replaced_directory_when_full_reconcile_follows_a_loss() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join("docs")).expect("create docs");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);
    let docs = site.display_path.join("docs");
    fs::remove_dir(&docs).expect("rmdir docs");
    fs::create_dir(&docs).expect("mkdir docs");
    // The changes are delivered but not reconciled, as if they had been lost: only the full
    // walk can notice that `docs` is another directory now.
    settle_with_marker(&mut source, &site);

    // when
    let report = source.reconcile_full(&site.dir);

    // then
    assert!(report.complete, "{report:?}");
    probe_until_delivered(&mut source, &site, "docs");
}

#[test]
fn should_not_watch_ignored_or_linked_directory_when_created() {
    // given
    let (base, root) = site();
    let outside = base.path().join("outside");
    fs::create_dir(&outside).expect("create outside");
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    for dir in [
        "node_modules/pkg",
        ".git/objects",
        "vendor/lib",
        "docs/.cache",
    ] {
        fs::create_dir_all(site.display_path.join(dir)).expect("create ignored");
    }
    std::os::unix::fs::symlink(&outside, site.display_path.join("linked")).expect("link");
    fs::create_dir(site.display_path.join("marker")).expect("create the marker");

    // then
    pump_until(&mut source, &site, "the marker watched", |source, _| {
        watching(source, &["marker", "docs"])
    });
    assert_eq!(names(source.watched()), ["", "docs", "marker"]);
    fs::write(outside.join("through-link.html"), b"x").expect("write through the link");
    fs::write(site.display_path.join("marker/after.html"), b"x").expect("write the marker");
    let mut seen = Vec::new();
    pump_until(&mut source, &site, "the marker edit", |_, changes| {
        seen = changes.to_vec();
        names_path(changes, "marker/after.html")
    });
    assert!(
        !names_path(&seen, "linked/through-link.html"),
        "the link was followed: {seen:?}"
    );
}

#[test]
fn should_report_relative_directory_once_when_new_directory_cannot_be_walked() {
    // given
    let (base, root) = site();
    let staging = base.path().join("staging");
    fs::create_dir_all(staging.join("new/locked")).expect("create the tree");
    fs::set_permissions(
        staging.join("new/locked"),
        fs::Permissions::from_mode(0o000),
    )
    .expect("chmod 000");
    // Only uid 0 holding its DAC capabilities may list a mode-000 directory; there the denial
    // cannot be provoked, and the test asserts the directory is then watched instead.
    let os_allows = fs::read_dir(staging.join("new/locked")).is_ok();
    let site = open_site_root(&root).expect("open root");
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);

    // when
    fs::rename(staging.join("new"), site.display_path.join("new")).expect("move in");
    let failures = pump_until(&mut source, &site, "new watched", |source, _| {
        watching(source, &["new"])
    });
    let again = source.reconcile_changed(&site.dir);
    fs::set_permissions(
        site.display_path.join("new/locked"),
        fs::Permissions::from_mode(0o755),
    )
    .expect("restore mode");
    let recovered = source.reconcile_changed(&site.dir);

    // then
    if os_allows {
        assert!(failures.is_empty(), "{failures:?}");
        assert!(again.complete && recovered.complete);
    } else {
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(
            failures[0].to_string(),
            "cannot watch directory `new/locked`: permission denied"
        );
        assert!(!again.complete, "the failure is outstanding");
        assert!(again.errors.is_empty(), "reported once: {again:?}");
        assert!(recovered.complete, "retried and watched: {recovered:?}");
    }
    assert!(watching(&source, &["new", "new/locked"]));
}

#[test]
fn should_rescan_nearest_existing_parent_when_nested_directories_are_removed() {
    // given
    let (_base, root) = site();
    fs::create_dir_all(root.join("a/b/c")).expect("create a/b/c");
    let site = open_site_root(&root).expect("open root");
    let (mut watcher, _errors, _notifications) = start(&site);
    // Only the deepest removal is reported; `a/b` going and `a/x` arriving are not.
    fs::remove_dir_all(site.display_path.join("a/b")).expect("rm -r a/b");
    fs::create_dir_all(site.display_path.join("a/x/y")).expect("mkdir -p a/x/y");
    let hints = std::collections::BTreeSet::from([ReconcileHint::Entry(rel("a/b/c"))]);

    // when
    let report = watcher.reconcile(&site.dir, &hints);

    // then: `a` is the nearest directory left, and its listing finds the rest
    assert!(report.complete && report.errors.is_empty(), "{report:?}");
    assert_eq!(
        names(watcher.watched().iter().cloned()),
        ["", "a", "a/x", "a/x/y"]
    );
}

#[test]
fn should_keep_watch_set_when_file_is_created_and_deleted() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join("docs")).expect("create docs");
    let site = open_site_root(&root).expect("open root");
    let (mut watcher, _errors, _notifications) = start(&site);
    let page = site.display_path.join("docs/page.html");
    fs::write(&page, b"x").expect("create a file");
    let created = std::collections::BTreeSet::from([ReconcileHint::Entry(rel("docs/page.html"))]);

    // when
    let after_create = watcher.reconcile(&site.dir, &created);
    fs::remove_file(&page).expect("delete the file");
    let after_delete = watcher.reconcile(&site.dir, &created);

    // then
    assert!(after_create.complete && after_delete.complete);
    assert_eq!(names(watcher.watched().iter().cloned()), ["", "docs"]);
}

#[test]
fn should_watch_directory_created_between_listing_and_watch_when_registering() {
    // given: `a` is listed, then `a/late/deeper` appears before `a` is watched
    let (_base, root) = site();
    fs::create_dir(root.join("a")).expect("create a");
    let site = open_site_root(&root).expect("open root");
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    let (sender, _notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");
    let late = site.display_path.join("a/late/deeper");
    watcher.before_watch = Some(Box::new(move |dir: &RelPath| {
        if *dir == rel("a") {
            fs::create_dir_all(&late).expect("create in the window");
        }
    }));

    // when
    let errors = watcher.register(&site.dir, traversal);

    // then
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let now = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse again");
    assert_eq!(
        names(watcher.watched().iter().cloned()),
        names(now.dirs.clone())
    );
    assert_eq!(names(now.dirs), ["", "a", "a/late", "a/late/deeper"]);
}

#[test]
fn should_hand_the_rest_to_a_full_reconcile_when_relisting_passes_the_cap() {
    // given: `a` is listed, then five children appear in it before it is watched, and the
    // second listings of one call may queue two of them
    let (_base, root) = site();
    fs::create_dir(root.join("a")).expect("create a");
    let site = open_site_root(&root).expect("open root");
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    let (sender, _notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");
    watcher.relist_cap = 2;
    let a = site.display_path.join("a");
    watcher.before_watch = Some(Box::new(move |dir: &RelPath| {
        if *dir == rel("a") {
            for n in 0..5 {
                fs::create_dir_all(a.join(format!("n{n}"))).expect("create in the window");
            }
        }
    }));

    // when
    let errors = watcher.register(&site.dir, traversal);
    let capped = names(watcher.watched().iter().cloned());
    let needed = core::mem::take(&mut watcher.rescan_needed);
    let full = watcher.reconcile(
        &site.dir,
        &std::collections::BTreeSet::from([ReconcileHint::Full]),
    );

    // then: the call stopped at the cap and said so; the full reconcile found the rest
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(capped, ["", "a", "a/n0", "a/n1"]);
    assert!(needed, "a rescan is requested past the cap");
    assert!(full.complete, "{full:?}");
    assert!(
        !watcher.rescan_needed,
        "an already watched tree never reaches the cap"
    );
    let now = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse again");
    assert_eq!(names(watcher.watched().iter().cloned()), names(now.dirs));
}

#[test]
fn should_report_overflow_when_a_reconcile_passes_the_relisting_cap() {
    // given: a running source whose second listings may queue one directory per call, and a
    // new directory that gains four children between its listing and its watch
    let (_base, root) = site();
    let site = open_site_root(&root).expect("open root");
    let (mut source, report) = NotifyChangeSource::start(&site.dir, &site.display_path);
    assert!(report.is_complete(), "{report:?}");
    let new = site.display_path.join("new");
    let watcher = source.watcher_mut().expect("a watcher");
    watcher.relist_cap = 1;
    let children = new.clone();
    watcher.before_watch = Some(Box::new(move |dir: &RelPath| {
        if *dir == rel("new") {
            for n in 0..4 {
                fs::create_dir_all(children.join(format!("c{n}"))).expect("create in the window");
            }
        }
    }));

    // when
    fs::create_dir(&new).expect("create new");

    // then: the capped reconcile is followed by a loss, whose full reconcile watches the rest
    let mut overflowed = false;
    let mut last = Vec::new();
    wait_until(DEADLINE, "every new directory watched", || {
        while let Some(batch) = poll_next(&mut source) {
            match batch {
                ChangeBatch::Events(_) => {
                    let _ = source.reconcile_changed(&site.dir);
                }
                ChangeBatch::Overflow => {
                    overflowed = true;
                    let _ = source.reconcile_full(&site.dir);
                }
                other => panic!("unexpected batch: {other:?}"),
            }
        }
        last = names(source.watched());
        last == ["", "new", "new/c0", "new/c1", "new/c2", "new/c3"]
    })
    .unwrap_or_else(|_| panic!("watched: {last:?}"));
    assert!(overflowed, "the rest was handed to a full reconcile");
}

/// Drains every batch available now, reconciling as the pipeline does: a loss walks
/// everything, a batch of changes what it touched.
fn pump_reconciling(source: &mut NotifyChangeSource, site: &SiteRoot) {
    while let Some(batch) = poll_next(source) {
        match batch {
            ChangeBatch::Events(_) => {
                let _ = source.reconcile_changed(&site.dir);
            }
            ChangeBatch::Overflow => {
                let _ = source.reconcile_full(&site.dir);
            }
            other => panic!("unexpected batch: {other:?}"),
        }
    }
}

#[test]
fn should_watch_every_eligible_directory_when_nested_directories_burst_during_start_and_reconcile()
{
    // given: a burst of nested mkdirs racing the start-up walk and the reconciles after it
    let (_base, root) = site();
    let site = open_site_root(&root).expect("open root");
    let display = site.display_path.clone();
    let burst = std::thread::spawn(move || {
        for tree in 0..40 {
            let mut at = display.join(format!("t{tree}"));
            for level in 0..6 {
                at = at.join(format!("d{level}"));
                fs::create_dir_all(&at).expect("mkdir -p");
                fs::create_dir(at.with_extension("side")).expect("mkdir beside");
            }
        }
    });

    // when
    let (mut source, _report) = NotifyChangeSource::start(&site.dir, &site.display_path);
    burst.join().expect("burst thread");

    // then: every directory is eventually watched; a missed one is never notified
    let mut last = Vec::new();
    wait_until(
        DEADLINE,
        "the watch set equal to the eligible directories",
        || {
            pump_reconciling(&mut source, &site);
            let eligible = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
            last = names(eligible.dirs);
            names(source.watched()) == last
        },
    )
    .unwrap_or_else(|_| {
        let watched = names(source.watched());
        let missing: Vec<_> = last.iter().filter(|dir| !watched.contains(dir)).collect();
        panic!("never watched: {missing:?}")
    });
}

#[test]
fn should_keep_watching_link_target_when_link_is_withdrawn() {
    // given: `x` is listed as a directory, then replaced by a link to the watched `b`
    let (_base, root) = site();
    fs::create_dir(root.join("b")).expect("create b");
    fs::create_dir(root.join("x")).expect("create x");
    let site = open_site_root(&root).expect("open root");
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    let x = site.display_path.join("x");
    fs::remove_dir(&x).expect("remove x");
    std::os::unix::fs::symlink("b", &x).expect("swap in a link to b");
    let (sender, notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");

    // when: watching `x` follows the link to `b`'s watch, and withdrawing it removes that
    let errors = watcher.register(&site.dir, traversal);
    let target = site.display_path.join("b/page.html");
    fs::write(&target, b"x").expect("edit inside b");

    // then
    assert_eq!(names(errors.iter().map(|e| e.dir.clone())), ["x"]);
    assert_eq!(names(watcher.watched().iter().cloned()), ["", "b"]);
    wait_for_path(&notifications, &target, &mut Vec::new());
}

#[test]
fn should_retry_at_most_the_budget_when_many_directories_failed() {
    // given: three budgets of failed directories (gone by now, so each retry settles one)
    let (_base, root) = site();
    let site = open_site_root(&root).expect("open root");
    let (mut watcher, _errors, _notifications) = start(&site);
    for n in 0..RETRY_BUDGET * 3 {
        watcher
            .failed
            .insert(rel(&format!("gone-{n:03}")), WatchFault::PermissionDenied);
    }
    let none = std::collections::BTreeSet::new();

    // when
    let left: Vec<usize> = (0..3)
        .map(|_| {
            watcher.reconcile(&site.dir, &none);
            watcher.failed.len()
        })
        .collect();

    // then: one budget per changes-only reconcile, round robin, until none is left
    assert_eq!(left, [RETRY_BUDGET * 2, RETRY_BUDGET, 0]);
}

#[test]
fn should_retry_nothing_until_a_watch_is_freed_when_watch_limit_is_reached() {
    // given: the watch limit was hit, leaving directories outstanding
    let (_base, root) = site();
    fs::create_dir(root.join("docs")).expect("create docs");
    let site = open_site_root(&root).expect("open root");
    let (mut watcher, _errors, _notifications) = start(&site);
    for n in 0..4 {
        watcher
            .failed
            .insert(rel(&format!("gone-{n}")), WatchFault::WatchLimitReached);
    }
    watcher.limit_reached = true;
    let none = std::collections::BTreeSet::new();

    // when
    let blocked = watcher.reconcile(&site.dir, &none);
    let still_blocked = watcher.failed.len();
    watcher.unwatch(std::collections::BTreeSet::from([rel("docs")]));
    let freed = watcher.reconcile(&site.dir, &none);

    // then
    assert!(!blocked.complete);
    assert_eq!(still_blocked, 4, "retried while the limit held");
    assert!(freed.complete, "retried once a watch was freed: {freed:?}");
    assert!(watcher.failed.is_empty());
}

#[test]
fn should_probe_the_watch_limit_again_when_reconciling_fully() {
    // given
    let (_base, root) = site();
    fs::create_dir(root.join("docs")).expect("create docs");
    let site = open_site_root(&root).expect("open root");
    let (sender, _notifications) = channel();
    let mut watcher = NotifyWatcher::new(&site.display_path, sender).expect("create watcher");
    watcher.limit_reached = true;
    let traversal = eligible_dirs(&site.dir, &RelPath::root()).expect("traverse");
    let refused = watcher.register(&site.dir, traversal);

    // when
    let full = std::collections::BTreeSet::from([ReconcileHint::Full]);
    let report = watcher.reconcile(&site.dir, &full);

    // then
    assert!(
        refused
            .iter()
            .all(|e| e.fault == WatchFault::WatchLimitReached),
        "{refused:?}"
    );
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(report.complete, "{report:?}");
    assert_eq!(names(watcher.watched().iter().cloned()), ["", "docs"]);
}
