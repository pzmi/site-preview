//! `cargo xtask coverage --changed-since [REV]` and the `changed-lines` stage of `check`: at
//! least 95% of the instrumented lines a change adds or modifies in `crates/**/src/**/*.rs`
//! must be covered (foundation.md section 5.3, ADR 0018, ADR 0064).
//!
//! **The change** is `git diff -U0 <base>` against the working tree, plus every untracked,
//! non-ignored file under `crates/` (all its lines are new), so uncommitted work is gated
//! before it is committed. `<base>` is the merge-base of `REV` and `HEAD`, so commits that
//! landed on `REV` after this branch left it never count as this change. With no `REV` it is
//! the merge-base with `origin/main`, or `HEAD~1` when `origin/main` is absent or already
//! contains `HEAD` (a push to `main`, where the last commit is what changed). In `check`, a
//! commit id in [`BASE_ENV`] (CI sets it to the commit a push started from) takes the place of
//! `REV`, so every commit of a push counts (ADR 0069). Renames are detected, so a moved file
//! counts only its edited lines; deleted lines count for nothing. Untracked files are ignored
//! by the committed `.gitignore` files only, never by `.git/info/exclude` or a personal
//! excludes file, so every machine gates the same files.
//!
//! **The join** reads the LCOV `DA:` records of `target/coverage/lcov.info` per
//! repository-relative file (`coverage::parse_lcov_lines`). A changed line with a record is
//! instrumented, and covered when its hit count is non-zero; a changed line with no record
//! (a comment, a blank line, an item signature, a file the build never compiles) is not
//! instrumented, is left out of the ratio and is counted apart. No instrumented changed line
//! passes. The ratio is integer arithmetic, so 94.99% fails.
//!
//! **Freshness:** the report is used only while `target/coverage/lcov.stamp` matches
//! [`tree_fingerprint`] of the tree now. The command measures again otherwise; the `check`
//! stage, which runs right after `coverage` wrote the report, fails instead of measuring twice.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::coverage::{self, FileLines, LCOV_PATH};
use crate::process::Stage;
use crate::{repo, tools};

/// Percent of instrumented changed lines that must be covered (foundation.md section 5.3).
const THRESHOLD: u64 = 95;

/// The default comparison branch.
const DEFAULT_REV: &str = "origin/main";

/// The pathspec the diff is limited to; [`is_gated`] narrows it further.
const PATHSPEC: &str = "crates/";

/// The environment variable through which CI names the commit a push started from.
pub const BASE_ENV: &str = "CHANGED_LINES_BASE";

/// Changed line numbers per repository-relative file.
pub type Changed = BTreeMap<String, BTreeSet<u32>>;

/// The `changed-lines` stage, as `check` registers it.
pub fn stage() -> Stage {
    Stage::in_process(
        "changed-lines",
        "git diff -U0 <merge-base with $CHANGED_LINES_BASE, else origin/main, else HEAD~1> over crates/**/src/**/*.rs, joined with target/coverage/lcov.info: >=95% of instrumented changed lines covered",
        run_in_check,
    )
}

/// Whether a repository-relative path matches `crates/**/src/**/*.rs`.
pub fn is_gated(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() >= 4
        && parts[0] == "crates"
        && path.ends_with(".rs")
        && parts[2..parts.len() - 1].contains(&"src")
}

/// The new-side start and length of a hunk header body (`-a,b +c,d @@ ...`).
fn parse_hunk(header: &str) -> Option<(u32, u32)> {
    let plus = header
        .split(' ')
        .find_map(|token| token.strip_prefix('+'))?;
    match plus.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((plus.parse().ok()?, 1)),
    }
}

/// The added and modified lines of a `git diff -U0` with `a/` and `b/` prefixes, for gated
/// files only. A `+++` line is read only in a file's header, before its first hunk, so an added
/// line whose text starts with `++ ` is never taken for a file name.
pub fn parse_diff(text: &str) -> Result<Changed, String> {
    let mut changed = Changed::new();
    let mut current: Option<String> = None;
    let mut in_header = false;
    for (number, line) in text.lines().enumerate() {
        if line.starts_with("diff --git ") {
            current = None;
            in_header = true;
        } else if in_header && let Some(path) = line.strip_prefix("+++ ") {
            current = path
                .strip_prefix("b/")
                .filter(|p| is_gated(p))
                .map(str::to_owned);
        } else if let Some(header) = line.strip_prefix("@@ ") {
            in_header = false;
            let Some(file) = &current else { continue };
            let (start, count) = parse_hunk(header)
                .ok_or_else(|| format!("diff line {}: bad hunk header {line:?}", number + 1))?;
            if count > 0 {
                changed
                    .entry(file.clone())
                    .or_default()
                    .extend(start..start.saturating_add(count));
            }
        }
    }
    Ok(changed)
}

