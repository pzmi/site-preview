//! The action-pinning check of `cargo xtask security` (foundation.md section 8, D23): every
//! `uses:` in a workflow or local action must name a full commit SHA, so a moved tag cannot
//! change what CI runs. Local actions (`./path`) and digest-pinned `docker://` images pass.

use std::fs;
use std::path::{Path, PathBuf};

/// Directories scanned for workflow and action files, relative to the repository root.
const SCANNED: &[&str] = &[".github/workflows", ".github/actions"];

/// One `uses:` reference that is not pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpinned {
    /// 1-based line number.
    pub line: usize,
    /// The reference as written.
    pub reference: String,
}

/// The `uses:` value on `line`, if it has one.
fn uses_value(line: &str) -> Option<&str> {
    let rest = line
        .trim_start()
        .strip_prefix("- ")
        .unwrap_or(line.trim_start());
    let value = rest.trim_start().strip_prefix("uses:")?.trim();
    let value = value.split(" #").next().unwrap_or(value).trim();
    Some(value.trim_matches(['"', '\'']))
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_pinned(reference: &str) -> bool {
    if reference.starts_with("./") {
        return true;
    }
    if let Some(image) = reference.strip_prefix("docker://") {
        return image
            .rsplit_once("@sha256:")
            .is_some_and(|(_, d)| is_hex(d, 64));
    }
    reference
        .rsplit_once('@')
        .is_some_and(|(name, sha)| name.contains('/') && is_hex(sha, 40))
}

/// Every unpinned `uses:` in one file's text.
pub fn unpinned(text: &str) -> Vec<Unpinned> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let reference = uses_value(line)?;
            (!is_pinned(reference)).then(|| Unpinned {
                line: i + 1,
                reference: reference.to_owned(),
            })
        })
        .collect()
}

fn yaml_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.is_dir() {
            yaml_files(&path, out)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        ) {
            out.push(path);
        }
    }
    Ok(())
}

/// Checks the files under `root`; returns one message per unpinned reference.
pub fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for dir in SCANNED {
        yaml_files(&root.join(dir), &mut files)?;
    }
    files.sort();
    let mut problems = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for u in unpinned(&text) {
            problems.push(format!(
                "{shown}:{}: '{}' is not pinned to a full commit SHA",
                u.line, u.reference
            ));
        }
    }
    Ok(problems)
}

/// The in-process stage body: 0 when every reference is pinned.
pub fn run() -> i32 {
    match check(Path::new(".")) {
        Ok(problems) if problems.is_empty() => 0,
        Ok(problems) => {
            for p in problems {
                eprintln!("xtask: {p}");
            }
            1
        }
        Err(err) => {
            eprintln!("xtask: action pins: {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "3d3c42e5aac5ba805825da76410c181273ba90b1";

    #[test]
    fn should_report_reference_when_action_uses_a_tag() {
        // given
        let text = "steps:\n  - uses: actions/checkout@v4\n";

        // when
        let found = unpinned(text);

        // then
        assert_eq!(
            found,
            vec![Unpinned {
                line: 2,
                reference: "actions/checkout@v4".into()
            }]
        );
    }

    #[test]
    fn should_pass_when_action_is_pinned_to_a_full_sha_with_comment() {
        // given
        let text = format!("      - uses: actions/checkout@{SHA} # v7.0.1\n");

        // when
        let found = unpinned(&text);

        // then
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn should_report_reference_when_sha_is_abbreviated_or_missing() {
        // given
        let text = "uses: actions/cache@3d3c42e\nuses: actions/cache\n";

        // when
        let found = unpinned(text);

        // then
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn should_pass_when_action_is_local_or_digest_pinned_image() {
        // given
        let digest = "a".repeat(64);
        let text = format!("uses: ./.github/actions/x\nuses: docker://alpine@sha256:{digest}\n");

        // when
        let found = unpinned(&text);

        // then
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn should_report_docker_image_when_it_is_only_tagged() {
        // given
        let text = "uses: docker://alpine:3.20\n";

        // when
        let found = unpinned(text);

        // then
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn should_report_file_and_line_when_workflow_on_disk_is_unpinned() {
        // given
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join(".github/workflows");
        fs::create_dir_all(&dir).expect("create dir");
        fs::write(
            dir.join("a.yml"),
            "jobs:\n  j:\n    steps:\n      - uses: actions/checkout@v4\n",
        )
        .expect("write");

        // when
        let problems = check(temp.path()).expect("scan");

        // then
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains(".github/workflows/a.yml:4:"),
            "{problems:?}"
        );
    }

    #[test]
    fn should_pass_when_repository_workflows_are_pinned() {
        // given
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("root");

        // when
        let problems = check(root).expect("scan");

        // then
        assert!(problems.is_empty(), "{problems:?}");
    }
}
