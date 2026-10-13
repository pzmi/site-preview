//! `cargo xtask coverage`: LLVM line coverage of every in-process suite and the floors that
//! gate it (foundation.md section 5.3, ADR 0018, ADR 0062).
//!
//! What is measured: `cargo llvm-cov nextest --workspace` under the `ci` profile, so every
//! unit, component, contract, integration, CLI black-box and conformance test of every
//! workspace member runs instrumented, including `tools/xtask` and the negative controls.
//! Not measured: the Playwright browser suite and the `client-js` suite (they drive the binary
//! or the browser script from outside the Rust instrumentation), doctests (stable `llvm-cov`
//! cannot instrument them), and files under `tests/`, which `cargo-llvm-cov` leaves out.
//!
//! The conformance replay runs here with `CONFORMANCE_GENERATED_MANIFEST` removed, so
//! `preview-conformance` generates its own traces with the locked Quint, exactly as it does
//! when the variable is unset (`preview_conformance::generate`). That keeps the stage
//! independent of whether `formal-generate` ran first and of a stale
//! `formal/traces/generated/`; the price is one more trace generation per `check`, and a
//! missing `npm ci --prefix formal` fails the stage with the conformance crate's own message.
//!
//! The report is `target/coverage/lcov.info`. Floors live in `coverage-floor.toml`; the
//! evaluator works on integer line counts, so there is no rounding at the boundary.
//!
//! Beside the report, a successful measurement writes `target/coverage/lcov.stamp`: the
//! fingerprint of the source tree it measured (`changed_lines::tree_fingerprint`, taken before
//! the run). The changed-line gate (TEST-07, ADR 0064) reads the report only while the stamp
//! still matches the tree, so it never joins a diff with an LCOV of other file contents.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::process::{Stage, finish, run_report};
use crate::{changed_lines, conformance, repo, test, tools};

/// The floors file, from the repository root.
const FLOOR_FILE: &str = "coverage-floor.toml";

/// Where the LCOV report is written, from the repository root.
pub const LCOV_PATH: &str = "target/coverage/lcov.info";

/// The fingerprint of the tree the LCOV report measured, from the repository root.
const STAMP_PATH: &str = "target/coverage/lcov.stamp";

/// No floor may be set below this (ADR 0018).
const MIN_FLOOR: u32 = 90;

/// The key under which files of `tools/xtask` are counted.
const XTASK: &str = "xtask";

/// The `coverage` stage, as `check` registers it.
pub fn stage() -> Stage {
    Stage::in_process(
        "coverage",
        "cargo llvm-cov nextest --workspace --locked --profile ci --lcov --output-path target/coverage/lcov.info, then coverage-floor.toml",
        run,
    )
    .needs("cargo-llvm-cov")
}

/// The floors in `coverage-floor.toml`.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Floors {
    /// Percent of all instrumented lines.
    pub overall_line: u32,
    /// Percent per crate directory name.
    #[serde(default)]
    pub crate_floors: BTreeMap<String, u32>,
}

/// The on-disk shape: `[crate]` is a keyword-like name, so it is renamed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FloorFile {
    overall_line: u32,
    #[serde(default, rename = "crate")]
    crates: BTreeMap<String, u32>,
}

/// Parses the floors file and refuses a floor outside `MIN_FLOOR..=100`.
pub fn parse_floors(text: &str) -> Result<Floors, String> {
    let file: FloorFile =
        toml::from_str(text).map_err(|e| format!("{FLOOR_FILE} does not parse: {e}"))?;
    let all = std::iter::once(("overall_line", file.overall_line))
        .chain(file.crates.iter().map(|(k, v)| (k.as_str(), *v)));
    for (name, value) in all {
        if !(MIN_FLOOR..=100).contains(&value) {
            return Err(format!(
                "{FLOOR_FILE}: {name} = {value} is outside {MIN_FLOOR}..=100 (ADR 0018)"
            ));
        }
    }
    Ok(Floors {
        overall_line: file.overall_line,
        crate_floors: file.crates,
    })
}

