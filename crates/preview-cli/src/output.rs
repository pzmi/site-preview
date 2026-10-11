//! Human start-up output (plan.md section 3, "Human startup output").

use std::io::Write;

use preview_composition::{Ready, StartupReporter};

/// Writes the human start-up block to a stream and flushes it, so a reader of a pipe sees the
/// URL as soon as the server is ready.
pub struct HumanReport<W: Write> {
    out: W,
}

impl<W: Write> HumanReport<W> {
    /// Reports to `out`.
    pub const fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> StartupReporter for HumanReport<W> {
    /// Pinned by `should_render_startup_block_when_ready`.
    fn ready(&mut self, ready: &Ready<'_>) {
        let block = format!(
            "Site Preview\nServing:  {}\nLocal:    {}\nWatching: not yet available\nReady.\n",
            ready.root.display(),
            ready.url
        );
        // A closed stdout has nowhere left to report to; the server keeps serving.
        let _ = self.out.write_all(block.as_bytes());
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn should_render_startup_block_when_ready() {
        // given
        let mut report = HumanReport::new(Vec::new());
        let ready = Ready {
            root: Path::new("/srv/site"),
            url: "http://127.0.0.1:49152/",
        };

        // when
        report.ready(&ready);

        // then
        assert_eq!(
            String::from_utf8(report.out).expect("utf-8"),
            "Site Preview\n\
             Serving:  /srv/site\n\
             Local:    http://127.0.0.1:49152/\n\
             Watching: not yet available\n\
             Ready.\n"
        );
    }
}
