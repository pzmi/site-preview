//! Opening the served URL in the user's browser (`--open`, plan.md section 3, ADR 0028).
//!
//! The opener is a port so tests record the URL instead of starting a browser. The system
//! opener runs the platform's own launcher (`open` on macOS, `xdg-open` elsewhere) directly,
//! with the URL as one `argv` entry and no shell, so no character of the URL is interpreted
//! (plan.md section 5). A launch that fails is a warning, never a reason to stop serving.

use std::io;
use std::process::{Command, Stdio};
use std::thread;

use preview_adapter_http::WarningSink;
use preview_domain::warning_code;

/// Opens a URL in the user's browser.
pub trait UrlOpener: Send + Sync {
    /// Starts the browser for `url` without waiting for it to show the page.
    ///
    /// # Errors
    ///
    /// The launcher could not be started.
    fn open(&self, url: &str) -> io::Result<()>;
}

/// Runs the platform's launcher program with the URL as its only argument.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemOpener;

impl SystemOpener {
    /// The launcher program of this platform.
    #[cfg(target_os = "macos")]
    const PROGRAM: &'static str = "open";
    /// The launcher program of this platform.
    #[cfg(not(target_os = "macos"))]
    const PROGRAM: &'static str = "xdg-open";
}

/// The command that opens `url` with `program`: the URL is a single argument, never parsed
/// by a shell. Pinned by `should_pass_url_as_single_argument_when_launching`.
fn command(program: &str, url: &str) -> Command {
    let mut command = Command::new(program);
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Spawns `command` and reaps it on a detached thread, so a launcher that lingers neither
/// blocks the caller nor stays a zombie. Only the spawn is checked: a launcher's own exit
/// status says nothing reliable about whether a browser showed the page.
fn spawn_detached(mut command: Command) -> io::Result<()> {
    let mut child = command.spawn()?;
    // Dropping the handle on a failed thread spawn leaves the child to init, which is fine.
    let _ = thread::Builder::new()
        .name("url-opener".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

impl UrlOpener for SystemOpener {
    fn open(&self, url: &str) -> io::Result<()> {
        spawn_detached(command(Self::PROGRAM, url))
    }
}

/// Opens `url` through `opener`; a failure is the `open-failed` warning and nothing more.
/// Pinned by `should_warn_and_keep_serving_when_opener_fails`.
pub(crate) fn launch(opener: &dyn UrlOpener, url: &str, warnings: &dyn WarningSink) {
    if let Err(error) = opener.open(url) {
        warnings.warn(
            warning_code::OPEN_FAILED,
            &format!("could not open a browser ({error}); open {url} yourself"),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::Mutex;

    use preview_domain::warning_code::WarningCode;

    use super::*;

    #[derive(Default)]
    struct Warnings(Mutex<Vec<(String, String)>>);

    impl WarningSink for Warnings {
        fn warn(&self, code: WarningCode, message: &str) {
            self.0
                .lock()
                .expect("lock")
                .push((code.to_string(), message.to_owned()));
        }
    }

    struct Failing;

    impl UrlOpener for Failing {
        fn open(&self, _url: &str) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::NotFound, "no launcher"))
        }
    }

    #[test]
    fn should_pass_url_as_single_argument_when_launching() {
        // given
        let url = "http://127.0.0.1:1/?a=1&b=$(touch x);`id`|cat *";

        // when
        let command = command("xdg-open", url);

        // then
        assert_eq!(command.get_program(), OsStr::new("xdg-open"));
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(args, [OsStr::new(url)]);
    }

    #[test]
    fn should_warn_with_url_when_launcher_cannot_start() {
        // given
        let warnings = Warnings::default();

        // when
        launch(&Failing, "http://127.0.0.1:9/", &warnings);

        // then
        let recorded = warnings.0.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "open-failed");
        assert!(recorded[0].1.contains("no launcher"), "{}", recorded[0].1);
        assert!(
            recorded[0].1.contains("http://127.0.0.1:9/"),
            "{}",
            recorded[0].1
        );
    }

    #[test]
    fn should_fail_to_spawn_when_program_does_not_exist() {
        // given
        let missing = command("site-preview-no-such-launcher", "http://127.0.0.1:1/");

        // when
        let result = spawn_detached(missing);

        // then
        assert!(result.is_err());
    }
}
