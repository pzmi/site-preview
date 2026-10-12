//! The `site-preview` binary: parses the command line, delegates to composition and maps exit codes.

mod args;
mod exit;
mod output;

use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use preview_composition::{BindScope, ServeConfig, StartupError, StopSignal, Stopped};

use crate::args::{Cli, Command, ServeArgs};
use crate::exit::Outcome;
use crate::output::{HumanReport, StderrWarnings};

fn main() -> ExitCode {
    run().into()
}

fn run() -> Outcome {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let outcome = if error.use_stderr() {
                Outcome::Usage
            } else {
                Outcome::Success
            };
            // A failed write to a closed stdout or stderr has nowhere left to be reported.
            let _ = error.print();
            return outcome;
        }
    };
    match cli.command {
        Command::Serve(args) => serve(&config_from(args)),
    }
}

fn config_from(args: ServeArgs) -> ServeConfig {
    let mut config = ServeConfig::new(args.directory);
    config.host = args.host;
    config.port = args.port;
    config.allow_remote = args.allow_remote;
    config.debounce = args.debounce;
    config.json = args.json;
    config.open = args.open;
    config
}

/// Refuses a flag that does nothing yet, and a remote bind without `--allow-remote`, as usage
/// errors (exit 2), then runs the server; the remote-binding warning is printed once the
/// listener is bound. A second stop signal during the drain exits 128 plus its number.
fn serve(config: &ServeConfig) -> Outcome {
    if let Some(flag) = unimplemented_flag(config) {
        eprintln!(
            "site-preview: error: {flag} is not implemented yet; run without it (the human \
             output on stdout carries the URL on its `Local:` line)"
        );
        return Outcome::Usage;
    }
    let report = HumanReport::new(io::stdout(), io::stderr());
    let mut report = match config.bind_scope() {
        Err(refusal) => {
            eprintln!("site-preview: error: {refusal}");
            return Outcome::Usage;
        }
        Ok(BindScope::Remote) => report.warn_remote(config.host),
        Ok(BindScope::Loopback) => report,
    };
    match preview_composition::run(config, &mut report, Arc::new(StderrWarnings)) {
        Ok(Stopped::Drained | Stopped::DrainTimedOut) => Outcome::Success,
        Ok(Stopped::Forced(StopSignal::Interrupt)) => Outcome::Interrupted,
        Ok(Stopped::Forced(StopSignal::Terminate)) => Outcome::Terminated,
        Err(error) => {
            eprintln!("site-preview: error: {error}");
            if matches!(error, StartupError::RemoteBind(_)) {
                Outcome::Usage
            } else {
                Outcome::Failure
            }
        }
    }
}

/// `--json` (CLI-06) and `--open` (CLI-07) are parsed but not wired yet. Accepting them would
/// promise what the run does not do: an agent asking for JSON would read human text, and
/// `--open` would silently open nothing. Pinned by
/// `should_exit_2_when_unimplemented_flag_given`.
const fn unimplemented_flag(config: &ServeConfig) -> Option<&'static str> {
    if config.json {
        Some("--json")
    } else if config.open {
        Some("--open")
    } else {
        None
    }
}
