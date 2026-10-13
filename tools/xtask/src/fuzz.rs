//! `cargo xtask fuzz <target> [--time N] [--reproduce <artifact>]`: a bounded cargo-fuzz run of
//! one target of the `fuzz/` crate, or the replay of one saved input (foundation.md section 5,
//! ADR 0026, SEC-04).
//!
//! The run needs `cargo-fuzz` and the dated nightly of `tools/versions.toml`, both checked
//! first (ADR 0010): a missing one is a failure with its install command, never a skip. The
//! nightly is used only here, through `cargo +<nightly>`; the production build stays on the
//! stable toolchain. A run reads the committed seeds in `fuzz/seeds/<target>/` and writes its
//! findings to the ignored `fuzz/corpus/<target>/`; a crash leaves its input under
//! `fuzz/artifacts/<target>/`, and the process exits non-zero.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{repo, tools};

/// The `tools/versions.toml` entry of the pinned nightly.
const NIGHTLY_TOOL: &str = "nightly";

/// Where the targets live, relative to the repository root.
const TARGETS_DIR: &str = "fuzz/fuzz_targets";

/// Committed seed corpora, one directory per target, relative to the repository root.
const SEEDS_DIR: &str = "fuzz/seeds";

/// Generated corpora (ignored by Git), one directory per target.
const CORPUS_DIR: &str = "fuzz/corpus";

/// Runs the command and returns the exit code.
pub fn run(target: &str, time: u64, reproduce: Option<&Path>) -> i32 {
    match prepare(target, time, reproduce) {
        Ok(args) => execute(&args),
        Err(err) => {
            eprintln!("xtask: fuzz: {err}");
            1
        }
    }
}

/// Checks the tools and the target and builds the `cargo` arguments.
fn prepare(target: &str, time: u64, reproduce: Option<&Path>) -> Result<Vec<String>, String> {
    let root = repo::root();
    tools::require("cargo-fuzz")?;
    tools::require(NIGHTLY_TOOL)?;
    let nightly = pinned_nightly()?;
    check_target(&available_targets(&root)?, target)?;
    let reproduce = reproduce
        .map(std::path::absolute)
        .transpose()
        .map_err(|e| format!("cannot resolve the --reproduce path: {e}"))?;
    if let Some(artifact) = &reproduce
        && !artifact.is_file()
    {
        return Err(format!("--reproduce: {} is not a file", artifact.display()));
    }
    let seeds = root.join(SEEDS_DIR).join(target);
    let corpus = root.join(CORPUS_DIR).join(target);
    // libFuzzer refuses a corpus directory that does not exist.
    if reproduce.is_none() {
        std::fs::create_dir_all(&corpus)
            .map_err(|e| format!("cannot create {}: {e}", corpus.display()))?;
    }
    Ok(cargo_args(
        &nightly,
        target,
        time,
        reproduce.as_deref(),
        &corpus,
        seeds.is_dir().then_some(seeds.as_path()),
    ))
}

/// The arguments of `cargo`: `+<nightly> fuzz run <target> ...`.
///
/// A run lists the generated corpus first (libFuzzer writes new inputs to the first
/// directory) and the committed seeds second (read only), then bounds the run with
/// `-max_total_time`. A reproduction passes the one file, which libFuzzer executes once.
fn cargo_args(
    nightly: &str,
    target: &str,
    time: u64,
    reproduce: Option<&Path>,
    corpus: &Path,
    seeds: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        format!("+{nightly}"),
        "fuzz".to_owned(),
        "run".to_owned(),
        target.to_owned(),
    ];
    if let Some(artifact) = reproduce {
        args.push(artifact.display().to_string());
        return args;
    }
    args.push(corpus.display().to_string());
    if let Some(seeds) = seeds {
        args.push(seeds.display().to_string());
    }
    args.push("--".to_owned());
    args.push(format!("-max_total_time={time}"));
    args.push("-print_final_stats=1".to_owned());
    args
}

