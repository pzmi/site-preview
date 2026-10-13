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
    /// LLVM line coverage of the whole workspace against `coverage-floor.toml`.
    Coverage {
        /// Gate only the lines changed since REV (default: the merge-base with origin/main,
        /// else HEAD~1): at least 95% of instrumented changed lines covered. Reuses
        /// target/coverage/lcov.info while it matches the tree, else measures.
        #[arg(long, value_name = "REV", num_args = 0..=1)]
        changed_since: Option<Option<String>>,
    },
    /// Dependency policy, secrets and workflow checks.
    Security,
    /// The full blocking PR-quality gate.
    Check {
        /// Run only these stages (catalog ids such as `coverage`), in the gate's order and
        /// through the same stage code as the full run. Repeat or separate with commas. An
        /// unknown name exits 2 and lists the stage names (OPS-02, ADR 0072).
        #[arg(long, value_name = "STAGE", action = clap::ArgAction::Append, value_delimiter = ',')]
        only: Vec<String>,
    },
    /// Longer source mutation analysis (cargo-mutants over `.cargo/mutants.toml`).
    Mutation {
        /// Try only the mutants on lines changed since REV (merge-base with HEAD, working
        /// tree included).
        #[arg(long, value_name = "REV")]
        in_diff: Option<String>,
        /// Mutate one workspace package only.
        #[arg(long, value_name = "CRATE")]
        package: Option<String>,
        /// Mutants tried in parallel (default: half the cores).
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
        jobs: Option<u32>,
    },
    /// Bounded fuzz run or reproduction.
    Fuzz {
        /// The target, a file name in `fuzz/fuzz_targets/` without `.rs`.
        target: String,
        /// Seconds to fuzz.
        #[arg(long, value_name = "SECONDS", default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
        time: u64,
        /// Run one saved input (a crash artifact) instead of fuzzing.
        #[arg(long, value_name = "ARTIFACT")]
        reproduce: Option<PathBuf>,
    },
    /// Apalache bounded verification and multi-seed simulation of the Quint models.
    FormalVerify {
        /// Run Miri over the domain and application tests instead.
        #[arg(long)]
        miri: bool,
    },
    /// Clean packaged artifact tests: builds, packages and smoke-tests the release archive.
    ReleaseCheck {
        /// Rust target triple to build and package (default: the host). The release targets
        /// are the four of plan section 11 M5, each run on its own native runner.
        #[arg(long, value_name = "TARGET")]
        target: Option<String>,
        /// Smoke this existing archive instead of building one (the release workflow's
        /// `verify` job); it must be named `site-preview-<TARGET>.tar.gz`.
        #[arg(long, value_name = "PATH", requires = "target")]
        archive: Option<PathBuf>,
    },
    /// One CycloneDX SBOM per release target, through the pinned cargo-cyclonedx.
    Sbom {
        /// Directory for `site-preview-<target>.cdx.json` (default: `target/sbom`).
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Non-blocking performance and soak measurement; writes `target/bench/report.json`.
    Bench,
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
    /// The subcommand name, which is also the catalog id (tests).
    #[cfg(test)]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Quick => "quick",
            Self::Architecture { .. } => "architecture",
            Self::Test { .. } => "test",
            Self::Conformance { .. } => "conformance",
            Self::Coverage { .. } => "coverage",
            Self::Security => "security",
            Self::Check { .. } => "check",
            Self::Mutation { .. } => "mutation",
            Self::Fuzz { .. } => "fuzz",
            Self::FormalVerify { .. } => "formal-verify",
            Self::ReleaseCheck { .. } => "release-check",
            Self::Sbom { .. } => "sbom",
            Self::Bench => "bench",
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
    fn should_select_the_miri_lane_when_the_flag_is_given() {
        // given
        let plain = Cli::try_parse_from(["xtask", "formal-verify"]).unwrap();
        let miri = Cli::try_parse_from(["xtask", "formal-verify", "--miri"]).unwrap();

        // when
        let flags = (plain.command, miri.command);

        // then
        assert!(matches!(flags.0, Command::FormalVerify { miri: false }));
        assert!(matches!(flags.1, Command::FormalVerify { miri: true }));
    }

    #[test]
    fn should_take_an_optional_target_when_release_check_is_parsed() {
        // given
        let host = Cli::try_parse_from(["xtask", "release-check"]).unwrap();
        let musl = Cli::try_parse_from([
            "xtask",
            "release-check",
            "--target",
            "x86_64-unknown-linux-musl",
        ])
        .unwrap();

        // when
        let targets = (host.command, musl.command);

        // then
        assert!(matches!(
            targets.0,
            Command::ReleaseCheck {
                target: None,
                archive: None
            }
        ));
        assert!(matches!(
            targets.1,
            Command::ReleaseCheck { target: Some(ref t), archive: None } if t == "x86_64-unknown-linux-musl"
        ));
    }

    #[test]
    fn should_require_a_target_when_an_archive_is_given_to_release_check() {
        // given
        let bare = ["xtask", "release-check", "--archive", "a.tar.gz"];
        let full = [
            "xtask",
            "release-check",
            "--target",
            "aarch64-apple-darwin",
            "--archive",
            "dl/site-preview-aarch64-apple-darwin.tar.gz",
        ];

        // when
        let (bare, full) = (Cli::try_parse_from(bare), Cli::try_parse_from(full));

        // then
        assert!(bare.is_err());
        assert!(matches!(
            full.unwrap().command,
            Command::ReleaseCheck { target: Some(_), archive: Some(ref a) }
                if a.ends_with("site-preview-aarch64-apple-darwin.tar.gz")
        ));
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
