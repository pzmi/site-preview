//! The `ChangeSource` contract harness against the native watcher over a temporary directory,
//! and its negative control: the same `Overflow`-swallowing wrapper that fails the harness
//! around the scripted fake in preview-testkit fails it here at the same case,
//! `overflow_before_later_changes` (foundation.md section 5.1).
//!
//! These tests observe the real operating system, so they live in the `os_watcher` module and
//! nextest runs them in the `os-watcher` group. The harness waits for native notifications with
//! bounded condition polling: the OS delivers asynchronously and offers no readiness signal,
//! and the deadline only limits how long a failing case hangs.

mod os_watcher {
    use core::num::NonZeroUsize;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use preview_adapter_fs::{MAX_DEPTH, NotifyChangeSource, open_site_root};
    use preview_application::{ChangeBatch, ChangeSource};
    use preview_domain::ChangeEvent;
    use preview_testkit::contracts::{
        CHANGE_SOURCE_CASES, ChangeSourceFactory, WatchTree, change_source_contract,
        check_change_source_contract,
    };
    use tempfile::TempDir;

    /// A temporary site: `<base>/site` is the root the source watches. Dropping it removes the
    /// directory.
    struct TempSite {
        root: PathBuf,
        _base: TempDir,
    }

    fn create_parent(path: &std::path::Path) {
        fs::create_dir_all(path.parent().expect("an entry has a parent")).expect("create parents");
    }

    /// `NotifyChangeSource` over a fresh temporary directory, driven by writing files.
    struct NativeWatch;

    impl ChangeSourceFactory for NativeWatch {
        type Source = NotifyChangeSource;
        type Site = TempSite;

        const DEADLINE: Duration = Duration::from_secs(10);

        fn start(tree: &WatchTree, capacity: NonZeroUsize) -> (NotifyChangeSource, TempSite) {
            let base = tempfile::tempdir().expect("tempdir");
            let root = base.path().join("site");
            fs::create_dir(&root).expect("create root");
            for dir in tree.dirs() {
                fs::create_dir_all(root.join(dir)).expect("create dir");
            }
            for file in tree.files() {
                let path = root.join(file);
                create_parent(&path);
                fs::write(path, b"x").expect("write file");
            }
            for dir in tree.unwatchable_dirs() {
                // one level deeper than the watcher descends: that level cannot be watched
                let mut at = root.join(dir);
                for _ in 0..=MAX_DEPTH {
                    at = at.join("d");
                }
                fs::create_dir_all(at).expect("create a too-deep chain");
            }
            let site = open_site_root(&root).expect("open site root");
            let (source, _report) =
                NotifyChangeSource::start_with_capacity(&site.dir, &site.display_path, capacity);
            let site = TempSite {
                root: site.display_path,
                _base: base,
            };
            (source, site)
        }

        fn write(site: &mut TempSite, path: &str) {
            fs::write(site.root.join(path), b"changed").expect("write file");
        }

        fn stop(_site: &mut TempSite, source: &mut NotifyChangeSource) {
            source.close();
        }
    }

    /// The deliberately broken adapter: passes every batch of the native watcher through
    /// except `Overflow`, which it swallows.
    struct DropsOverflow(NotifyChangeSource);

    impl ChangeSource for DropsOverflow {
        type Event = ChangeEvent;

        async fn next(&mut self) -> ChangeBatch<ChangeEvent> {
            loop {
                match self.0.next().await {
                    ChangeBatch::Overflow => {}
                    other => return other,
                }
            }
        }
    }

    struct BrokenDropsOverflow;

    impl ChangeSourceFactory for BrokenDropsOverflow {
        type Source = DropsOverflow;
        type Site = TempSite;

        const DEADLINE: Duration = NativeWatch::DEADLINE;

        fn start(tree: &WatchTree, capacity: NonZeroUsize) -> (DropsOverflow, TempSite) {
            let (source, site) = NativeWatch::start(tree, capacity);
            (DropsOverflow(source), site)
        }

        fn write(site: &mut TempSite, path: &str) {
            NativeWatch::write(site, path);
        }

        fn stop(site: &mut TempSite, source: &mut DropsOverflow) {
            NativeWatch::stop(site, &mut source.0);
        }
    }

    #[test]
    fn should_pass_contract_when_source_is_native_watcher_over_tempdir() {
        // given
        // NativeWatch starts NotifyChangeSource over a fresh temporary directory per case.

        // when
        let report = check_change_source_contract::<NativeWatch>();

        // then
        assert_eq!(report.cases_run(), CHANGE_SOURCE_CASES);
        report.assert_passed();
        change_source_contract::<NativeWatch>();
    }

    #[test]
    fn should_fail_contract_when_overflow_is_dropped() {
        // given
        // BrokenDropsOverflow wraps the native watcher and swallows every `Overflow`.

        // when
        let report = check_change_source_contract::<BrokenDropsOverflow>();

        // then
        assert_eq!(report.cases_run(), CHANGE_SOURCE_CASES);
        assert_eq!(
            report.failed_cases(),
            ["overflow_before_later_changes"],
            "{report}"
        );
    }
}
