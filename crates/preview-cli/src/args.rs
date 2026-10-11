//! Command-line grammar of `site-preview`, declared with `clap`'s derive API.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Zero-config local live-reload server for static sites.
#[derive(Debug, Parser)]
#[command(name = "site-preview", version, about, arg_required_else_help = true)]
pub struct Cli {
    /// The action to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands of `site-preview`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve a directory and reload the browser when a file changes.
    Serve(ServeArgs),
}

/// The arguments of `serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// The directory to serve (the current directory when omitted).
    #[arg(default_value = ".")]
    pub directory: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_serve_current_directory_when_no_directory_is_given() {
        // given
        let argv = ["site-preview", "serve"];

        // when
        let cli = Cli::try_parse_from(argv).expect("serve parses");

        // then
        let Command::Serve(args) = cli.command;
        assert_eq!(args.directory, PathBuf::from("."));
    }

    #[test]
    fn should_take_directory_when_given_as_positional() {
        // given
        let argv = ["site-preview", "serve", "public"];

        // when
        let cli = Cli::try_parse_from(argv).expect("serve parses");

        // then
        let Command::Serve(args) = cli.command;
        assert_eq!(args.directory, PathBuf::from("public"));
    }
}
