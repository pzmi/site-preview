//! The `commits` stage of `check` (ADR 0080): every commit message of a range follows the
//! repository's commit rules (CLAUDE.md "Commits").
//!
//! The range is `$COMMITS_BASE..HEAD`, or `origin/main..HEAD` when the variable is unset, empty
//! or the all-zeros SHA of a new branch; merge commits are left out. An empty range passes with
//! a note, so commits already on `main` are never checked again. Each message must have:
//!
//! - a subject `<ID>: summary` under 72 characters, where the ID is a task id as `doc/tasks.md`
//!   heads them (`FND-05`, `FORMAL-10`) and the summary starts with an imperative verb (`add`,
//!   not `adds`, `added` or `adding`);
//! - a blank line after the subject when there is a body;
//! - a `Signed-off-by:` line (DCO);
//! - a last paragraph that is the trailer block and nothing else: `Signed-off-by`, then any
//!   `Co-Authored-By`, then any `Claude-Session`, in that order. `git commit -s` on a message
//!   that already ends in trailers appends `Signed-off-by` after them, which is flagged.
//!
//! Only trailer keys and their order are checked; no value (an address, a URL) is.

use std::path::Path;
use std::process::Command;

use crate::process::Stage;
use crate::repo;

/// The environment variable that names the range's base revision.
pub const BASE_ENV: &str = "COMMITS_BASE";

/// The base when [`BASE_ENV`] names none.
const DEFAULT_BASE: &str = "origin/main";

/// A subject must be shorter than this, in characters.
const SUBJECT_LIMIT: usize = 72;

/// The trailer keys of the final block, in their required order (compared ignoring case).
const TRAILERS: [&str; 3] = ["Signed-off-by", "Co-Authored-By", "Claude-Session"];

/// First words that end like a non-imperative form but are imperative verbs.
const IMPERATIVE_EXCEPTIONS: &[&str] = &[
    "alias", "bias", "bring", "embed", "exceed", "feed", "need", "ping", "proceed", "seed", "shed",
    "speed", "string", "succeed",
];

/// What `check` and `list-checks` show for the stage.
const DISPLAY: &str = "git log --no-merges <$COMMITS_BASE, else origin/main>..HEAD: `<ID>: summary` under 72 characters, imperative; trailers Signed-off-by, Co-Authored-By, Claude-Session, in that order, last";

/// The `commits` stage.
pub fn stage() -> Stage {
    Stage::in_process("commits", DISPLAY, run)
}

fn run() -> i32 {
    let base = base_from(std::env::var(BASE_ENV).ok().as_deref());
    match check_range(&repo::root(), &base) {
        Ok(report) => {
            for line in &report.lines {
                eprintln!("{line}");
            }
            i32::from(report.failed)
        }
        Err(err) => {
            eprintln!("xtask: commits: {err}");
            1
        }
    }
}

/// The base revision: `env` unless it is unset, blank or all zeros (a push that created the
/// branch), else [`DEFAULT_BASE`].
fn base_from(env: Option<&str>) -> String {
    match env.map(str::trim) {
        Some(base) if !base.is_empty() && !base.bytes().all(|b| b == b'0') => base.to_owned(),
        _ => DEFAULT_BASE.to_owned(),
    }
}

/// What the stage prints, and whether it failed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    lines: Vec<String>,
    failed: bool,
}

/// Checks every non-merge commit of `base..HEAD` in the repository at `root`.
fn check_range(root: &Path, base: &str) -> Result<Report, String> {
    if base.starts_with('-') {
        return Err(format!("{BASE_ENV} `{base}` is not a revision"));
    }
    let commits = messages(root, base)?;
    let mut report = Report::default();
    if commits.is_empty() {
        report.lines.push(format!(
            "commits: no commits in {base}..HEAD; nothing to check"
        ));
        return Ok(report);
    }
    for (sha, message) in &commits {
        let short = &sha[..sha.len().min(12)];
        for problem in problems(message) {
            report.lines.push(format!("commits: {short}: {problem}"));
            report.failed = true;
        }
    }
    if !report.failed {
        report.lines.push(format!(
            "commits: {} commit(s) in {base}..HEAD follow the commit rules",
            commits.len()
        ));
    }
    Ok(report)
}

