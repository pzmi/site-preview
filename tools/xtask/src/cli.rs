//! Command-line surface of `cargo xtask`, from foundation.md section 9 plus `doctor` and
//! `install-tools`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Repository automation: gate, architecture checker and tool management.
#[derive(Debug, Parser)]
#[command(name = "xtask", version)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Every command of the surface. Names are the kebab-case variant names.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Apply formatting.
    Fmt,
    /// Formatter check, architecture policy, Clippy and fast tests. Not equivalent to `check`.
    Quick,
    /// Crate and module policy plus negative fixtures.
    Architecture {
        /// Check only the workspace with this root manifest (no fixtures); its root holds
        /// `architecture.toml` and `Cargo.lock`.
        #[arg(long, value_name = "PATH")]
        manifest_path: Option<PathBuf>,
    },
    /// All ordinary test levels including docs and E2E.
    Test {
        /// Run only this stage (a `test` stage id such as `browser`), not the others.
        #[arg(long, value_name = "STAGE")]
        only: Option<String>,
    },
    /// Quint model, scenarios and traces plus Rust replay.
    Conformance {
        /// Only typecheck the Quint models.
        #[arg(long)]
        typecheck: bool,
    },
    /// LLVM coverage, threshold and changed-code gate.
    Coverage,
    /// Dependency policy, secrets and workflow checks.
    Security,
    /// The full blocking PR-quality gate.
    Check,
    /// Longer source mutation analysis.
    Mutation,
    /// Bounded fuzz run or reproduction.
    Fuzz,
    /// Apalache and Kani deep lane.
    FormalVerify,
    /// Clean packaged artifact tests.
    ReleaseCheck,
    /// Print the check catalog as JSON.
    ListChecks,
    /// Report missing or wrong-version dev tools.
    Doctor,
    /// Install pinned dev tools.
    InstallTools {
        /// Tools to install; all when empty.
        tools: Vec<String>,
    },
}

impl Command {
    /// The subcommand name, which is also the catalog id.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Quick => "quick",
            Self::Architecture { .. } => "architecture",
            Self::Test { .. } => "test",
            Self::Conformance { .. } => "conformance",
            Self::Coverage => "coverage",
            Self::Security => "security",
            Self::Check => "check",
            Self::Mutation => "mutation",
            Self::Fuzz => "fuzz",
            Self::FormalVerify => "formal-verify",
            Self::ReleaseCheck => "release-check",
            Self::ListChecks => "list-checks",
            Self::Doctor => "doctor",
            Self::InstallTools { .. } => "install-tools",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn should_match_clap_names_when_ids_are_derived() {
        // given
        let parsed = Cli::try_parse_from(["xtask", "formal-verify"]).unwrap();

        // when
        let id = parsed.command.id();

        // then
        assert_eq!(id, "formal-verify");
    }

    #[test]
    fn should_have_valid_definition_when_clap_asserts_it() {
        // given
        let command = Cli::command();

        // when
        let name = command.get_name().to_owned();

        // then
        command.debug_assert();
        assert_eq!(name, "xtask");
    }
}
