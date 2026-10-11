//! The `site-preview` binary: parses the command line, delegates to composition and maps exit codes.

mod args;
mod exit;
mod output;

use std::io;
use std::process::ExitCode;

use clap::Parser;
use preview_composition::{BindScope, ServeConfig, StartupError};

use crate::args::{Cli, Command, ServeArgs};
use crate::exit::Outcome;
use crate::output::HumanReport;

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

/// Refuses a remote bind without `--allow-remote` as a usage error (exit 2), warns on stderr
/// when one is allowed, then runs the server.
fn serve(config: &ServeConfig) -> Outcome {
    match config.bind_scope() {
        Err(refusal) => {
            eprintln!("site-preview: error: {refusal}");
            return Outcome::Usage;
        }
        Ok(BindScope::Remote) => eprintln!(
            "site-preview: warning[remote-binding]: listening on {} with no authentication: \
             anyone who can reach this address can read the site; do not use it on an \
             untrusted network",
            config.host
        ),
        Ok(BindScope::Loopback) => {}
    }
    match preview_composition::run(config, &mut HumanReport::new(io::stdout())) {
        Ok(()) => Outcome::Success,
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