/// The SHA and full message of each non-merge commit of `base..HEAD`, newest first.
fn messages(root: &Path, base: &str) -> Result<Vec<(String, String)>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "log",
            "-z",
            "--no-merges",
            "--format=%H%n%B",
            "--end-of-options",
        ])
        .arg(format!("{base}..HEAD"))
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git log {base}..HEAD failed (set {BASE_ENV} to a revision this clone has): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter_map(|record| {
            let (sha, message) = record.trim_start_matches('\n').split_once('\n')?;
            Some((sha.to_owned(), message.to_owned()))
        })
        .collect())
}

/// Everything wrong with one commit message; empty when it follows the rules.
pub fn problems(message: &str) -> Vec<String> {
    let lines: Vec<&str> = message.trim_end().lines().collect();
    let subject = lines.first().copied().unwrap_or_default();
    let mut out = subject_problems(subject);
    if lines.get(1).is_some_and(|line| !line.trim().is_empty()) {
        out.push("put a blank line between the subject and the body".to_owned());
    }
    out.extend(trailer_problems(&lines));
    out
}

/// The subject's problems: shape, task id, length and mood.
fn subject_problems(subject: &str) -> Vec<String> {
    let mut out = Vec::new();
    let length = subject.chars().count();
    if length >= SUBJECT_LIMIT {
        out.push(format!(
            "the subject is {length} characters; keep it under {SUBJECT_LIMIT}"
        ));
    }
    let Some((id, summary)) = subject.split_once(": ") else {
        out.push(format!(
            "the subject `{subject}` is not `<ID>: summary` (a task id such as FND-05)"
        ));
        return out;
    };
    if !is_task_id(id) {
        out.push(format!(
            "`{id}` is not a task id (`doc/tasks.md` heads them like FND-05 or FORMAL-10)"
        ));
    }
    match summary.split_whitespace().next() {
        None => out.push("the subject has no summary after the task id".to_owned()),
        Some(word) if !is_imperative(word) => out.push(format!(
            "the summary starts with `{word}`; use the imperative (`add`, not `adds`, `added` or `adding`)"
        )),
        Some(_) => {}
    }
    out
}

/// `LANE-NN`: an upper-case lane name (letters, then letters or digits), a hyphen and at
/// least two digits.
fn is_task_id(id: &str) -> bool {
    let Some((lane, number)) = id.split_once('-') else {
        return false;
    };
    lane.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && lane
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && number.len() >= 2
        && number.chars().all(|c| c.is_ascii_digit())
}

/// A heuristic for the mood of the first word: it rejects the forms an indicative or
/// progressive subject starts with (`adds`, `fixes`, `added`, `adding`), keeping the
/// imperative verbs that merely end alike (`address`, `focus`, `embed`, `bring`).
fn is_imperative(word: &str) -> bool {
    let word = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if IMPERATIVE_EXCEPTIONS.contains(&word.as_str()) {
        return true;
    }
    let third_person =
        word.ends_with('s') && !["ss", "us", "is"].iter().any(|end| word.ends_with(end));
    !(word.ends_with("ing") || word.ends_with("ed") || third_person)
}

/// The key of a `Key: value` line, if the line is one.
fn key_of(line: &str) -> Option<&str> {
    let (key, _) = line.split_once(':')?;
    let key = key.trim();
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some(key)
}

