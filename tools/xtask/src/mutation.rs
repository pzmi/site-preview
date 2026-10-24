//! `cargo xtask mutation [--in-diff <rev>] [--package <crate>] [--jobs N] [--shard K/N]`: a cargo-mutants
//! run over the scope of `.cargo/mutants.toml`, with a summary (foundation.md section 5.3,
//! ADR 0068).
//!
//! The run needs `cargo-mutants` and `cargo-nextest` at their pinned versions, checked first
//! (ADR 0010): a missing one is a failure with its install command, never a skip. cargo-mutants
//! writes its report to `target/mutants/mutants.out/`, and this command adds
//! `target/mutants/summary.md`: the counts, the score and the missed mutants.
//!
//! With `--in-diff <rev>` only the mutants on lines changed since `rev` are tried. The diff
//! is taken from the merge-base of `rev` and `HEAD` (the base of `changed-lines`, ADR 0064) to
//! the working tree and handed to `cargo mutants --in-diff`.
//!
//! **Exit code:** missed mutants and timeouts are findings to triage, not a failure of the
//! command (cargo-mutants exits 2 and 3 for them): the weekly lane reports them and the score
//! is ratcheted by hand (foundation.md section 5.3 gates only *new* survivors, later). A
//! failing unmutated baseline, a usage error or a crash is a failure. A diff with no mutant on
//! its lines (cargo-mutants then writes no report) is a clean result.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{changed_lines, repo, tools};

/// Where cargo-mutants writes (`mutants.out/` below it) and where the summary goes.
const OUTPUT_DIR: &str = "target/mutants";

/// cargo-mutants exit code: some mutants were missed.
const EXIT_MISSED: i32 = 2;

/// cargo-mutants exit code: some mutants timed out.
const EXIT_TIMEOUT: i32 = 3;

/// The outcome counts of one run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Mutants a test failed on.
    pub caught: u64,
    /// Mutants every selected test passed on.
    pub missed: u64,
    /// Mutants that ran past the time limit.
    pub timeout: u64,
    /// Mutants that did not compile.
    pub unviable: u64,
}

impl Counts {
    /// Mutants whose outcome says something about the tests: everything but `unviable`.
    pub fn viable(self) -> u64 {
        self.caught + self.missed + self.timeout
    }

    /// Whole-number percent `caught / viable`, rounded down; a timeout counts against the
    /// score, the conservative reading. `None` when no viable mutant ran.
    pub fn score(self) -> Option<u64> {
        let viable = self.viable();
        (viable > 0).then(|| self.caught * 100 / viable)
    }
}

/// Runs the command and returns the exit code.
pub fn run(
    in_diff: Option<&str>,
    package: Option<&str>,
    jobs: Option<u32>,
    shard: Option<&str>,
) -> i32 {
    match execute(in_diff, package, jobs, shard) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("xtask: mutation: {err}");
            1
        }
    }
}