/// The changed lines joined with the coverage report.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Every changed line of a gated file.
    pub changed: u64,
    /// Of those, lines with a `DA:` record.
    pub instrumented: u64,
    /// Of those, lines executed at least once.
    pub covered: u64,
    /// Instrumented changed lines never executed, as `(file, line)`.
    pub uncovered: Vec<(String, u32)>,
}

impl Verdict {
    /// Changed lines with no `DA:` record, left out of the ratio.
    pub fn not_instrumented(&self) -> u64 {
        self.changed - self.instrumented
    }

    /// Whether `covered / instrumented` reaches the threshold, in integers.
    pub fn passes(&self) -> bool {
        self.covered * 100 >= self.instrumented * THRESHOLD
    }
}

/// Joins changed lines with per-file LCOV hits.
pub fn join(changed: &Changed, lines: &FileLines) -> Verdict {
    let mut verdict = Verdict::default();
    for (file, numbers) in changed {
        let hits = lines.get(file);
        for &number in numbers {
            verdict.changed += 1;
            let Some(&count) = hits.and_then(|h| h.get(&number)) else {
                continue;
            };
            verdict.instrumented += 1;
            if count > 0 {
                verdict.covered += 1;
            } else {
                verdict.uncovered.push((file.clone(), number));
            }
        }
    }
    verdict
}

/// The printed report.
pub fn report(verdict: &Verdict, base: &str) -> String {
    let mut out = format!("changed-lines: base {base}, files crates/**/src/**/*.rs\n");
    let _ = writeln!(
        out,
        "changed lines: {} ({} instrumented, {} not instrumented and left out)",
        verdict.changed,
        verdict.instrumented,
        verdict.not_instrumented()
    );
    if verdict.instrumented == 0 {
        out.push_str("no instrumented changed line: nothing to gate\n");
        return out;
    }
    let percent = verdict.covered as f64 * 100.0 / verdict.instrumented as f64;
    let status = if verdict.passes() { "ok" } else { "BELOW" };
    let _ = writeln!(
        out,
        "covered: {}/{} ({percent:.2}%), threshold {THRESHOLD}%: {status}",
        verdict.covered, verdict.instrumented
    );
    if !verdict.uncovered.is_empty() {
        out.push_str("uncovered:\n");
        for (file, line) in &verdict.uncovered {
            let _ = writeln!(out, "  {file}:{line}");
        }
    }
    out
}

