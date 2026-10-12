//! Black-box suite for the watcher pipeline on Linux (plan.md sections 8 and 12): the real
//! binary watches a copy of the fixture site and an open event stream reports what changed.
//!
//! No case asserts a latency: every wait is bounded by the harness deadline, and "nothing was
//! published" is a bounded silence of twice the default debounce window followed by a positive
//! control edit whose revision proves that nothing was published before it.
//!
//! Not forced through the binary, because there is no production test knob:
//!
//! - Channel overflow is pinned at composition level by
//!   `should_publish_after_overflow_when_channel_overflowed` (`watch_pipeline.rs`) and at
//!   adapter level by FS-08's bridge tests.
//! - An unavailable watcher cannot be provoked unprivileged and without touching the shared
//!   machine (a tiny inotify limit is a system-wide setting; the suite may run as root). The
//!   composition-level failing source stands in for it: `should_warn_and_keep_serving_when_watcher_is_unavailable`
//!   (`server.rs`) pins the `watch-unavailable` warning, and `run` treats that state as
//!   degraded mode, never a start-up failure.
//!
//! macOS runs come with OPS-03. The one-file smoke tests are in `watch_reload.rs`.

pub mod support;

/// In the `os-watcher` nextest group: real watches share the system's inotify budget.
mod os_watcher {
    use std::fs;
    use std::time::Duration;

    use super::support::{EventStream, PreviewProcess, Reload, Site};

    /// Twice the default 150 ms debounce window: how long silence must last to count as
    /// "no revision".
    const QUIET: Duration = Duration::from_millis(300);

    /// A started server on a fresh copy of the site, with its stream past the baseline.
    struct Watched {
        site: Site,
        _server: PreviewProcess,
        stream: EventStream,
    }

    fn watch() -> Watched {
        let site = Site::copy();
        let server = PreviewProcess::serve(&site);
        let mut stream = server.events(&[]).expect("the stream opens");
        stream.baseline();
        Watched {
            site,
            _server: server,
            stream,
        }
    }

    impl Watched {
        fn write(&self, relative: &str, text: &str) {
            fs::write(self.site.path().join(relative), text).expect("a file is written");
        }

        /// Asserts that no reload arrives for [`QUIET`].
        fn assert_quiet(&mut self, why: &str) {
            let frame = self.stream.next_event_within(QUIET);
            assert_eq!(frame, None, "{why}");
        }

        /// Edits `file`, then waits for the reload that lists it and returns it.
        fn edit(&mut self, file: &str) -> Reload {
            self.write(file, "/* edited */\n");
            self.stream.reload_listing(&format!("/{file}"))
        }
    }

    fn assert_reload(reload: &Reload, path: &str, kind: &str, full: bool) {
        assert_eq!(reload.kind_of(path), Some(kind), "{reload:?}");
        assert_eq!(reload.full, full, "{reload:?}");
    }

    #[test]
    fn should_reload_hot_when_css_is_edited() {
        // given
        let mut watched = watch();

        // when
        let reload = watched.edit("styles.css");

        // then
        assert_reload(&reload, "/styles.css", "css", false);
    }

    #[test]
    fn should_reload_hot_when_js_is_edited() {
        // given
        let mut watched = watch();

        // when
        let reload = watched.edit("app.js");

        // then
        assert_reload(&reload, "/app.js", "js", false);
    }

    #[test]
    fn should_reload_full_when_html_is_edited() {
        // given
        let mut watched = watch();

        // when
        let reload = watched.edit("guide/index.html");

        // then
        assert_reload(&reload, "/guide/index.html", "html", true);
    }

    #[test]
    fn should_reload_full_when_gpx_is_edited() {
        // given
        let mut watched = watch();

        // when
        let reload = watched.edit("routes.gpx");

        // then
        assert_reload(&reload, "/routes.gpx", "other", true);
    }

    #[test]
    fn should_reload_full_when_json_is_edited() {
        // given
        let mut watched = watch();

        // when
        let reload = watched.edit("data.json");

        // then
        assert_reload(&reload, "/data.json", "other", true);
    }

    #[test]
    fn should_reload_hot_when_asset_is_edited() {
        // given
        let mut watched = watch();

        // when
        let reload = watched.edit("images/sample.svg");

        // then
        assert_reload(&reload, "/images/sample.svg", "asset", false);
    }

    #[test]
    fn should_reload_when_file_is_created_then_full_when_it_is_deleted() {
        // given
        let mut watched = watch();

        // when
        watched.write("fresh.css", "a { color: red }\n");
        let created = watched.stream.reload_listing("/fresh.css");
        fs::remove_file(watched.site.path().join("fresh.css")).expect("the file is removed");
        let removed = watched.stream.reload_listing("/fresh.css");

        // then
        assert_reload(&created, "/fresh.css", "css", false);
        assert_reload(&removed, "/fresh.css", "css", true);
        assert!(
            removed.revision > created.revision,
            "{created:?} {removed:?}"
        );
    }

    #[test]
    fn should_reload_once_for_the_page_when_saved_atomically() {
        // given: the editor writes a temporary file, then renames it over the page
        let mut watched = watch();
        let root = watched.site.path().to_path_buf();

        // when
        fs::write(root.join("index.html.tmp"), "<!doctype html><p>saved</p>\n")
            .expect("the temporary file is written");
        fs::rename(root.join("index.html.tmp"), root.join("index.html")).expect("the rename");
        let reload = watched.stream.reload_listing("/index.html");

        // then
        assert_reload(&reload, "/index.html", "html", true);
        assert!(
            !reload.paths().iter().any(|path| path.contains(".tmp")),
            "{reload:?}"
        );
        assert_eq!(reload.revision, 1, "{reload:?}");
        watched.assert_quiet("the temporary name is not a second revision");
    }

