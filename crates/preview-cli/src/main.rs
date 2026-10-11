//! The `site-preview` binary: parses the command line, delegates to composition and maps exit codes.

mod args;
mod exit;
mod output;

use std::io;
use std::process::ExitCode;

use clap::Parser;
use preview_composition::ServeConfig;

use crate::args::{Cli, Command};
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
        Command::Serve(args) => {
            let config = ServeConfig::new(args.directory);
            match preview_composition::run(&config, &mut HumanReport::new(io::stdout())) {
                Ok(()) => Outcome::Success,
                Err(error) => {
                    eprintln!("site-preview: error: {error}");
                    Outcome::Failure
                }
            }
        }
    }
}