/// Runs `git -C dir args`, feeding `input` on stdin, and returns its standard output.
fn git_bytes(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    // Feed stdin from its own thread: git may write output before it has read all its input,
    // and a full stdout pipe nobody drains would block both sides.
    let stdin = child.stdin.take();
    let out = std::thread::scope(|scope| {
        let writer = scope.spawn(move || match (input, stdin) {
            (Some(input), Some(mut stdin)) => stdin.write_all(input),
            _ => Ok(()),
        });
        let out = child.wait_with_output();
        writer
            .join()
            .map_err(|_| "the thread writing to git panicked".to_owned())?
            .map_err(|e| format!("cannot write to git: {e}"))?;
        out.map_err(|e| format!("cannot run git: {e}"))
    })?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Runs `git -C dir args` and returns its standard output as text.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    git_bytes(dir, args, None).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// The commit `rev` names, or an error.
fn commit_of(dir: &Path, rev: &str) -> Result<String, String> {
    git(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .map(|s| s.trim().to_owned())
}

/// The commit the diff starts from, and how it was chosen.
#[derive(Debug, PartialEq, Eq)]
pub struct Base {
    /// The commit id.
    pub commit: String,
    /// What the report prints.
    pub label: String,
}

/// Resolves the base: the merge-base of `rev` (default `origin/main`) and `HEAD`; with no
/// `rev`, `HEAD~1` when `origin/main` is absent or already contains `HEAD`.
pub fn resolve_base(dir: &Path, rev: Option<&str>) -> Result<Base, String> {
    let head = commit_of(dir, "HEAD").map_err(|_| "HEAD names no commit".to_owned())?;
    let merge_base = |rev: &str| -> Result<String, String> {
        let commit = commit_of(dir, rev)?;
        git(dir, &["merge-base", &commit, &head]).map(|s| s.trim().to_owned())
    };
    let short = |commit: &str| commit.chars().take(12).collect::<String>();
    if let Some(rev) = rev {
        let commit = merge_base(rev)
            .map_err(|_| format!("--changed-since {rev}: no common commit with HEAD"))?;
        let label = format!("{rev} (merge-base {})", short(&commit));
        return Ok(Base { commit, label });
    }
    if let Ok(commit) = merge_base(DEFAULT_REV)
        && commit != head
    {
        let label = format!("{DEFAULT_REV} (merge-base {})", short(&commit));
        return Ok(Base { commit, label });
    }
    let commit = commit_of(dir, "HEAD~1").map_err(|_| {
        format!(
            "no base revision: {DEFAULT_REV} is absent or already contains HEAD, and HEAD~1 \
             does not exist (a shallow clone needs the full history: fetch-depth: 0)"
        )
    })?;
    let label = format!("HEAD~1 ({})", short(&commit));
    Ok(Base { commit, label })
}

/// The changed lines of gated files between `base` and the working tree, untracked files
/// included.
pub fn changed_since(dir: &Path, base: &str) -> Result<Changed, String> {
    let diff = git(
        dir,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--diff-filter=d",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            base,
            "--",
            PATHSPEC,
        ],
    )?;
    let mut changed = parse_diff(&diff)?;
    let untracked = git(
        dir,
        &[
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-per-directory=.gitignore",
            "--",
            PATHSPEC,
        ],
    )?;
    for path in untracked.lines().filter(|p| is_gated(p)) {
        let text = std::fs::read_to_string(dir.join(path))
            .map_err(|e| format!("cannot read {path}: {e}"))?;
        let count = u32::try_from(text.lines().count())
            .map_err(|_| format!("{path} has too many lines"))?;
        if count > 0 {
            changed
                .entry(path.to_owned())
                .or_default()
                .extend(1..=count);
        }
    }
    Ok(changed)
}

/// A fingerprint of the source tree's content: every tracked or untracked file that exists and
/// no committed `.gitignore` ignores, with its path and git blob id, hashed by git. It depends on content only, not
/// on `HEAD`, so committing a measured tree keeps its report fresh, and two trees with equal
/// fingerprints build and test the same. `target/` and other ignored paths do not count; a
/// symbolic link counts by its target. Nothing is written to the object store.
pub fn tree_fingerprint(dir: &Path) -> Result<String, String> {
    let listing = git_bytes(
        dir,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-per-directory=.gitignore",
        ],
        None,
    )?;
    let mut files = BTreeSet::new();
    let mut links = BTreeMap::new();
    for path in listing.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8(path.to_vec())
            .map_err(|_| "a file name is not UTF-8; it cannot be fingerprinted".to_owned())?;
        if path.contains('\n') {
            return Err(format!("{path:?} has a newline in its name"));
        }
        match std::fs::symlink_metadata(dir.join(&path)) {
            Ok(meta) if meta.is_symlink() => {
                let target = std::fs::read_link(dir.join(&path))
                    .map_err(|e| format!("cannot read link {path}: {e}"))?;
                links.insert(path, target.display().to_string());
            }
            Ok(meta) if meta.is_file() => {
                files.insert(path);
            }
            // Deleted but still in the index, or a submodule directory: not content here.
            _ => {}
        }
    }
    let paths: String = files.iter().map(|p| format!("{p}\n")).collect();
    let mut input = paths.clone().into_bytes();
    input.push(0);
    if !files.is_empty() {
        input.extend(git_bytes(
            dir,
            &["hash-object", "--no-filters", "--stdin-paths"],
            Some(paths.as_bytes()),
        )?);
    }
    for (path, target) in &links {
        input.extend(format!("\0{path} -> {target}").into_bytes());
    }
    let id = git_bytes(dir, &["hash-object", "--stdin"], Some(&input))?;
    Ok(String::from_utf8_lossy(&id).trim().to_owned())
}

