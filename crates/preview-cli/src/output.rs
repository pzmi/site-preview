//! Start-up output as a person reads it (plan.md section 3, "Human startup output") or as an
//! agent does (`--json`: JSONL events, "JSON startup protocol"), and warnings on stderr
//! (ADR 0020).

use std::io::{self, Write};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use preview_composition::{
    DegradedReason, Ready, StartupReporter, WarningSink, WatchStatus, WatcherEvents, WatcherState,
    warning_code,
};

/// One warning line in the ADR 0020 format: `site-preview: warning[<code>]: <message>`.
/// Pinned by `should_format_warning_line_when_rendered`.
pub fn warning_line(code: &str, message: &str) -> String {
    format!("site-preview: warning[{code}]: {message}")
}

/// The text after `Watching:` (plan.md section 3): a count of watched directories, never a
/// claim of live reload the watcher cannot back. Pinned by
/// `should_render_watching_line_when_watcher_is_live_partial_or_unavailable`.
pub fn watching_line(status: WatchStatus<'_>) -> String {
    let count = |dirs: usize| {
        if dirs == 1 {
            "1 eligible directory".to_owned()
        } else {
            format!("{dirs} eligible directories")
        }
    };
    match status {
        WatchStatus::Watching { dirs } => count(dirs),
        WatchStatus::Partial { dirs } => format!(
            "{} (degraded: some directories cannot be watched; see the warnings above)",
            count(dirs)
        ),
        WatchStatus::Unavailable { reason } => {
            format!("unavailable ({reason}); live reload disabled")
        }
    }
}

/// The `remote-binding` warning line for a listener on `host`.
fn remote_warning(host: IpAddr) -> String {
    let message = format!(
        "listening on {host} with no authentication: anyone who can reach this address can \
         read the site; do not use it on an untrusted network"
    );
    format!("{}\n", warning_line(warning_code::REMOTE_BINDING, &message))
}

/// The shutdown notice, the same in both output modes: it goes to stderr, so stdout in
/// `--json` mode stays JSON only.
fn draining_notice(bound: Duration) -> String {
    format!(
        "site-preview: shutting down: waiting up to {bound:?} for open connections; \
         press Ctrl-C again to stop now\n"
    )
}

/// `text` as a JSON string literal, quotes included.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string always serializes")
}

/// The `ready` event line, without its newline (ADR 0020). Pinned by
/// `should_render_ready_event_when_watching_or_unavailable`.
fn ready_event(url: &str, pid: u32, watching: bool) -> String {
    format!(
        r#"{{"event":"ready","url":{},"pid":{pid},"watching":{watching}}}"#,
        json_string(url)
    )
}

/// The `watcher` event line, without its newline; `reason` only when degraded. Pinned by
/// `should_render_watcher_event_when_degraded_and_recovered`.
fn watcher_event(state: WatcherState) -> String {
    match state {
        WatcherState::Watching => r#"{"event":"watcher","state":"watching"}"#.to_owned(),
        WatcherState::Degraded(reason) => format!(
            r#"{{"event":"watcher","state":"degraded","reason":{}}}"#,
            json_string(reason.code())
        ),
    }
}

/// Writes each warning as one [`warning_line`] on stderr.
pub struct StderrWarnings;

impl WarningSink for StderrWarnings {
    fn warn(&self, code: &str, message: &str) {
        // One `writeln!` on the locked handle, so concurrent warnings never interleave; a
        // closed stderr has nowhere left to report to.
        let _ = writeln!(io::stderr().lock(), "{}", warning_line(code, message));
    }
}

/// Writes the human start-up block to `out` and warnings and shutdown notices to `err`,
/// flushing each, so a reader of a pipe sees the URL as soon as the server is ready.
pub struct HumanReport<O: Write, E: Write> {
    out: O,
    err: E,
    remote: Option<IpAddr>,
}

impl<O: Write, E: Write> HumanReport<O, E> {
    /// Reports to `out` and `err`.
    pub const fn new(out: O, err: E) -> Self {
        Self {
            out,
            err,
            remote: None,
        }
    }

    /// Warns that `host` is reachable from other machines, once the listener is bound: a
    /// bind that fails never claims to be listening (`should_not_warn_when_remote_bind_fails`).
    pub fn warn_remote(mut self, host: IpAddr) -> Self {
        self.remote = Some(host);
        self
    }
}

