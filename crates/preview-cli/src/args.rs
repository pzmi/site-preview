//! Command-line grammar of `site-preview`, declared with `clap`'s derive API.

use clap::{Parser, Subcommand};

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
    Serve,
}
