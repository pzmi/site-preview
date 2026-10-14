//! Repository automation entry point, invoked as `cargo xtask`.

mod action_pins;
mod adr;
mod apalache;
mod architecture;
mod bench;
mod browser;
mod catalog;
mod changed_lines;
mod check;
mod cli;
mod commits;
mod conformance;
mod coverage;
mod docs;
mod formal_verify;
mod fuzz;
mod land;
mod mutation;
mod process;
mod quick;
mod release;
mod repo;
mod sbom;
mod security;
mod test;
mod tools;

use std::process::{Command as Proc, ExitCode};

use clap::Parser;
use cli::{AdrAction, Cli, Command};

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
        Command::ReleaseCheck { target, archive } => {
            release::run(target.as_deref(), archive.as_deref())
        }
        Command::Sbom { out } => sbom::run(out.as_deref()),
        Command::Bench => bench::run(),
        Command::Doctor { json, all } => tools::doctor(*json, *all),
        Command::InstallTools { tools } => tools::install(tools),
        Command::Adr { action } => match action {
            AdrAction::New {
                title,
                number,
                date,
                status,
            } => adr::run_new(&adr::NewAdr {
                title,
                number: *number,
                date: date.as_deref(),
                status,
            }),
            AdrAction::Index { fix } => adr::run_index(*fix),
        },
        Command::Land {
            refs,
            push,
            also,
            no_push: _,
            log,
        } => {
            let target = match push.as_deref() {
                Some([remote, branch]) => land::Target::Push {
                    remote: remote.clone(),
                    branch: branch.clone(),
                    also: also.clone(),
                },
                _ => land::Target::NoPush,
            };
            land::run(refs, &target, log.clone())
        }
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