impl<O: Write, E: Write> StartupReporter for HumanReport<O, E> {
    /// Pinned by `should_render_startup_block_when_ready` and
    /// `should_warn_before_block_when_remote_and_ready`.
    fn ready(&mut self, ready: &Ready<'_>) {
        // A closed stdout or stderr has nowhere left to report to; the server keeps serving.
        if let Some(host) = self.remote {
            let _ = self.err.write_all(remote_warning(host).as_bytes());
            let _ = self.err.flush();
        }
        let block = format!(
            "Site Preview\nServing:  {}\nLocal:    {}\nWatching: {}\nReady.\n",
            ready.root.display(),
            ready.url,
            watching_line(ready.watching)
        );
        let _ = self.out.write_all(block.as_bytes());
        let _ = self.out.flush();
    }

    /// Pinned by `should_tell_how_to_stop_now_when_draining`.
    fn draining(&mut self, bound: Duration) {
        let _ = self.err.write_all(draining_notice(bound).as_bytes());
        let _ = self.err.flush();
    }
}

/// What the `--json` events share between the start-up report and the watcher pipeline's
/// thread: the output, and whether `ready` is out yet.
struct Lines<O: Write> {
    out: O,
    ready: bool,
    /// The latest watcher change that arrived before `ready`, written right after it: an
    /// agent reads `ready` first, always. A held `watching` is dropped when `ready` already
    /// says `watching:true`: a recovery before `ready` leaves nothing to report, and a
    /// `watching` event must follow a `degraded` one.
    held: Option<WatcherState>,
}

impl<O: Write> Lines<O> {
    /// One line, flushed, so an agent reading a pipe sees it at once. A closed stdout has
    /// nowhere left to report to; the server keeps serving.
    fn emit(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
        let _ = self.out.flush();
    }
}

fn locked<O: Write>(lines: &Mutex<Lines<O>>) -> MutexGuard<'_, Lines<O>> {
    // A panic elsewhere must not silence the events of a server that is still running.
    lines.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The watcher pipeline's way to write `watcher` events through the same lines as `ready`.
struct JsonWatcher<O: Write>(Arc<Mutex<Lines<O>>>);

impl<O: Write + Send> WatcherEvents for JsonWatcher<O> {
    /// Pinned by `should_write_watcher_event_after_ready_when_change_comes_before_and_after`.
    fn changed(&self, state: WatcherState) {
        let mut lines = locked(&self.0);
        if lines.ready {
            lines.emit(&watcher_event(state));
        } else {
            lines.held = Some(state);
        }
    }
}

/// Writes JSONL events to `out` (stdout carries nothing else in `--json` mode, ADR 0020) and
/// warnings and shutdown notices as text to `err`, flushing each line.
pub struct JsonReport<O: Write, E: Write> {
    lines: Arc<Mutex<Lines<O>>>,
    err: E,
    pid: u32,
    remote: Option<IpAddr>,
}

impl<O: Write, E: Write> JsonReport<O, E> {
    /// Reports to `out` and `err`, naming `pid` as the process to signal.
    pub fn new(out: O, err: E, pid: u32) -> Self {
        Self {
            lines: Arc::new(Mutex::new(Lines {
                out,
                ready: false,
                held: None,
            })),
            err,
            pid,
            remote: None,
        }
    }

    /// As [`HumanReport::warn_remote`]: the warning is text on stderr.
    pub fn warn_remote(mut self, host: IpAddr) -> Self {
        self.remote = Some(host);
        self
    }
}