fn execute(
    in_diff: Option<&str>,
    package: Option<&str>,
    jobs: Option<u32>,
    shard: Option<&str>,
) -> Result<i32, String> {
    tools::require("cargo-mutants")?;
    tools::require("cargo-nextest")?;
    let root = repo::root();
    let out = root.join(OUTPUT_DIR);
    // A previous run's report would otherwise be read as this run's.
    if out.exists() {
        std::fs::remove_dir_all(&out)
            .map_err(|e| format!("cannot clear {}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let diff = match in_diff {
        Some(rev) => Some(write_diff(&root, rev, &out)?),
        None => None,
    };
    if let Some(shard) = shard {
        println!("xtask: mutation: shard {shard} of the selected scope (zero-based)");
    }
    let args = mutants_args(
        &out,
        diff.as_deref(),
        package,
        jobs.unwrap_or_else(default_jobs),
        shard,
    );
    let status = Command::new("cargo")
        .args(&args)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cannot run cargo mutants: {e}"))?;
    let code = status.code().unwrap_or(1);
    if !matches!(code, 0 | EXIT_MISSED | EXIT_TIMEOUT) {
        return Err(format!(
            "cargo mutants exited {code} (4 is a failing unmutated baseline); no summary written"
        ));
    }
    let report = out.join("mutants.out");
    // cargo-mutants writes no report at all when the diff holds no mutant (a change of tests,
    // docs or comments only); that is a clean result, but only for a diff run that exited 0.
    let summary = if diff.is_some() && code == 0 && !report.exists() {
        no_mutants_summary()
    } else {
        summarize(&report)?
    };
    let summary = scoped_summary(summary, shard);
    std::fs::write(out.join("summary.md"), &summary)
        .map_err(|e| format!("cannot write the summary: {e}"))?;
    print!("{summary}");
    Ok(0)
}

/// Validates cargo-mutants' zero-based `K/N` shard syntax before a run clears its report.
pub fn parse_shard(value: &str) -> Result<String, String> {
    let invalid = || "shard must be K/N with 0 <= K < N".to_owned();
    let (k, n) = value.split_once('/').ok_or_else(invalid)?;
    let k = k.parse::<u32>().map_err(|_| invalid())?;
    let n = n.parse::<u32>().map_err(|_| invalid())?;
    if k >= n {
        return Err(invalid());
    }
    Ok(format!("{k}/{n}"))
}

fn scoped_summary(summary: String, shard: Option<&str>) -> String {
    match shard {
        Some(shard) => format!(
            "Scope: shard `{shard}` (zero-based). Counts and score describe this shard only, not the full baseline.\n\n{summary}"
        ),
        None => summary,
    }
}

/// Half the cores, at least one: every job also runs a parallel nextest.
fn default_jobs() -> u32 {
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    u32::try_from((cores / 2).max(1)).unwrap_or(1)
}

/// The arguments of `cargo`: `mutants --output <dir> ...`.
fn mutants_args(
    out: &Path,
    diff: Option<&Path>,
    package: Option<&str>,
    jobs: u32,
    shard: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "mutants".to_owned(),
        "--output".to_owned(),
        out.display().to_string(),
        "--jobs".to_owned(),
        jobs.to_string(),
    ];
    if let Some(shard) = shard {
        args.extend(["--shard".to_owned(), shard.to_owned()]);
    }
    if let Some(package) = package {
        args.push("--package".to_owned());
        args.push(package.to_owned());
    }
    if let Some(diff) = diff {
        args.push("--in-diff".to_owned());
        args.push(diff.display().to_string());
    }
    args
}

/// Writes `git diff <merge-base of rev and HEAD>` of `crates/` (working tree included) to
/// `<out>/in-diff.patch` and returns its path.
fn write_diff(root: &Path, rev: &str, out: &Path) -> Result<PathBuf, String> {
    let base = changed_lines::resolve_base(root, Some(rev))?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--no-color", "--no-ext-diff", "--no-textconv"])
        .arg(&base.commit)
        .args(["--", "crates/"])
        .output()
        .map_err(|e| format!("cannot run git diff: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff {}: {}",
            base.commit,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let path = out.join("in-diff.patch");
    std::fs::write(&path, &output.stdout)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    println!(
        "xtask: mutation: mutants on lines changed since {}",
        base.label
    );
    Ok(path)
}

/// The non-empty lines of one of cargo-mutants' list files; a missing file is an empty list
/// (cargo-mutants writes none when nothing fell in that outcome).
fn read_list(dir: &Path, name: &str) -> Result<Vec<String>, String> {
    let path = dir.join(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_owned)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// The summary of the report in `dir` (cargo-mutants' `mutants.out`).
fn summarize(dir: &Path) -> Result<String, String> {
    if !dir.join("outcomes.json").is_file() {
        // cargo-mutants writes an empty mutant list but no outcomes for an empty shard.
        let empty_selection = std::fs::read(dir.join("mutants.json"))
            .ok()
            .is_some_and(|bytes| {
                bytes
                    .into_iter()
                    .filter(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                    .eq(b"[]".iter().copied())
            });
        if empty_selection {
            return Ok(no_mutants_summary());
        }
        return Err(format!(
            "{} has no outcomes.json: cargo mutants wrote no report",
            dir.display()
        ));
    }
    let missed = read_list(dir, "missed.txt")?;
    let timeouts = read_list(dir, "timeout.txt")?;
    let counts = Counts {
        caught: read_list(dir, "caught.txt")?.len() as u64,
        missed: missed.len() as u64,
        timeout: timeouts.len() as u64,
        unviable: read_list(dir, "unviable.txt")?.len() as u64,
    };
    Ok(render(counts, &missed, &timeouts))
}

/// The summary of a diff run that found no mutant to try.
fn no_mutants_summary() -> String {
    "# Mutation summary\n\nNo mutant in the selected scope: nothing was tried.\n".to_owned()
}

/// The summary text.
fn render(counts: Counts, missed: &[String], timeouts: &[String]) -> String {
    let mut out = String::from("# Mutation summary\n\n");
    let score = counts.score().map_or_else(
        || "n/a (no viable mutant ran)".to_owned(),
        |s| format!("{s}%"),
    );
    let _ = writeln!(
        out,
        "caught {}, missed {}, timeout {}, unviable {}; score {score} (caught / (caught + missed + timeout))\n",
        counts.caught, counts.missed, counts.timeout, counts.unviable
    );
    for (title, list) in [("Missed", missed), ("Timeout", timeouts)] {
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(out, "## {title}\n");
        for line in list {
            let _ = writeln!(out, "- {line}");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_score_caught_over_viable_when_mutants_ran() {
        // given
        let counts = Counts {
            caught: 9,
            missed: 1,
            timeout: 1,
            unviable: 50,
        };

        // when
        let score = counts.score();

        // then
        assert_eq!(counts.viable(), 11);
        assert_eq!(score, Some(81));
    }

    #[test]
    fn should_have_no_score_when_no_viable_mutant_ran() {
        // given
        let counts = Counts {
            unviable: 3,
            ..Counts::default()
        };

        // when
        let score = counts.score();

        // then
        assert_eq!(score, None);
    }

    #[test]
    fn should_pass_diff_package_and_jobs_when_arguments_are_built() {
        // given
        let out = Path::new("target/mutants");
        let diff = Path::new("target/mutants/in-diff.patch");

        // when
        let plain = mutants_args(out, None, None, 2, None);
        let scoped = mutants_args(out, Some(diff), Some("preview-domain"), 3, None);

        // then
        assert_eq!(
            plain,
            ["mutants", "--output", "target/mutants", "--jobs", "2"]
        );
        assert_eq!(
            scoped[5..],
            [
                "--package",
                "preview-domain",
                "--in-diff",
                "target/mutants/in-diff.patch"
            ]
        );
        assert_eq!(scoped[4], "3");
    }

    #[test]
    fn should_select_and_label_subset_when_shard_requested() {
        // given
        let shard = parse_shard("3/10").expect("valid shard");

        // when
        let args = mutants_args(Path::new("out"), None, None, 2, Some(&shard));
        let summary = scoped_summary("# Mutation summary\n".to_owned(), Some(&shard));

        // then
        assert_eq!(&args[5..], ["--shard", "3/10"]);
        assert!(summary.contains("shard `3/10`"));
        assert!(summary.contains("not the full baseline"));
        assert_eq!(scoped_summary("full".to_owned(), None), "full");
    }

    #[test]
    fn should_reject_invalid_shards_when_arguments_are_parsed() {
        // given
        let invalid = ["", "1", "0/0", "10/10", "-1/10", "1/2/3", "x/10"];

        // when
        let results = invalid.map(parse_shard);
        let valid = parse_shard("0/1");

        // then
        for (shard, result) in invalid.into_iter().zip(results) {
            assert!(result.is_err(), "{shard}");
        }
        assert_eq!(valid, Ok("0/1".to_owned()));
    }

    #[test]
    fn should_list_missed_mutants_when_report_is_summarized() {
        // given
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [
            ("outcomes.json", "{}"),
            ("caught.txt", "a\nb\nc\n"),
            ("missed.txt", "src/x.rs:1:2: replace f -> bool with true\n"),
            ("unviable.txt", "u\n"),
        ] {
            std::fs::write(dir.path().join(name), text).unwrap();
        }

        // when
        let summary = summarize(dir.path()).unwrap();

        // then
        assert!(summary.contains("caught 3, missed 1, timeout 0, unviable 1; score 75%"));
        assert!(summary.contains("## Missed"));
        assert!(summary.contains("- src/x.rs:1:2: replace f -> bool with true"));
        assert!(!summary.contains("## Timeout"));
    }

    #[test]
    fn should_report_no_score_when_shard_selects_no_mutants() {
        // given
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mutants.json"), "[]").unwrap();

        // when
        let summary = summarize(dir.path()).unwrap();

        // then
        assert!(summary.contains("nothing was tried"));
        assert!(!summary.contains("score"));
    }

    #[test]
    fn should_fail_when_report_has_no_outcomes() {
        // given
        let dir = tempfile::tempdir().unwrap();

        // when
        let err = summarize(dir.path()).unwrap_err();

        // then
        assert!(err.contains("no outcomes.json"), "{err}");
    }

    #[test]
    fn should_say_nothing_was_tried_when_the_diff_holds_no_mutant() {
        // given
        let summary = no_mutants_summary();

        // when
        let tried = summary.contains("nothing was tried");

        // then
        assert!(tried, "{summary}");
        assert!(!summary.contains("score"), "{summary}");
    }

    #[test]
    fn should_list_timeouts_when_any_mutant_timed_out() {
        // given
        let counts = Counts {
            caught: 1,
            timeout: 1,
            ..Counts::default()
        };

        // when
        let text = render(counts, &[], &["t".to_owned()]);

        // then
        assert!(text.contains("score 50%"), "{text}");
        assert!(text.contains("## Timeout\n\n- t\n"), "{text}");
    }
}