fn execute(args: &[String]) -> i32 {
    match Command::new("cargo")
        .args(args)
        .current_dir(repo::root())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("xtask: fuzz: cannot run cargo: {err}");
            1
        }
    }
}

/// The pinned nightly's toolchain name, from `tools/versions.toml`.
pub fn pinned_nightly() -> Result<String, String> {
    tools::manifest()?
        .into_iter()
        .find(|t| t.name == NIGHTLY_TOOL)
        .map(|t| t.version)
        .ok_or_else(|| "tools/versions.toml has no nightly entry".to_owned())
}

/// The names of the fuzz targets: the `*.rs` files of `fuzz/fuzz_targets/`, sorted.
fn available_targets(root: &Path) -> Result<Vec<String>, String> {
    let dir: PathBuf = root.join(TARGETS_DIR);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {TARGETS_DIR}: {e}"))?;
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".rs").map(str::to_owned))
        .collect();
    names.sort();
    Ok(names)
}

/// The `fuzz-fmt` stage as `quick` and `check` show it (ADR 0069).
pub const FMT_DISPLAY: &str = "rustfmt --check --edition 2024 fuzz/fuzz_targets/*.rs";

/// The `rustfmt` arguments over the target files: `--check` to only report.
fn rustfmt_args(root: &Path, check: bool) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    if check {
        args.push("--check".to_owned());
    }
    args.extend(["--edition".to_owned(), "2024".to_owned()]);
    for target in available_targets(root)? {
        args.push(
            root.join(TARGETS_DIR)
                .join(format!("{target}.rs"))
                .display()
                .to_string(),
        );
    }
    Ok(args)
}

/// Runs `rustfmt` over the fuzz targets, which `cargo fmt --all` never sees because the fuzz
/// crate is not a workspace member; `check` only reports. Returns the exit code.
pub fn rustfmt_targets(check: bool) -> i32 {
    let root = repo::root();
    let args = match rustfmt_args(&root, check) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("xtask: {err}");
            return 1;
        }
    };
    match Command::new("rustfmt")
        .args(&args)
        .current_dir(&root)
        .status()
    {
        Ok(status) if status.success() => 0,
        Ok(status) => {
            if check {
                eprintln!("xtask: a fuzz target is not formatted; run `cargo xtask fmt`");
            }
            status.code().unwrap_or(1)
        }
        Err(err) => {
            eprintln!(
                "xtask: cannot run rustfmt: {err}; install it with `rustup component add rustfmt`"
            );
            1
        }
    }
}

/// The `fuzz-fmt` stage body.
pub fn fmt_check() -> i32 {
    rustfmt_targets(true)
}