/// Instrumented and covered line counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Lines with a `DA:` record.
    pub instrumented: u64,
    /// Of those, lines executed at least once.
    pub covered: u64,
}

impl Counts {
    /// Whether `covered / instrumented` reaches `floor` percent, in integers.
    fn meets(self, floor: u32) -> bool {
        self.covered * 100 >= self.instrumented * u64::from(floor)
    }

    /// The percentage, for display only.
    fn percent(self) -> f64 {
        if self.instrumented == 0 {
            return 100.0;
        }
        self.covered as f64 * 100.0 / self.instrumented as f64
    }
}

/// Line counts of one LCOV report, overall and per crate.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Measured {
    /// Every file.
    pub overall: Counts,
    /// Per crate directory name (`preview-domain`, `xtask`, ...).
    pub per_crate: BTreeMap<String, Counts>,
}

/// The crate a source path belongs to: the directory after `crates/`, or `xtask` for
/// `tools/xtask/`.
fn crate_of(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    if let Some((_, rest)) = path.rsplit_once("/crates/") {
        return rest.split('/').next().map(str::to_owned);
    }
    path.contains("/tools/xtask/").then(|| XTASK.to_owned())
}

/// One `DA:<line>,<hits>[,<checksum>]` record; `number` is its zero-based line in the report.
fn parse_da(record: &str, number: usize) -> Result<(u32, u64), String> {
    let mut fields = record.split(',');
    let line = fields.next().and_then(|l| l.parse::<u32>().ok());
    let hits = fields.next().and_then(|h| h.parse::<u64>().ok());
    match (line, hits) {
        (Some(line), Some(hits)) => Ok((line, hits)),
        _ => Err(format!(
            "lcov line {}: bad DA record {record:?}",
            number + 1
        )),
    }
}

/// Reads the `SF:` / `DA:` records of an LCOV report.
pub fn parse_lcov(text: &str) -> Result<Measured, String> {
    let mut measured = Measured::default();
    let mut current: Option<String> = None;
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            current = crate_of(path);
        } else if let Some(record) = line.strip_prefix("DA:") {
            let (_, hits) = parse_da(record, number)?;
            let add = |counts: &mut Counts| {
                counts.instrumented += 1;
                counts.covered += u64::from(hits > 0);
            };
            add(&mut measured.overall);
            if let Some(name) = &current {
                add(measured.per_crate.entry(name.clone()).or_default());
            }
        }
    }
    if measured.overall.instrumented == 0 {
        return Err("the LCOV report has no instrumented line".to_owned());
    }
    Ok(measured)
}

/// Per repository-relative file (`crates/preview-domain/src/a.rs`), the hit count of each
/// instrumented line.
pub type FileLines = BTreeMap<String, BTreeMap<u32, u64>>;

/// The repository-relative, `/`-separated form of an LCOV `SF:` path, trying each root in
/// turn (the root as the build saw it, then its canonical form). `None` for a file outside
/// every root.
pub fn relative_path(path: &str, roots: &[String]) -> Option<String> {
    let path = path.replace('\\', "/");
    roots.iter().find_map(|root| {
        let root = root.replace('\\', "/");
        path.strip_prefix(root.trim_end_matches('/'))?
            .strip_prefix('/')
            .map(str::to_owned)
    })
}

/// Reads an LCOV report into per-file line hits, keyed by repository-relative path. Files
/// outside `roots` are left out. A line recorded twice for one file (one record per
/// instantiation) adds its hits up, so a line any instantiation ran counts as covered.
pub fn parse_lcov_lines(text: &str, roots: &[String]) -> Result<FileLines, String> {
    let mut files = FileLines::new();
    let mut current: Option<String> = None;
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            current = relative_path(path, roots);
        } else if line == "end_of_record" {
            current = None;
        } else if let Some(record) = line.strip_prefix("DA:") {
            let (at, hits) = parse_da(record, number)?;
            if let Some(file) = &current {
                let slot = files
                    .entry(file.clone())
                    .or_default()
                    .entry(at)
                    .or_default();
                *slot = slot.saturating_add(hits);
            }
        }
    }
    Ok(files)
}

