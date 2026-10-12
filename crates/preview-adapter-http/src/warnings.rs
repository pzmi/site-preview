//! Warnings about pages served without live reload (ADR 0020, ADR 0046).
//!
//! The adapter reports through a [`WarningSink`] that composition injects; the CLI renders
//! each warning on stderr as `site-preview: warning[<code>]: <message>`. Nothing here reaches
//! an HTTP response.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

/// Receives warnings from the HTTP adapter.
///
/// `code` is a stable warning code (`html-too-large`, `html-not-utf8`,
/// `html-rewrite-failed`); `message` is one line that starts with the request path as the
/// site sees it (percent-encoded, never an absolute filesystem path or a query string).
/// Called from blocking threads, so an implementation may write to a stream directly.
pub trait WarningSink: Send + Sync {
    /// Reports one warning.
    fn warn(&self, code: &str, message: &str);
}

/// A sink that drops every warning; the default for an [`crate::Injection`] built in tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct SilentWarnings;

impl WarningSink for SilentWarnings {
    fn warn(&self, _code: &str, _message: &str) {}
}

/// How many `(code, path)` pairs are remembered. Past it, a new pair is not reported at all,
/// so a client requesting endless spellings of one page (case variants on a case-insensitive
/// disk) can neither grow the set nor flood stderr.
const MAX_REMEMBERED: usize = 4096;

/// Forwards each `(code, path)` warning to the sink the first time only (plan.md §6 step 3).
///
/// Pinned by `should_serve_unchanged_and_warn_once_when_html_over_limit` in `tests/inject.rs`
/// and `should_stop_reporting_when_memo_is_full` below.
pub(crate) struct OncePerPath {
    sink: Arc<dyn WarningSink>,
    seen: Mutex<HashSet<(&'static str, String)>>,
}

impl OncePerPath {
    pub(crate) fn new(sink: Arc<dyn WarningSink>) -> Self {
        Self {
            sink,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// Reports `message` for `path` under `code` unless that pair was reported before.
    pub(crate) fn warn(&self, code: &'static str, path: &str, message: &str) {
        let first = {
            let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            seen.len() < MAX_REMEMBERED && seen.insert((code, path.to_owned()))
        };
        if first {
            self.sink.warn(code, &format!("{path}: {message}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl WarningSink for Recorder {
        fn warn(&self, code: &str, message: &str) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{code} {message}"));
        }
    }

    impl Recorder {
        fn lines(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    #[test]
    fn should_report_once_per_code_and_path_when_repeated() {
        // given
        let recorder = Arc::new(Recorder::default());
        let once = OncePerPath::new(recorder.clone());

        // when
        once.warn("html-too-large", "/a.html", "big");
        once.warn("html-too-large", "/a.html", "big");
        once.warn("html-not-utf8", "/a.html", "latin1");
        once.warn("html-too-large", "/b.html", "big");

        // then
        assert_eq!(
            recorder.lines(),
            [
                "html-too-large /a.html: big",
                "html-not-utf8 /a.html: latin1",
                "html-too-large /b.html: big",
            ]
        );
    }

    #[test]
    fn should_stop_reporting_when_memo_is_full() {
        // given
        let recorder = Arc::new(Recorder::default());
        let once = OncePerPath::new(recorder.clone());
        for i in 0..MAX_REMEMBERED {
            once.warn("html-too-large", &format!("/{i}.html"), "big");
        }

        // when
        once.warn("html-too-large", "/one-more.html", "big");

        // then
        assert_eq!(recorder.lines().len(), MAX_REMEMBERED);
    }
}
