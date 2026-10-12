//! The `site-preview` binary: parses the command line, delegates to composition and maps exit codes.

mod args;
mod exit;
mod output;

use std::io;
use std::net::IpAddr;
use std::process::{self, ExitCode};
use std::sync::Arc;

use clap::Parser;
use preview_composition::{
    BindScope, ServeConfig, StartupError, StartupReporter, StopSignal, Stopped,
};

use crate::args::{Cli, Command, ServeArgs};
use crate::exit::Outcome;
use crate::output::{HumanReport, JsonReport, StderrWarnings};

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

/// Refuses a remote bind without `--allow-remote` as a usage error (exit 2), then runs the
/// server with the human report, or with the JSONL report under `--json`; the remote-binding
/// warning is printed once the listener is bound. Every other start-up refusal (an invalid or
/// reserved root, the runtime, the signal handlers, the bind) exits 1 with its message, which
/// says what to do next, on stderr in both modes. A second stop signal during the drain exits
/// 128 plus its number.
fn serve(config: &ServeConfig) -> Outcome {
    let remote = match config.bind_scope() {
        Err(refusal) => {
            eprintln!("site-preview: error: {refusal}");
            return Outcome::Usage;
        }
        Ok(BindScope::Remote) => Some(config.host),
        Ok(BindScope::Loopback) => None,
    };
    let report = reporter(config.json, remote);
    match preview_composition::run(config, report, Arc::new(StderrWarnings)) {
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

/// The reporter for the chosen output mode, warning about a remote bind (`remote`) once ready.
fn reporter(json: bool, remote: Option<IpAddr>) -> Box<dyn StartupReporter> {
    if json {
        let report = JsonReport::new(io::stdout(), io::stderr(), process::id());
        Box::new(match remote {
            Some(host) => report.warn_remote(host),
            None => report,
        })
    } else {
        let report = HumanReport::new(io::stdout(), io::stderr());
        Box::new(match remote {
            Some(host) => report.warn_remote(host),
            None => report,
        })
    }
}