/// The LCOV report, when its stamp matches the tree as it is now; `None` when there is no
/// report or stamp, or the tree changed since the report was measured.
pub fn fresh_lcov(root: &Path) -> Result<Option<String>, String> {
    let Ok(stamp) = std::fs::read_to_string(root.join(STAMP_PATH)) else {
        return Ok(None);
    };
    if stamp.trim() != changed_lines::tree_fingerprint(root)? {
        return Ok(None);
    }
    std::fs::read_to_string(root.join(LCOV_PATH))
        .map(Some)
        .map_err(|e| format!("cannot read {LCOV_PATH}: {e}"))
}

/// Records that the LCOV report measured the tree with this fingerprint.
fn write_stamp(root: &Path, fingerprint: &str) -> Result<(), String> {
    std::fs::write(root.join(STAMP_PATH), format!("{fingerprint}\n"))
        .map_err(|e| format!("cannot write {STAMP_PATH}: {e}"))
}

/// Compares the measurement with the floors. Returns the report text and whether every floor
/// holds. A floor naming a crate absent from the report fails: it would otherwise pass
/// silently after a rename.
pub fn evaluate(measured: &Measured, floors: &Floors) -> (String, bool) {
    let mut out = String::new();
    let mut ok = true;
    let mut row = |name: &str, counts: Counts, floor: Option<u32>| {
        let verdict = match floor {
            None => "no floor".to_owned(),
            Some(f) if counts.meets(f) => format!("floor {f}%: ok"),
            Some(f) => {
                ok = false;
                format!(
                    "BELOW floor {f}% by {:.2} points ({} uncovered lines)",
                    f64::from(f) - counts.percent(),
                    counts.instrumented - counts.covered
                )
            }
        };
        let _ = writeln!(
            out,
            "{name:<24} {:>6.2}%  {:>6}/{:<6} lines  {verdict}",
            counts.percent(),
            counts.covered,
            counts.instrumented
        );
    };
    row("overall", measured.overall, Some(floors.overall_line));
    for (name, counts) in &measured.per_crate {
        row(name, *counts, floors.crate_floors.get(name).copied());
    }
    for name in floors.crate_floors.keys() {
        if !measured.per_crate.contains_key(name) {
            ok = false;
            let _ = writeln!(
                out,
                "{name:<24} FAILED: floor set but no such crate in the report"
            );
        }
    }
    (out, ok)
}

/// Runs the instrumented workspace tests and writes the LCOV report and its stamp. The
/// fingerprint is taken before the run, so an edit made during it leaves the report stale. A
/// tree that cannot be fingerprinted (no git) gets no stamp: the floors still apply, and the
/// changed-line gate measures again.
pub fn measure(root: &Path) -> Result<(), String> {
    tools::require("cargo-nextest")?;
    std::fs::create_dir_all(root.join("target/coverage"))
        .map_err(|e| format!("cannot create target/coverage: {e}"))?;
    if let Err(e) = std::fs::remove_file(root.join(STAMP_PATH))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(format!("cannot remove {STAMP_PATH}: {e}"));
    }
    let fingerprint = changed_lines::tree_fingerprint(root);
    let status = Command::new("cargo")
        .args([
            "llvm-cov",
            "nextest",
            "--workspace",
            "--locked",
            "--profile",
            "ci",
            "--lcov",
            "--output-path",
            LCOV_PATH,
        ])
        .current_dir(root)
        .env_remove(conformance::GENERATED_ENV)
        .env(
            "PROPTEST_CASES",
            test::proptest_cases(std::env::var("PROPTEST_CASES").ok().as_deref()),
        )
        .status()
        .map_err(|e| format!("cannot run cargo llvm-cov: {e}"))?;
    if !status.success() {
        return Err(format!("cargo llvm-cov failed with {status}"));
    }
    match fingerprint {
        Ok(fingerprint) => write_stamp(root, &fingerprint),
        Err(e) => {
            eprintln!("xtask: {LCOV_PATH} gets no freshness stamp: {e}");
            Ok(())
        }
    }
}