    #[test]
    fn should_watch_new_directory_when_created_then_file_edited() {
        // given
        let mut watched = watch();

        // when
        fs::create_dir(watched.site.path().join("posts")).expect("mkdir");
        let created = watched.stream.next_reload();
        watched.write("posts/one.css", "a { top: 0 }\n");
        let edited = watched.stream.reload_listing("/posts/one.css");

        // then
        assert!(created.full, "{created:?}");
        assert_reload(&edited, "/posts/one.css", "css", false);
    }

    /// The three reloads of a directory that goes away, comes back and is edited.
    struct Recreated {
        gone: Reload,
        back: Reload,
        edited: Reload,
    }

    /// Makes `guide/` go away with `how`, recreates it, and edits a file inside. Waiting for
    /// each reload before the next step is what orders the steps: the pipeline registers a new
    /// directory before it publishes the revision that announces it.
    fn recreate_after(how: impl FnOnce(&Site)) -> Recreated {
        let mut watched = watch();
        how(&watched.site);
        let gone = watched.stream.next_reload();
        fs::create_dir(watched.site.path().join("guide")).expect("the directory is recreated");
        let back = watched.stream.next_reload();
        watched.write("guide/extra.css", "p { margin: 0 }\n");
        let edited = watched.stream.reload_listing("/guide/extra.css");
        Recreated { gone, back, edited }
    }

    fn assert_recreated(recreated: &Recreated) {
        let Recreated { gone, back, edited } = recreated;
        assert!(gone.full, "{gone:?}");
        // The directory's own path, or the page inside it that went first when it was deleted.
        assert!(
            gone.paths().iter().any(|path| path.starts_with("/guide")),
            "{gone:?}"
        );
        assert!(back.full, "{back:?}");
        assert_reload(edited, "/guide/extra.css", "css", false);
    }

    #[test]
    fn should_watch_directory_again_when_deleted_then_recreated() {
        // given
        let delete = |site: &Site| {
            fs::remove_dir_all(site.path().join("guide")).expect("the directory is deleted");
        };

        // when
        let recreated = recreate_after(delete);

        // then
        assert_recreated(&recreated);
    }

    #[test]
    fn should_watch_directory_again_when_renamed_away_then_recreated() {
        // given
        let rename = |site: &Site| {
            fs::rename(site.path().join("guide"), site.path().join("guide-old"))
                .expect("the directory is renamed");
        };

        // when
        let recreated = recreate_after(rename);

        // then
        assert_recreated(&recreated);
    }

    #[test]
    fn should_publish_nothing_when_renamed_out_directory_is_edited_outside_the_root() {
        // given
        let mut watched = watch();
        let moved = watched.site.outside().join("guide");
        fs::rename(watched.site.path().join("guide"), &moved).expect("moved out of the root");
        let gone = watched.stream.next_reload();

        // when
        fs::write(moved.join("index.html"), "<p>edited outside</p>\n").expect("an outside edit");
        let silent = watched.stream.next_event_within(QUIET);
        let control = watched.edit("styles.css");

        // then
        assert!(gone.full, "{gone:?}");
        assert_eq!(silent, None, "an edit outside the root published");
        assert_eq!(control.revision, gone.revision + 1, "{control:?}");
        assert_eq!(control.paths(), ["/styles.css"], "{control:?}");
    }

    #[test]
    fn should_publish_one_revision_when_twenty_edits_burst() {
        // given
        let mut watched = watch();

        // when
        for round in 0..20 {
            watched.write("styles.css", &format!("a {{ order: {round} }}\n"));
        }
        let burst = watched.stream.next_reload();

        // then
        assert_eq!(burst.revision, 1, "{burst:?}");
        assert_reload(&burst, "/styles.css", "css", false);
        watched.assert_quiet("the burst settled into one revision");
        let control = watched.edit("app.js");
        assert_eq!(control.revision, 2, "{control:?}");
    }

    #[test]
    fn should_publish_nothing_when_git_editor_temp_and_node_modules_change() {
        // given
        let mut watched = watch();
        let root = watched.site.path().to_path_buf();

        // when
        fs::write(root.join(".git/config"), "changed\n").expect("a .git edit");
        fs::create_dir_all(root.join(".git/objects")).expect("a .git directory");
        fs::write(root.join(".git/objects/pack"), "x\n").expect("a file in .git");
        for name in [".index.html.swp", "index.html~", "draft.tmp", "#notes#"] {
            fs::write(root.join(name), "scratch\n").expect("an editor temp file");
        }
        fs::write(root.join("node_modules/x.js"), "changed\n").expect("a node_modules edit");
        fs::create_dir_all(root.join("node_modules/pkg")).expect("a node_modules directory");
        fs::write(root.join("node_modules/pkg/index.js"), "x\n").expect("a nested file");
        let silent = watched.stream.next_event_within(QUIET);
        let control = watched.edit("index.html");

        // then
        assert_eq!(silent, None, "an excluded change published");
        assert_eq!(control.revision, 1, "{control:?}");
        assert_eq!(control.paths(), ["/index.html"], "{control:?}");
    }
}
