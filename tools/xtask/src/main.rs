//! Repository automation entry point, invoked as `cargo xtask`.

mod action_pins;
mod apalache;
mod architecture;
mod bench;
mod browser;
mod catalog;
mod changed_lines;
mod check;
mod cli;
mod conformance;
mod coverage;
mod formal_verify;
mod fuzz;
mod mutation;
mod process;
mod quick;
mod repo;
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
        Command::Test { only } => test::run(only.as_deref()),
        Command::Check { only } => check::run(only),
        Command::Conformance { typecheck } => conformance::run(*typecheck),
        Command::Coverage { changed_since } => match changed_since {
            None => coverage::command(),
            Some(rev) => changed_lines::command(rev.as_deref()),
        },
        Command::FormalVerify { miri } => formal_verify::run(*miri),
        Command::Security => security::run(),
        Command::Fmt => run_fmt(),
        Command::Architecture { manifest_path } => architecture::run(manifest_path.as_deref()),
        Command::ListChecks => {
            print!("{}", catalog::to_json(&catalog::all_rows()));
            0
        }
        Command::Fuzz {
            target,
            time,
            reproduce,
        } => fuzz::run(target, *time, reproduce.as_deref()),
        Command::Mutation {
            in_diff,
            package,
            jobs,
        } => mutation::run(in_diff.as_deref(), package.as_deref(), *jobs),
        Command::Bench => bench::run(),
        Command::Doctor => tools::doctor(),
        Command::InstallTools { tools } => tools::install(tools),
        other => not_available(other.id()),
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// `cargo fmt --all`, then the fuzz targets, which are outside the workspace (ADR 0069).
fn run_fmt() -> i32 {
    let workspace = match Proc::new("cargo").args(["fmt", "--all"]).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("xtask: cannot run cargo fmt: {err}");
            1
        }
    };
    let fuzz_targets = fuzz::rustfmt_targets(false);
    if workspace != 0 {
        workspace
    } else {
        fuzz_targets
    }
}

fn not_available(id: &str) -> i32 {
    let owner = catalog::find(id).map_or("an unknown task", |c| c.owner);
    eprintln!("not available yet: introduced by {owner}");
    i32::from(NOT_AVAILABLE)
}
