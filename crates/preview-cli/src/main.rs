//! The `site-preview` binary: parses the command line, delegates to composition and maps exit codes.

mod args;
mod exit;

use std::process::ExitCode;

use clap::{CommandFactory, Parser, error::ErrorKind};

use crate::args::{Cli, Command};
use crate::exit::Outcome;

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
        Command::Serve => {
            let error = Cli::command().error(
                ErrorKind::MissingRequiredArgument,
                "`serve` is not available yet",
            );
            let _ = error.print();
            Outcome::Usage
        }
    }
}
