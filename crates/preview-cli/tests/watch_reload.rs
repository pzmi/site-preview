//! Real-filesystem smoke test of the watcher pipeline: an edit reaches an open event stream
//! as one `reload` (plan.md section 8.3). The whole watcher suite, `.git` and editor temp
//! files included, is E2E-03's `watcher.rs`; the debounce and overflow are pinned at
//! composition level with paused time (`watch_pipeline.rs`).

pub mod support;

/// In the `os-watcher` nextest group: real watches share the system's inotify budget.
mod os_watcher {
    use std::fs;
    use std::time::Instant;

    use super::support::{DEADLINE, EventStream, Frame, PreviewProcess, Site};

    /// The next frame that is not a keepalive comment; each read is bounded by [`DEADLINE`],
    /// and so is the whole wait.
    fn next_event(stream: &mut EventStream) -> Frame {
        let start = Instant::now();
        loop {
            let frame = stream.next_frame();
            if !frame.comment {
                return frame;
            }
            assert!(start.elapsed() < DEADLINE, "no event within {DEADLINE:?}");
        }
    }

    #[test]
    fn should_send_reload_when_index_html_is_edited() {
        // given
        let site = Site::copy();
        let server = PreviewProcess::serve(&site);
        let mut stream = server.events(&[]).expect("the stream opens");
        stream.baseline();

        // when
        fs::write(
            site.path().join("index.html"),
            "<!doctype html><title>edited</title><body>edited</body>\n",
        )
        .expect("the page is edited");

        // then
        let frame = next_event(&mut stream);
        assert_eq!(frame.event.as_deref(), Some("reload"), "{frame:?}");
        let data = frame.data.join("\n");
        assert!(data.contains(r#""revision":1"#), "{data}");
        assert!(data.contains(r#""path":"/index.html""#), "{data}");
        assert!(data.contains(r#""full":true"#), "{data}");
    }

    #[test]
    fn should_watch_new_nested_directory_when_created_then_file_edited() {
        // given
        let site = Site::copy();
        let server = PreviewProcess::serve(&site);
        let mut stream = server.events(&[]).expect("the stream opens");
        stream.baseline();

        // when: `mkdir -p`, so the inner directory exists before the outer one is watched
        fs::create_dir_all(site.path().join("new/nested")).expect("mkdir -p");
        let created = next_event(&mut stream);
        fs::write(site.path().join("new/nested/a.css"), "a { color: red }\n")
            .expect("a file is written inside");

        // then
        assert_eq!(created.event.as_deref(), Some("reload"), "{created:?}");
        let edited = next_event(&mut stream);
        assert_eq!(edited.event.as_deref(), Some("reload"), "{edited:?}");
        let data = edited.data.join("\n");
        assert!(data.contains(r#""path":"/new/nested/a.css""#), "{data}");
    }
}