/// Fails, naming the targets that exist, unless `target` is one of them. Taking the name from
/// the directory also keeps a path or an option out of the `cargo` command line.
fn check_target(available: &[String], target: &str) -> Result<(), String> {
    if available.iter().any(|n| n == target) {
        Ok(())
    } else {
        Err(format!(
            "unknown fuzz target `{target}`; available: {}",
            available.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_list_decode_path_when_targets_are_scanned() {
        // given
        let root = repo::root();

        // when
        let targets = available_targets(&root).unwrap();

        // then
        assert!(targets.contains(&"decode_path".to_owned()), "{targets:?}");
    }

    #[test]
    fn should_refuse_target_when_name_is_not_a_fuzz_target() {
        // given
        let available = vec!["decode_path".to_owned()];

        // when
        let err = check_target(&available, "../evil").unwrap_err();

        // then
        assert!(err.contains("unknown fuzz target"), "{err}");
        assert!(err.contains("decode_path"), "{err}");
    }

    #[test]
    fn should_bound_run_and_read_seeds_when_not_reproducing() {
        // given
        let corpus = Path::new("/c/corpus/t");
        let seeds = Path::new("/c/seeds/t");

        // when
        let args = cargo_args("nightly-x", "t", 60, None, corpus, Some(seeds));

        // then
        assert_eq!(
            args,
            [
                "+nightly-x",
                "fuzz",
                "run",
                "t",
                "/c/corpus/t",
                "/c/seeds/t",
                "--",
                "-max_total_time=60",
                "-print_final_stats=1"
            ]
        );
    }

    #[test]
    fn should_pass_only_the_artifact_when_reproducing() {
        // given
        let artifact = Path::new("/a/crash-1");

        // when
        let args = cargo_args(
            "nightly-x",
            "t",
            60,
            Some(artifact),
            Path::new("/c"),
            Some(Path::new("/s")),
        );

        // then
        assert_eq!(args, ["+nightly-x", "fuzz", "run", "t", "/a/crash-1"]);
    }

    #[test]
    fn should_pin_the_same_nightly_when_toolchain_file_and_manifest_are_compared() {
        // given
        let manifest = pinned_nightly().unwrap();

        // when
        let file = std::fs::read_to_string(repo::root().join("fuzz/rust-toolchain.toml")).unwrap();

        // then
        assert!(
            file.contains(&format!("channel = \"{manifest}\"")),
            "fuzz/rust-toolchain.toml must pin {manifest}"
        );
    }

    #[test]
    fn should_declare_a_bin_when_a_target_exists() {
        // given
        let root = repo::root();
        let manifest: toml::Table = std::fs::read_to_string(root.join("fuzz/Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();

        // when
        let mut bins: Vec<String> = manifest["bin"]
            .as_array()
            .unwrap()
            .iter()
            .map(|bin| bin["name"].as_str().unwrap().to_owned())
            .collect();
        bins.sort();

        // then
        assert_eq!(bins, available_targets(&root).unwrap());
    }

    #[test]
    fn should_schedule_every_target_when_fuzz_workflow_is_read() {
        // given
        let root = repo::root();
        let text = std::fs::read_to_string(root.join(".github/workflows/fuzz.yml")).unwrap();
        let docs = yaml_rust2::YamlLoader::load_from_str(&text).unwrap();

        // when
        let mut scheduled: Vec<String> = docs[0]["jobs"]["fuzz"]["strategy"]["matrix"]["target"]
            .as_vec()
            .expect("fuzz.yml has a target matrix")
            .iter()
            .map(|t| t.as_str().unwrap().to_owned())
            .collect();
        scheduled.sort();

        // then
        assert_eq!(scheduled, available_targets(&root).unwrap());
    }

    #[test]
    fn should_check_every_target_with_edition_2024_when_rustfmt_args_are_built() {
        // given
        let root = repo::root();

        // when
        let args = rustfmt_args(&root, true).unwrap();

        // then
        assert_eq!(&args[..3], ["--check", "--edition", "2024"]);
        let files: Vec<&String> = args[3..].iter().collect();
        assert_eq!(files.len(), available_targets(&root).unwrap().len());
        assert!(
            files
                .iter()
                .all(|f| f.ends_with(".rs") && f.contains(TARGETS_DIR))
        );
        assert!(
            !rustfmt_args(&root, false)
                .unwrap()
                .contains(&"--check".to_owned())
        );
    }

    #[test]
    fn should_carry_the_workspace_lints_when_fuzz_manifest_is_read() {
        // given
        let root = repo::root();
        let read = |path: &str| -> toml::Table {
            std::fs::read_to_string(root.join(path))
                .unwrap()
                .parse()
                .unwrap()
        };

        // when
        let workspace = read("Cargo.toml")["workspace"]["lints"].clone();
        let fuzz = read("fuzz/Cargo.toml")["lints"].clone();

        // then
        assert_eq!(
            fuzz, workspace,
            "fuzz/Cargo.toml [lints] must equal [workspace.lints]"
        );
    }

    #[test]
    fn should_have_a_seed_directory_when_a_target_exists() {
        // given
        let root = repo::root();

        // when
        let targets = available_targets(&root).unwrap();

        // then
        for target in targets {
            let seeds = root.join(SEEDS_DIR).join(&target);
            let count = std::fs::read_dir(&seeds).map_or(0, Iterator::count);
            assert!(count > 0, "fuzz target {target} has no seeds in {seeds:?}");
        }
    }
}
