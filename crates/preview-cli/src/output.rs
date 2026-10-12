//! Human start-up output (plan.md section 3, "Human startup output") and warnings on stderr
//! (ADR 0020).

use std::io::{self, Write};
use std::net::IpAddr;
use std::time::Duration;

use preview_composition::{Ready, StartupReporter, WarningSink};

/// One warning line in the ADR 0020 format: `site-preview: warning[<code>]: <message>`.
/// Pinned by `should_format_warning_line_when_rendered`.
pub fn warning_line(code: &str, message: &str) -> String {
    format!("site-preview: warning[{code}]: {message}")
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
            let message = format!(
                "listening on {host} with no authentication: anyone who can reach this \
                 address can read the site; do not use it on an untrusted network"
            );
            let warning = format!("{}\n", warning_line("remote-binding", &message));
            let _ = self.err.write_all(warning.as_bytes());
            let _ = self.err.flush();
        }
        let block = format!(
            "Site Preview\nServing:  {}\nLocal:    {}\nWatching: not yet available\nReady.\n",
            ready.root.display(),
            ready.url
        );
        let _ = self.out.write_all(block.as_bytes());
        let _ = self.out.flush();
    }

    /// Pinned by `should_tell_how_to_stop_now_when_draining`.
    fn draining(&mut self, bound: Duration) {
        let notice = format!(
            "site-preview: shutting down: waiting up to {bound:?} for open connections; \
             press Ctrl-C again to stop now\n"
        );
        let _ = self.err.write_all(notice.as_bytes());
        let _ = self.err.flush();
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
             Watching: not yet available\n\
             Ready.\n"
        );
        assert!(report.err.is_empty());
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
}
