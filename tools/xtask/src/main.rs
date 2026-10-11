//! Repository automation entry point, invoked as `cargo xtask`.

mod action_pins;
mod architecture;
mod catalog;
mod check;
mod cli;
mod process;
mod quick;
mod security;
mod test;
mod tools;

use std::process::{Command as Proc, ExitCode};

use clap::Parser;
use cli::{Cli, Command};

/// Exit code for a command whose implementing task has not landed.
const NOT_AVAILABLE: u8 = 3;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = match &cli.command {
        Command::Quick => quick::run(),
        Command::Test => test::run(),
        Command::Check => check::run(),
        Command::Security => security::run(),
        Command::Fmt => run_fmt(),
        Command::Architecture { manifest_path } => architecture::run(manifest_path.as_deref()),
        Command::ListChecks => {
            print!("{}", catalog::to_json(&catalog::all_rows()));
            0
        }
        Command::Doctor => tools::doctor(),
        Command::InstallTools { tools } => tools::install(tools),
        other => not_available(other.id()),
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn run_fmt() -> i32 {
    match Proc::new("cargo").args(["fmt", "--all"]).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("xtask: cannot run cargo fmt: {err}");
            1
        }
    }
}

fn not_available(id: &str) -> i32 {
    let owner = catalog::find(id).map_or("an unknown task", |c| c.owner);
    eprintln!("not available yet: introduced by {owner}");
    i32::from(NOT_AVAILABLE)
}