/// The roots an LCOV `SF:` path may start with: the root as given and its canonical form.
fn roots(dir: &Path) -> Vec<String> {
    let mut roots = vec![dir.display().to_string()];
    if let Ok(canonical) = dir.canonicalize() {
        let canonical = canonical.display().to_string();
        if !roots.contains(&canonical) {
            roots.push(canonical);
        }
    }
    roots
}

/// The whole gate over one LCOV text: base, diff, join. Returns the report and the verdict.
pub fn gate(dir: &Path, rev: Option<&str>, lcov: &str) -> Result<(String, bool), String> {
    let base = resolve_base(dir, rev)?;
    let changed = changed_since(dir, &base.commit)?;
    let lines = coverage::parse_lcov_lines(lcov, &roots(dir))?;
    if !changed.is_empty() && !lines.keys().any(|f| f.starts_with(PATHSPEC)) {
        return Err(format!(
            "{LCOV_PATH} names no file under {}/crates: it was measured in another checkout",
            dir.display()
        ));
    }
    let verdict = join(&changed, &lines);
    Ok((report(&verdict, &base.label), verdict.passes()))
}

/// Prints a gate result; returns the exit code.
fn print_outcome(result: Result<(String, bool), String>) -> i32 {
    match result {
        Ok((report, true)) => {
            print!("{report}");
            0
        }
        Ok((report, false)) => {
            print!("{report}");
            eprintln!(
                "xtask: fewer than {THRESHOLD}% of the instrumented changed lines are covered; \
                 test the lines listed"
            );
            1
        }
        Err(err) => {
            eprintln!("xtask: {err}");
            1
        }
    }
}

/// `cargo xtask coverage --changed-since [REV]`: reuses a fresh LCOV report, else measures;
/// returns the exit code.
pub fn command(rev: Option<&str>) -> i32 {
    let root = repo::root();
    let lcov = (|| match coverage::fresh_lcov(&root)? {
        Some(text) => {
            eprintln!("xtask: {LCOV_PATH} matches this tree; reusing it");
            Ok(text)
        }
        None => {
            eprintln!("xtask: {LCOV_PATH} is missing or stale for this tree; measuring");
            tools::require("cargo-llvm-cov")?;
            coverage::measure(&root)?;
            std::fs::read_to_string(root.join(LCOV_PATH))
                .map_err(|e| format!("cannot read {LCOV_PATH}: {e}"))
        }
    })();
    print_outcome(lcov.and_then(|text| gate(&root, rev, &text)))
}

/// The base `check` takes from [`BASE_ENV`]: a full commit id (40 or 64 lowercase hex digits)
/// that is not all zeros and names a commit sharing history with `HEAD`. Anything else gives
/// `None` and a note saying why, and the default base applies; a value that is not hex never
/// reaches git, so it cannot be read as an option.
pub fn env_base(dir: &Path, value: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return (None, None);
    };
    let fallback = |why: &str| {
        (
            None,
            Some(format!("{BASE_ENV} {why}; using the default base")),
        )
    };
    let is_hex = value
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !matches!(value.len(), 40 | 64) || !is_hex {
        return fallback("is not a full commit id");
    }
    if value.bytes().all(|b| b == b'0') {
        return fallback("is all zeros (a new branch)");
    }
    match resolve_base(dir, Some(value)) {
        Ok(_) => (Some(value.to_owned()), None),
        Err(_) => fallback(&format!(
            "{value} is not a commit of this clone that HEAD shares history with"
        )),
    }
}