impl<O: Write + Send + 'static, E: Write> StartupReporter for JsonReport<O, E> {
    /// Pinned by `should_render_ready_event_when_watching_or_unavailable` and
    /// `should_not_write_watching_event_when_watcher_recovered_before_ready`.
    fn ready(&mut self, ready: &Ready<'_>) {
        if let Some(host) = self.remote {
            let _ = self.err.write_all(remote_warning(host).as_bytes());
            let _ = self.err.flush();
        }
        let unavailable = matches!(ready.watching, WatchStatus::Unavailable { .. });
        let mut lines = locked(&self.lines);
        lines.emit(&ready_event(ready.url, self.pid, !unavailable));
        lines.ready = true;
        // A watcher that never started is degraded from the first moment: say so as an event,
        // since `watching:false` alone does not say why or that it is a state. A partial
        // watch set is not (the health machine has not degraded); its warnings name the
        // directories.
        if unavailable {
            lines.emit(&watcher_event(WatcherState::Degraded(
                DegradedReason::RegistrationFailed,
            )));
        } else if let Some(state) = lines.held.take()
            && state != WatcherState::Watching
        {
            lines.emit(&watcher_event(state));
        }
    }

    /// Pinned by `should_tell_how_to_stop_now_when_draining`; stderr, never stdout.
    fn draining(&mut self, bound: Duration) {
        let _ = self.err.write_all(draining_notice(bound).as_bytes());
        let _ = self.err.flush();
    }

    fn watcher_events(&self) -> Arc<dyn WatcherEvents> {
        Arc::new(JsonWatcher(Arc::clone(&self.lines)))
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::path::Path;

    use super::*;

    fn ready() -> Ready<'static> {
        Ready {
            root: Path::new("/srv/site"),
            url: "http://127.0.0.1:49152/",
            watching: WatchStatus::Watching { dirs: 3 },
        }
    }

    #[test]
    fn should_format_warning_line_when_rendered() {
        // given
        let (code, message) = ("html-too-large", "/big.html: HTML is larger than 8 MiB");

        // when
        let line = warning_line(code, message);

        // then
        assert_eq!(
            line,
            "site-preview: warning[html-too-large]: /big.html: HTML is larger than 8 MiB"
        );
    }

    #[test]
    fn should_render_startup_block_when_ready() {
        // given
        let mut report = HumanReport::new(Vec::new(), Vec::new());

        // when
        report.ready(&ready());

        // then
        assert_eq!(
            String::from_utf8(report.out).expect("utf-8"),
            "Site Preview\n\
             Serving:  /srv/site\n\
             Local:    http://127.0.0.1:49152/\n\
             Watching: 3 eligible directories\n\
             Ready.\n"
        );
        assert!(report.err.is_empty());
    }

    #[test]
    fn should_render_watching_line_when_watcher_is_live_partial_or_unavailable() {
        // given
        let statuses = [
            WatchStatus::Watching { dirs: 1 },
            WatchStatus::Watching { dirs: 3 },
            WatchStatus::Partial { dirs: 2 },
            WatchStatus::Unavailable {
                reason: "the native file watcher is unavailable",
            },
        ];

        // when
        let lines = statuses.map(watching_line);

        // then
        assert_eq!(
            lines,
            [
                "1 eligible directory",
                "3 eligible directories",
                "2 eligible directories (degraded: some directories cannot be watched; see the \
                 warnings above)",
                "unavailable (the native file watcher is unavailable); live reload disabled",
            ]
        );
    }

    #[test]
    fn should_warn_before_block_when_remote_and_ready() {
        // given
        let report = HumanReport::new(Vec::new(), Vec::new());
        let mut report = report.warn_remote(IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        // when
        report.ready(&ready());

        // then
        let warning = String::from_utf8(report.err).expect("utf-8");
        assert!(
            warning.starts_with("site-preview: warning[remote-binding]: listening on 0.0.0.0 "),
            "{warning}"
        );
        assert!(warning.contains("no authentication"), "{warning}");
        assert!(!report.out.is_empty());
    }

    #[test]
    fn should_tell_how_to_stop_now_when_draining() {
        // given
        let mut report = HumanReport::new(Vec::new(), Vec::new());

        // when
        report.draining(Duration::from_secs(3));

        // then
        assert_eq!(
            String::from_utf8(report.err).expect("utf-8"),
            "site-preview: shutting down: waiting up to 3s for open connections; press Ctrl-C \
             again to stop now\n"
        );
        assert!(report.out.is_empty());
    }

    /// A writer whose bytes the test reads back after the reporter (or the watcher pipeline's
    /// thread) wrote them.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Shared {
        fn lines(&self) -> Vec<String> {
            let bytes = self.0.lock().expect("unpoisoned").clone();
            String::from_utf8(bytes)
                .expect("utf-8")
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("unpoisoned").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn json_report() -> (JsonReport<Shared, Shared>, Shared, Shared) {
        let (out, err) = (Shared::default(), Shared::default());
        (JsonReport::new(out.clone(), err.clone(), 4242), out, err)
    }

    fn parse(line: &str) -> serde_json::Value {
        serde_json::from_str(line).expect("a JSON line")
    }

    #[test]
    fn should_render_ready_event_when_watching_or_unavailable() {
        // given
        let unavailable = Ready {
            watching: WatchStatus::Unavailable {
                reason: "the native file watcher is unavailable",
            },
            ..ready()
        };
        let partial = Ready {
            watching: WatchStatus::Partial { dirs: 2 },
            ..ready()
        };

        // when
        let rendered = [ready(), partial, unavailable].map(|r| {
            let (mut report, out, _err) = json_report();
            report.ready(&r);
            out.lines()
        });

        // then: one ready line in the documented key order; only an unavailable watcher is
        // followed by a degraded event
        assert_eq!(
            rendered[0],
            [r#"{"event":"ready","url":"http://127.0.0.1:49152/","pid":4242,"watching":true}"#]
        );
        assert_eq!(rendered[1], rendered[0]);
        assert_eq!(
            rendered[2],
            [
                r#"{"event":"ready","url":"http://127.0.0.1:49152/","pid":4242,"watching":false}"#,
                r#"{"event":"watcher","state":"degraded","reason":"registration-failed"}"#,
            ]
        );
    }

    #[test]
    fn should_escape_url_when_it_holds_json_metacharacters() {
        // given
        let odd = Ready {
            url: "http://h/\"\\\n",
            ..ready()
        };
        let (mut report, out, _err) = json_report();

        // when
        report.ready(&odd);

        // then
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(parse(&lines[0])["url"], "http://h/\"\\\n");
    }

    #[test]
    fn should_render_watcher_event_when_degraded_and_recovered() {
        // given
        let states = [
            WatcherState::Degraded(DegradedReason::RuntimeFailure),
            WatcherState::Degraded(DegradedReason::RevisionExhausted),
            WatcherState::Degraded(DegradedReason::RootRemoved),
            WatcherState::Watching,
        ];

        // when
        let lines = states.map(watcher_event);

        // then
        assert_eq!(
            lines,
            [
                r#"{"event":"watcher","state":"degraded","reason":"runtime-failure"}"#,
                r#"{"event":"watcher","state":"degraded","reason":"revision-exhausted"}"#,
                r#"{"event":"watcher","state":"degraded","reason":"root-removed"}"#,
                r#"{"event":"watcher","state":"watching"}"#,
            ]
        );
    }

    #[test]
    fn should_write_watcher_event_after_ready_when_change_comes_before_and_after() {
        // given: the pipeline reports a failure before ready is out, and another change after
        let (mut report, out, _err) = json_report();
        let events = report.watcher_events();
        events.changed(WatcherState::Degraded(DegradedReason::RuntimeFailure));
        assert!(out.lines().is_empty(), "nothing precedes ready");

        // when
        report.ready(&ready());
        events.changed(WatcherState::Watching);

        // then
        let kinds: Vec<_> = out
            .lines()
            .iter()
            .map(|line| {
                let json = parse(line);
                format!("{} {}", json["event"], json["state"])
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "\"ready\" null",
                "\"watcher\" \"degraded\"",
                "\"watcher\" \"watching\""
            ]
        );
    }

    #[test]
    fn should_not_write_watching_event_when_watcher_recovered_before_ready() {
        // given: the watcher degrades and recovers before ready is out
        let (mut report, out, _err) = json_report();
        let events = report.watcher_events();
        events.changed(WatcherState::Degraded(DegradedReason::RuntimeFailure));
        events.changed(WatcherState::Watching);

        // when
        report.ready(&ready());

        // then: ready says watching:true, and nothing follows it without a degraded first
        let lines = out.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(parse(&lines[0])["event"], "ready");
        assert_eq!(parse(&lines[0])["watching"], true);
    }

    #[test]
    fn should_keep_stdout_json_only_when_remote_warning_and_draining() {
        // given
        let (report, out, err) = json_report();
        let mut report = report.warn_remote(IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        // when
        report.ready(&ready());
        report.draining(Duration::from_secs(3));

        // then
        assert!(out.lines().iter().all(|line| parse(line).is_object()));
        let notices = err.lines();
        assert!(
            notices[0].starts_with("site-preview: warning[remote-binding]: "),
            "{notices:?}"
        );
        assert!(
            notices[1].starts_with("site-preview: shutting down: waiting up to 3s"),
            "{notices:?}"
        );
    }
}