/// `cargo xtask coverage`: the one stage, with its tool check; returns the exit code.
pub fn command() -> i32 {
    finish(&run_report(&[stage()], &tools::require))
}

/// The stage body: measure, evaluate, print; returns the exit code.
fn run() -> i32 {
    let root = repo::root();
    let result = (|| {
        let floors = parse_floors(
            &std::fs::read_to_string(root.join(FLOOR_FILE))
                .map_err(|e| format!("cannot read {FLOOR_FILE}: {e}"))?,
        )?;
        measure(&root)?;
        let lcov = std::fs::read_to_string(root.join(LCOV_PATH))
            .map_err(|e| format!("cannot read {LCOV_PATH}: {e}"))?;
        Ok::<_, String>(evaluate(&parse_lcov(&lcov)?, &floors))
    })();
    match result {
        Ok((report, true)) => {
            print!("{report}");
            println!("coverage: every floor holds; report at {LCOV_PATH}");
            0
        }
        Ok((report, false)) => {
            print!("{report}");
            eprintln!(
                "xtask: coverage is below a floor in {FLOOR_FILE}; add tests, never lower it"
            );
            1
        }
        Err(err) => {
            eprintln!("xtask: {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCOV: &str = "\
SF:/r/crates/preview-domain/src/a.rs
DA:1,3
DA:2,0
DA:3,1
end_of_record
SF:/r/tools/xtask/src/b.rs
DA:1,0
DA:2,5
end_of_record
";

    fn floors(overall: u32, domain: u32) -> Floors {
        Floors {
            overall_line: overall,
            crate_floors: BTreeMap::from([("preview-domain".to_owned(), domain)]),
        }
    }

    #[test]
    fn should_count_lines_per_crate_when_lcov_is_parsed() {
        // given
        let text = LCOV;

        // when
        let measured = parse_lcov(text).unwrap();

        // then
        assert_eq!(
            measured.overall,
            Counts {
                instrumented: 5,
                covered: 3
            }
        );
        assert_eq!(
            measured.per_crate["preview-domain"],
            Counts {
                instrumented: 3,
                covered: 2
            }
        );
        assert_eq!(measured.per_crate[XTASK].covered, 1);
    }

    #[test]
    fn should_pass_when_every_floor_is_met() {
        // given
        let measured = parse_lcov(LCOV).unwrap();

        // when
        let (report, ok) = evaluate(&measured, &floors(60, 66));

        // then
        assert!(ok, "{report}");
        assert!(report.contains("overall"), "{report}");
    }

    #[test]
    fn should_fail_and_name_the_gap_when_overall_floor_is_raised_to_100() {
        // given
        let measured = parse_lcov(LCOV).unwrap();

        // when
        let (report, ok) = evaluate(&measured, &floors(100, 66));

        // then
        assert!(!ok);
        assert!(
            report.contains("BELOW floor 100%") && report.contains("2 uncovered lines"),
            "{report}"
        );
    }

    #[test]
    fn should_fail_when_a_crate_floor_is_raised_to_100() {
        // given
        let measured = parse_lcov(LCOV).unwrap();

        // when
        let (report, ok) = evaluate(&measured, &floors(60, 100));

        // then
        assert!(!ok);
        assert!(report.contains("preview-domain"), "{report}");
        assert!(report.contains("BELOW floor 100%"), "{report}");
    }

    #[test]
    fn should_fail_when_a_floor_names_a_crate_missing_from_the_report() {
        // given
        let measured = parse_lcov(LCOV).unwrap();
        let mut set = floors(60, 66);
        set.crate_floors.insert("preview-gone".to_owned(), 90);

        // when
        let (report, ok) = evaluate(&measured, &set);

        // then
        assert!(!ok);
        assert!(report.contains("preview-gone"), "{report}");
    }

    #[test]
    fn should_not_round_up_when_coverage_is_just_below_the_floor() {
        // given: 8999 of 10000 lines is 89.99%
        let counts = Counts {
            instrumented: 10_000,
            covered: 8_999,
        };

        // when
        let meets = counts.meets(90);

        // then
        assert!(!meets);
    }

    #[test]
    fn should_refuse_a_floor_below_the_minimum_when_floors_are_parsed() {
        // given
        let text = "overall_line = 89\n";

        // when
        let err = parse_floors(text).unwrap_err();

        // then
        assert!(err.contains("overall_line = 89"), "{err}");
    }

    #[test]
    fn should_refuse_an_unknown_key_when_floors_are_parsed() {
        // given
        let text = "overall_line = 90\nbranch = 80\n";

        // when
        let result = parse_floors(text);

        // then
        assert!(result.is_err());
    }

    #[test]
    fn should_reject_a_report_without_lines_when_lcov_is_parsed() {
        // given
        let text = "SF:/r/crates/preview-domain/src/a.rs\nend_of_record\n";

        // when
        let result = parse_lcov(text);

        // then
        assert!(result.is_err());
    }

    #[test]
    fn should_hold_the_committed_floors_when_the_floor_file_is_parsed() {
        // given
        let text = std::fs::read_to_string(repo::root().join(FLOOR_FILE)).unwrap();

        // when
        let floors = parse_floors(&text).unwrap();

        // then
        assert!(floors.overall_line >= MIN_FLOOR);
        assert!(floors.crate_floors.contains_key("preview-domain"));
        assert!(floors.crate_floors.contains_key("preview-application"));
    }

    #[test]
    fn should_key_lines_by_repository_path_when_lcov_lines_are_read() {
        // given: a duplicate record for line 3 (two instantiations), a file outside the root
        // and a DA record before any SF
        let text = "\
DA:9,9
SF:/r/crates/preview-domain/src/a.rs
DA:1,3
DA:3,0
DA:3,2
end_of_record
DA:7,7
SF:/home/.cargo/registry/x.rs
DA:1,1
end_of_record
";
        let roots = ["/elsewhere".to_owned(), "/r/".to_owned()];

        // when
        let files = parse_lcov_lines(text, &roots).unwrap();

        // then
        assert_eq!(
            files,
            FileLines::from([(
                "crates/preview-domain/src/a.rs".to_owned(),
                BTreeMap::from([(1, 3), (3, 2)])
            )])
        );
    }

    #[test]
    fn should_refuse_a_record_without_a_line_number_when_lcov_lines_are_read() {
        // given
        let text = "SF:/r/crates/a/src/a.rs\nDA:x,1\n";

        // when
        let err = parse_lcov_lines(text, &["/r".to_owned()]).unwrap_err();

        // then
        assert!(err.contains("bad DA record"), "{err}");
    }

    #[test]
    fn should_normalise_backslashes_when_a_path_is_made_relative() {
        // given
        let path = r"C:\r\crates\a\src\a.rs";

        // when
        let relative = relative_path(path, &[r"C:\r".to_owned()]);

        // then
        assert_eq!(relative.as_deref(), Some("crates/a/src/a.rs"));
        assert_eq!(relative_path("/rx/crates/a.rs", &["/r".to_owned()]), None);
    }

    #[test]
    fn should_require_llvm_cov_when_the_stage_is_built() {
        // given
        let stage = stage();

        // when
        let tool = stage.tool;

        // then
        assert_eq!(stage.name, "coverage");
        assert_eq!(tool, Some("cargo-llvm-cov"));
    }
}