/// The `check` stage body: the report `coverage` just wrote, never a new measurement.
fn run_in_check() -> i32 {
    let root = repo::root();
    let (base, note) = env_base(&root, std::env::var(BASE_ENV).ok().as_deref());
    if let Some(note) = note {
        eprintln!("xtask: {note}");
    }
    let result = coverage::fresh_lcov(&root).and_then(|lcov| {
        let lcov = lcov.ok_or_else(|| {
            format!(
                "{LCOV_PATH} does not match this tree: the coverage stage failed, or the tree \
                 changed during the gate; run `cargo xtask coverage --changed-since`"
            )
        })?;
        gate(&root, base.as_deref(), &lcov)
    });
    print_outcome(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command as CliCommand};
    use clap::Parser;
    use std::path::PathBuf;

    const DIFF: &str = "\
diff --git a/crates/preview-domain/src/a.rs b/crates/preview-domain/src/a.rs
index 1111111..2222222 100644
--- a/crates/preview-domain/src/a.rs
+++ b/crates/preview-domain/src/a.rs
@@ -3,0 +4,2 @@ fn f() {
+    let x = 1;
+++ y;
@@ -10 +12 @@ fn g() {
-    old();
+    new();
@@ -20,2 +21,0 @@ fn h() {
-    gone();
-    gone();
diff --git a/crates/preview-domain/tests/t.rs b/crates/preview-domain/tests/t.rs
--- a/crates/preview-domain/tests/t.rs
+++ b/crates/preview-domain/tests/t.rs
@@ -1 +1 @@
-a
+b
diff --git a/crates/preview-cli/src/gone.rs b/crates/preview-cli/src/gone.rs
deleted file mode 100644
--- a/crates/preview-cli/src/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-x
";

    fn changed(entries: &[(&str, &[u32])]) -> Changed {
        entries
            .iter()
            .map(|(f, l)| ((*f).to_owned(), l.iter().copied().collect()))
            .collect()
    }

    fn lines(entries: &[(&str, &[(u32, u64)])]) -> FileLines {
        entries
            .iter()
            .map(|(f, l)| ((*f).to_owned(), l.iter().copied().collect()))
            .collect()
    }

    const A: &str = "crates/preview-domain/src/a.rs";

    #[test]
    fn should_fail_when_changed_line_uncovered() {
        // given: one of two instrumented changed lines never ran
        let diff = changed(&[(A, &[4, 5])]);
        let lcov = lines(&[(A, &[(4, 3), (5, 0)])]);

        // when
        let verdict = join(&diff, &lcov);
        let text = report(&verdict, "HEAD~1");

        // then
        assert!(!verdict.passes());
        assert_eq!(verdict.uncovered, vec![(A.to_owned(), 5)]);
        assert!(text.contains(&format!("  {A}:5")), "{text}");
        assert!(text.contains("BELOW"), "{text}");
    }

    #[test]
    fn should_pass_when_changed_lines_covered() {
        // given
        let diff = changed(&[(A, &[4, 5])]);
        let lcov = lines(&[(A, &[(4, 1), (5, 7), (6, 0)])]);

        // when
        let verdict = join(&diff, &lcov);

        // then
        assert!(verdict.passes());
        assert_eq!((verdict.instrumented, verdict.covered), (2, 2));
        assert!(report(&verdict, "HEAD~1").contains("2/2 (100.00%)"));
    }

    #[test]
    fn should_ignore_non_instrumented_lines_when_computing_ratio() {
        // given: 19 covered and 1 uncovered instrumented lines (95%), plus 30 changed lines
        // with no DA record (comments, a file the build never compiles)
        let instrumented: Vec<u32> = (1..=20).collect();
        let comments: Vec<u32> = (21..=40).collect();
        let mut hits: Vec<(u32, u64)> = (1..=19).map(|l| (l, 1)).collect();
        hits.push((20, 0));
        let diff = changed(&[
            (A, &[instrumented, comments].concat()),
            (
                "crates/preview-cli/src/unbuilt.rs",
                &(1..=10).collect::<Vec<_>>(),
            ),
        ]);
        let lcov = lines(&[(A, &hits)]);

        // when
        let verdict = join(&diff, &lcov);
        let text = report(&verdict, "HEAD~1");

        // then
        assert_eq!(verdict.changed, 50);
        assert_eq!(verdict.instrumented, 20);
        assert_eq!(verdict.not_instrumented(), 30);
        assert!(verdict.passes(), "{text}");
        assert!(text.contains("30 not instrumented"), "{text}");
    }

    #[test]
    fn should_fail_when_just_below_the_threshold() {
        // given: 1899 of 2000 is 94.95%
        let verdict = Verdict {
            changed: 2000,
            instrumented: 2000,
            covered: 1899,
            uncovered: Vec::new(),
        };

        // when
        let passes = verdict.passes();

        // then
        assert!(!passes);
    }

    #[test]
    fn should_pass_and_say_so_when_no_changed_line_is_instrumented() {
        // given
        let verdict = join(&changed(&[(A, &[1])]), &FileLines::new());

        // when
        let text = report(&verdict, "HEAD~1");

        // then
        assert!(verdict.passes());
        assert!(text.contains("nothing to gate"), "{text}");
    }

    #[test]
    fn should_read_added_and_modified_lines_when_a_diff_is_parsed() {
        // given
        let text = DIFF;

        // when
        let parsed = parse_diff(text).unwrap();

        // then: lines 4-5 and 12; the deletion hunk, the tests/ file and the deleted file count
        // for nothing, and "+++ y;" inside a hunk is content, not a file name
        assert_eq!(parsed, changed(&[(A, &[4, 5, 12])]));
    }

    #[test]
    fn should_refuse_a_bad_hunk_header_when_a_diff_is_parsed() {
        // given
        let text = format!("diff --git a/{A} b/{A}\n+++ b/{A}\n@@ -1 +x @@\n");

        // when
        let err = parse_diff(&text).unwrap_err();

        // then
        assert!(err.contains("bad hunk header"), "{err}");
    }

    #[test]
    fn should_gate_only_crate_sources_when_paths_are_matched() {
        // given
        let cases = [
            (A, true),
            ("crates/preview-cli/src/bin/x/main.rs", true),
            ("crates/preview-cli/tests/cli.rs", false),
            ("crates/preview-cli/src/data.toml", false),
            ("tools/xtask/src/main.rs", false),
            ("crates/src.rs", false),
        ];

        // when
        let got: Vec<_> = cases.iter().map(|(p, _)| is_gated(p)).collect();

        // then
        let want: Vec<_> = cases.iter().map(|(_, w)| *w).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn should_parse_the_optional_rev_when_coverage_takes_changed_since() {
        // given
        let forms: [&[&str]; 3] = [
            &["xtask", "coverage"],
            &["xtask", "coverage", "--changed-since"],
            &["xtask", "coverage", "--changed-since", "HEAD~3"],
        ];

        // when
        let parsed: Vec<_> = forms
            .iter()
            .map(|args| match Cli::try_parse_from(*args).unwrap().command {
                CliCommand::Coverage { changed_since } => changed_since,
                other => panic!("{other:?}"),
            })
            .collect();

        // then
        assert_eq!(
            parsed,
            vec![None, Some(None), Some(Some("HEAD~3".to_owned()))]
        );
    }

    fn sh_git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn write(dir: &Path, path: &str, text: &str) {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn commit(dir: &Path, message: &str) -> String {
        sh_git(dir, &["add", "--all"]);
        sh_git(dir, &["commit", "--quiet", "-m", message]);
        sh_git(dir, &["rev-parse", "HEAD"])
    }

    /// A repository with `/target` ignored and one commit holding `A` with three lines.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        sh_git(&dir, &["init", "--quiet"]);
        write(&dir, ".gitignore", "/target\n");
        write(&dir, A, "fn a() {\n    one();\n}\n");
        commit(&dir, "c0");
        (temp, dir)
    }

    #[test]
    fn should_fall_back_to_the_parent_when_origin_main_is_absent() {
        // given
        let (_temp, dir) = repository();
        let first = sh_git(&dir, &["rev-parse", "HEAD"]);
        write(&dir, A, "fn a() {\n    two();\n}\n");
        commit(&dir, "c1");

        // when
        let base = resolve_base(&dir, None).unwrap();

        // then
        assert_eq!(base.commit, first);
        assert!(base.label.starts_with("HEAD~1"), "{}", base.label);
    }

    #[test]
    fn should_fall_back_to_the_parent_when_origin_main_contains_head() {
        // given
        let (_temp, dir) = repository();
        let first = sh_git(&dir, &["rev-parse", "HEAD"]);
        let head = {
            write(&dir, A, "fn a() {\n    two();\n}\n");
            commit(&dir, "c1")
        };
        sh_git(&dir, &["update-ref", "refs/remotes/origin/main", &head]);

        // when
        let base = resolve_base(&dir, None).unwrap();

        // then
        assert_eq!(base.commit, first);
    }

    #[test]
    fn should_use_the_merge_base_when_origin_main_has_moved_on() {
        // given: origin/main gained a commit after this branch left it
        let (_temp, dir) = repository();
        let fork = sh_git(&dir, &["rev-parse", "HEAD"]);
        sh_git(&dir, &["checkout", "--quiet", "-b", "other"]);
        write(&dir, "crates/preview-cli/src/x.rs", "fn x() {}\n");
        let remote = commit(&dir, "elsewhere");
        sh_git(&dir, &["update-ref", "refs/remotes/origin/main", &remote]);
        sh_git(&dir, &["checkout", "--quiet", "main"]);
        write(&dir, A, "fn a() {\n    two();\n}\n");
        commit(&dir, "mine");

        // when
        let base = resolve_base(&dir, None).unwrap();
        let lines = changed_since(&dir, &base.commit).unwrap();

        // then: x.rs, which only origin/main has, is not part of this change
        assert_eq!(base.commit, fork);
        assert!(base.label.starts_with("origin/main"), "{}", base.label);
        assert_eq!(lines, changed(&[(A, &[2])]));
    }

    #[test]
    fn should_refuse_when_no_base_exists() {
        // given: a single commit and no origin/main
        let (_temp, dir) = repository();

        // when
        let err = resolve_base(&dir, None).unwrap_err();
        let unknown = resolve_base(&dir, Some("no-such-rev")).unwrap_err();

        // then
        assert!(err.contains("fetch-depth: 0"), "{err}");
        assert!(unknown.contains("no-such-rev"), "{unknown}");
    }

    #[test]
    fn should_include_uncommitted_and_untracked_lines_when_changes_are_collected() {
        // given: an edit not yet committed and a new file not yet added
        let (_temp, dir) = repository();
        let base = sh_git(&dir, &["rev-parse", "HEAD"]);
        write(&dir, A, "fn a() {\n    one();\n    more();\n}\n");
        write(
            &dir,
            "crates/preview-cli/src/new.rs",
            "fn n() {}\n\nfn m() {}\n",
        );
        write(&dir, "crates/preview-cli/tests/new.rs", "fn t() {}\n");

        // when
        let lines = changed_since(&dir, &base).unwrap();

        // then
        assert_eq!(
            lines,
            changed(&[(A, &[3]), ("crates/preview-cli/src/new.rs", &[1, 2, 3])])
        );
    }

    #[test]
    fn should_take_the_pushed_from_commit_when_ci_names_one() {
        // given: the commit before two new ones
        let (_temp, dir) = repository();
        let before = sh_git(&dir, &["rev-parse", "HEAD"]);
        write(&dir, A, "fn a() {\n    uno();\n}\n");
        commit(&dir, "c1");
        write(&dir, A, "fn a() {\n    uno();\n    two();\n}\n");
        commit(&dir, "c2");

        // when
        let (base, note) = env_base(&dir, Some(&before));
        let lines = changed_since(&dir, base.as_deref().unwrap()).unwrap();

        // then: both commits of the push count, not only the last
        assert_eq!(base.as_deref(), Some(before.as_str()));
        assert_eq!(note, None);
        assert_eq!(lines, changed(&[(A, &[2, 3])]));
    }

    #[test]
    fn should_fall_back_to_the_default_when_ci_names_no_usable_commit() {
        // given
        let (_temp, dir) = repository();
        let missing = "1".repeat(40);

        // when
        let unset = env_base(&dir, None);
        let empty = env_base(&dir, Some(""));
        let new_branch = env_base(&dir, Some(&"0".repeat(40)));
        let option = env_base(&dir, Some("--output=/tmp/x"));
        let unknown = env_base(&dir, Some(&missing));

        // then
        assert_eq!(unset, (None, None));
        assert_eq!(empty, (None, None));
        for (base, note) in [new_branch, option, unknown] {
            assert_eq!(base, None);
            assert!(note.is_some_and(|n| n.contains(BASE_ENV)));
        }
    }

    #[test]
    fn should_gate_an_untracked_file_when_only_a_personal_exclude_hides_it() {
        // given: a new source file listed in .git/info/exclude
        let (_temp, dir) = repository();
        let base = sh_git(&dir, &["rev-parse", "HEAD"]);
        let info = PathBuf::from(sh_git(&dir, &["rev-parse", "--git-path", "info/exclude"]));
        let info = if info.is_absolute() {
            info
        } else {
            dir.join(info)
        };
        std::fs::create_dir_all(info.parent().unwrap()).unwrap();
        std::fs::write(&info, "crates/preview-cli/src/hidden.rs\n").unwrap();
        write(&dir, "crates/preview-cli/src/hidden.rs", "fn h() {}\n");

        // when
        let lines = changed_since(&dir, &base).unwrap();

        // then
        assert_eq!(
            lines,
            changed(&[("crates/preview-cli/src/hidden.rs", &[1])])
        );
    }

    #[test]
    fn should_change_the_fingerprint_when_the_tree_changes() {
        // given
        let (_temp, dir) = repository();
        let clean = tree_fingerprint(&dir).unwrap();

        // when
        let again = tree_fingerprint(&dir).unwrap();
        write(&dir, "target/coverage/lcov.info", "ignored\n");
        let ignored = tree_fingerprint(&dir).unwrap();
        write(&dir, "notes.txt", "untracked\n");
        let untracked = tree_fingerprint(&dir).unwrap();
        write(&dir, "notes.txt", "untracked, edited\n");
        let edited_untracked = tree_fingerprint(&dir).unwrap();
        write(&dir, A, "fn a() {}\n");
        let modified = tree_fingerprint(&dir).unwrap();
        commit(&dir, "c1");
        let committed = tree_fingerprint(&dir).unwrap();
        std::fs::remove_file(dir.join("notes.txt")).unwrap();
        let removed = tree_fingerprint(&dir).unwrap();

        // then: content decides, a commit does not
        assert_eq!(clean, again);
        assert_eq!(clean, ignored);
        assert_eq!(modified, committed);
        assert_ne!(removed, committed);
        let all = [&clean, &untracked, &edited_untracked, &modified];
        let distinct: BTreeSet<_> = all.iter().collect();
        assert_eq!(distinct.len(), all.len(), "{all:?}");
    }

    #[test]
    fn should_reuse_the_report_only_when_its_stamp_matches_the_tree() {
        // given
        let (_temp, dir) = repository();
        write(&dir, LCOV_PATH, "SF:x\nDA:1,1\nend_of_record\n");
        let missing = coverage::fresh_lcov(&dir).unwrap();
        let stamp = coverage::stamp_of(&dir, &tree_fingerprint(&dir).unwrap());
        write(&dir, "target/coverage/lcov.stamp", &stamp);

        // when
        let fresh = coverage::fresh_lcov(&dir).unwrap();
        write(&dir, A, "fn a() {}\n");
        let stale = coverage::fresh_lcov(&dir).unwrap();

        // then
        assert_eq!(missing, None);
        assert!(fresh.is_some_and(|t| t.contains("DA:1,1")));
        assert_eq!(stale, None);
    }

    #[test]
    fn should_list_the_uncovered_line_when_a_real_diff_meets_an_lcov_with_absolute_paths() {
        // given: line 3 added and covered, line 4 added and not covered
        let (_temp, dir) = repository();
        let base = sh_git(&dir, &["rev-parse", "HEAD"]);
        write(
            &dir,
            A,
            "fn a() {\n    one();\n    two();\n    three();\n}\n",
        );
        commit(&dir, "c1");
        let lcov = format!(
            "SF:{}/{A}\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nend_of_record\n",
            dir.display()
        );

        // when
        let (text, ok) = gate(&dir, Some(&base), &lcov).unwrap();

        // then
        assert!(!ok, "{text}");
        assert!(text.contains("1/2 (50.00%)"), "{text}");
        assert!(text.contains(&format!("{A}:4")), "{text}");
    }

    #[test]
    fn should_refuse_an_lcov_from_another_checkout_when_lines_changed() {
        // given
        let (_temp, dir) = repository();
        let base = sh_git(&dir, &["rev-parse", "HEAD"]);
        write(&dir, A, "fn a() {\n    two();\n}\n");
        let lcov = format!("SF:/elsewhere/{A}\nDA:2,1\nend_of_record\n");

        // when
        let err = gate(&dir, Some(&base), &lcov).unwrap_err();

        // then
        assert!(err.contains("another checkout"), "{err}");
    }

    #[test]
    fn should_name_the_stage_changed_lines_when_it_is_built() {
        // given
        let stage = stage();

        // when
        let name = stage.name;

        // then
        assert_eq!(name, "changed-lines");
        assert_eq!(stage.tool, None);
    }
}