/// The trailer block's problems. `lines` is the whole message, without trailing blank lines.
fn trailer_problems(lines: &[&str]) -> Vec<String> {
    let rank = |key: &str| TRAILERS.iter().position(|t| t.eq_ignore_ascii_case(key));
    let signed = |line: &&str| key_of(line).is_some_and(|k| rank(k) == Some(0));
    if !lines.iter().any(signed) {
        return vec!["no `Signed-off-by:` line: commit with `git commit -s` (DCO)".to_owned()];
    }
    let start = lines
        .iter()
        .rposition(|line| line.trim().is_empty())
        .map_or(lines.len(), |blank| blank + 1);
    let block = &lines[start..];
    if !block.iter().any(signed) {
        return vec![
            "the trailer block is not last: the final paragraph has no `Signed-off-by:`".to_owned(),
        ];
    }
    let mut out = Vec::new();
    let mut previous: Option<(usize, &str)> = None;
    for line in block {
        let Some((key, position)) = key_of(line).and_then(|k| rank(k).map(|r| (k, r))) else {
            out.push(format!(
                "`{line}` is in the trailer block, which holds only {}",
                TRAILERS.join(", ")
            ));
            continue;
        };
        if let Some((before, before_key)) = previous
            && position < before
        {
            let why = if position == 0 {
                " (`git commit -s` appends it after trailers already in the message)"
            } else {
                ""
            };
            out.push(format!(
                "`{key}` comes after `{before_key}`{why}; the order is {}",
                TRAILERS.join(", then ")
            ));
        }
        if previous.is_none_or(|(before, _)| position >= before) {
            previous = Some((position, key));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAILER_BLOCK: &str = "Signed-off-by: A Person <a@example.com>\nCo-Authored-By: B <b@example.com>\nClaude-Session: https://example.com/s";

    fn message(subject: &str, trailers: &str) -> String {
        format!("{subject}\n\nWhy it changed.\n\n{trailers}\n")
    }

    #[test]
    fn should_accept_a_message_when_it_follows_every_rule() {
        // given
        let text = message("FND-05: add the commits stage", TRAILER_BLOCK);

        // when
        let found = problems(&text);

        // then
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn should_accept_the_optional_trailers_when_they_are_absent() {
        // given
        let text = "FORMAL-10: pin the seed\n\nSigned-off-by: A <a@example.com>\n";

        // when
        let found = problems(text);

        // then
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn should_flag_signed_off_by_when_it_follows_co_authored_by() {
        // given
        let text = message(
            "FND-05: add the commits stage",
            "Co-Authored-By: B <b@example.com>\nClaude-Session: https://example.com/s\nSigned-off-by: A <a@example.com>",
        );

        // when
        let found = problems(&text);

        // then
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("`Signed-off-by` comes after `Claude-Session`")
                && found[0].contains("git commit -s"),
            "{found:?}"
        );
    }

    #[test]
    fn should_flag_claude_session_when_it_precedes_co_authored_by() {
        // given
        let text = message(
            "FND-05: add the commits stage",
            "Signed-off-by: A <a@example.com>\nClaude-Session: x\nCo-Authored-By: B <b@example.com>",
        );

        // when
        let found = problems(&text);

        // then
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("`Co-Authored-By` comes after `Claude-Session`"));
    }

    #[test]
    fn should_require_a_sign_off_when_none_is_present() {
        // given
        let text = "FND-05: add the commits stage\n\nCo-Authored-By: B <b@example.com>\n";

        // when
        let found = problems(text);

        // then
        assert_eq!(
            found,
            ["no `Signed-off-by:` line: commit with `git commit -s` (DCO)"]
        );
    }

    #[test]
    fn should_flag_the_trailer_block_when_a_paragraph_follows_it() {
        // given
        let text =
            "FND-05: add the commits stage\n\nSigned-off-by: A <a@example.com>\n\nA late note.\n";

        // when
        let found = problems(text);

        // then
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("not last"), "{found:?}");
    }

    #[test]
    fn should_flag_another_line_when_it_is_inside_the_trailer_block() {
        // given
        let text =
            "FND-05: add the commits stage\n\nImpact: none\nSigned-off-by: A <a@example.com>\n";

        // when
        let found = problems(text);

        // then
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("`Impact: none` is in the trailer block"));
    }

    #[test]
    fn should_reject_the_subject_when_it_has_no_task_id() {
        // given
        let subjects = [
            "add the commits stage",
            "fnd-05: add the commits stage",
            "FND-5: add the commits stage",
            "CLAUDE.md: clarify trailers",
        ];

        // when
        let found: Vec<Vec<String>> = subjects.iter().map(|s| subject_problems(s)).collect();

        // then
        assert!(found[0][0].contains("is not `<ID>: summary`"), "{found:?}");
        for problems in &found[1..] {
            assert!(problems[0].contains("is not a task id"), "{problems:?}");
        }
    }

    #[test]
    fn should_reject_the_subject_when_it_is_72_characters_or_longer() {
        // given
        let at_limit = format!("FND-05: add {}", "x".repeat(72 - 12));
        let under = format!("FND-05: add {}", "x".repeat(71 - 12));

        // when
        let (long, short) = (subject_problems(&at_limit), subject_problems(&under));

        // then
        assert_eq!(at_limit.chars().count(), 72);
        assert_eq!(long, ["the subject is 72 characters; keep it under 72"]);
        assert!(short.is_empty(), "{short:?}");
    }

    #[test]
    fn should_reject_the_summary_when_it_is_not_imperative() {
        // given
        let words = ["adds", "fixes", "added", "adding", "Updated"];

        // when
        let moods: Vec<bool> = words.iter().map(|w| is_imperative(w)).collect();

        // then
        assert!(moods.iter().all(|imperative| !imperative), "{moods:?}");
        let found = subject_problems("FND-05: adds the stage");
        assert!(found[0].contains("use the imperative"), "{found:?}");
    }

    #[test]
    fn should_accept_imperative_verbs_when_they_end_like_other_forms() {
        // given
        let words = [
            "add", "address", "focus", "embed", "bring", "Pin", "replay", "process",
        ];

        // when
        let moods: Vec<bool> = words.iter().map(|w| is_imperative(w)).collect();

        // then
        assert!(moods.iter().all(|imperative| *imperative), "{moods:?}");
    }

    #[test]
    fn should_flag_a_body_when_no_blank_line_follows_the_subject() {
        // given
        let text = "FND-05: add the stage\nNo blank line.\n\nSigned-off-by: A <a@example.com>\n";

        // when
        let found = problems(text);

        // then
        assert_eq!(found, ["put a blank line between the subject and the body"]);
    }

    #[test]
    fn should_use_origin_main_when_the_base_is_unset_blank_or_zeros() {
        // given
        let zeros = "0".repeat(40);

        // when
        let bases = [
            base_from(None),
            base_from(Some(" ")),
            base_from(Some(&zeros)),
            base_from(Some("abc123")),
        ];

        // then
        assert_eq!(bases, [DEFAULT_BASE, DEFAULT_BASE, DEFAULT_BASE, "abc123"]);
    }

    /// A repository in a temporary directory with one commit per message, in order.
    fn repository(messages: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .expect("git");
            assert!(status.status.success(), "{status:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "base: not checked"]);
        git(&["tag", "base"]);
        for text in messages {
            git(&[
                "commit",
                "-q",
                "--allow-empty",
                "--cleanup=verbatim",
                "-m",
                text,
            ]);
        }
        dir
    }

    #[test]
    fn should_pass_with_a_note_when_the_range_is_empty() {
        // given
        let repo = repository(&[]);

        // when
        let report = check_range(repo.path(), "base").unwrap();

        // then
        assert!(!report.failed);
        assert_eq!(
            report.lines,
            ["commits: no commits in base..HEAD; nothing to check"]
        );
    }

    #[test]
    fn should_fail_naming_the_commit_when_one_message_breaks_a_rule() {
        // given
        let good = message("FND-05: add the stage", TRAILER_BLOCK);
        let repo = repository(&[
            &good,
            "FND-05: added the stage\n\nSigned-off-by: A <a@example.com>\n",
        ]);

        // when
        let report = check_range(repo.path(), "base").unwrap();

        // then
        assert!(report.failed);
        assert_eq!(report.lines.len(), 1, "{:?}", report.lines);
        assert!(
            report.lines[0].contains("use the imperative"),
            "{:?}",
            report.lines
        );
    }

    #[test]
    fn should_pass_when_every_commit_of_the_range_follows_the_rules() {
        // given
        let good = message("FND-05: add the stage", TRAILER_BLOCK);
        let repo = repository(&[&good, &good]);

        // when
        let report = check_range(repo.path(), "base").unwrap();

        // then
        assert!(!report.failed, "{:?}", report.lines);
        assert_eq!(
            report.lines,
            ["commits: 2 commit(s) in base..HEAD follow the commit rules"]
        );
    }

    #[test]
    fn should_fail_closed_when_the_base_is_not_a_revision() {
        // given
        let repo = repository(&[]);

        // when
        let (unknown, option) = (
            check_range(repo.path(), "no-such-rev"),
            check_range(repo.path(), "--all"),
        );

        // then
        assert!(unknown.unwrap_err().contains(BASE_ENV));
        assert!(option.unwrap_err().contains("is not a revision"));
    }
}
