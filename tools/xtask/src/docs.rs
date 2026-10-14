//! The `docs` stage of `check`: the documentation says what the code does (DOC-05, ADR 0077).
//!
//! Each check compares a document with its source of truth and names the first thing that
//! drifted, as `path:line: message`:
//!
//! 1. ADRs: every `doc/adr/NNNN-*.md` is a row of `doc/adr/README.md` and vice versa, once, in
//!    ascending order, with the same title and status as the file; the number in the title and
//!    the link match the file name; no number is used twice; the status is one of `Proposed`,
//!    `Accepted` or `Superseded by NNNN` (an annotation such as `. Amended by 0069` may follow).
//! 2. Every rule id of the architecture catalog is in `doc/architecture-rules.md` and every
//!    `ARCH-*` id that document names is a rule.
//! 3. Every `cargo xtask <cmd>` in a code span or fence of any Markdown file is a command of
//!    this binary's clap surface.
//! 4. Every `serve` flag in the built binary's `--help` is in the README flags table and the
//!    table names no flag the binary lacks.
//! 5. Every warning code in `preview-domain`'s `warning_code` registry is in the README
//!    "Warnings" table and the table lists no other. The registry's `WarningCode` newtype has a
//!    private field, so only the registry can build the argument of `warn` and `warning_line`
//!    and the registry is the complete list without scanning call sites.
//! 6. Every relative Markdown link resolves, `#anchor`s into Markdown files included.
//! 7. The commands in `AGENTS.md` are the rows of the agent workflow spec's `EXERCISED` map
//!    and of its comment table.
//!
//! The negative fixtures under `tests/docs-invalid/` (each with an `expected.txt` listing the
//! exact diagnostics) and the positive ones under `tests/docs-valid/` run in the stage and in
//! the unit tests below; a check nothing can fail is not a check (foundation.md section 3.4).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use clap::CommandFactory;

use crate::cli::Cli;
use crate::process::Stage;
use crate::repo;

const ADR_DIR: &str = "doc/adr";
const ADR_INDEX: &str = "doc/adr/README.md";
const RULES_DOC: &str = "doc/architecture-rules.md";
const README: &str = "README.md";
const AGENTS: &str = "AGENTS.md";
const AGENT_SPEC: &str = "tests/e2e/browser/agent-workflow.spec.ts";
const WARNING_REGISTRY: &str = "crates/preview-domain/src/warning_code.rs";
const NEGATIVE_FIXTURES: &str = "tests/docs-invalid";
const POSITIVE_FIXTURES: &str = "tests/docs-valid";

/// The `docs` stage.
pub fn stage() -> Stage {
    Stage::in_process("docs", "cargo xtask check --only docs", run)
}

fn run() -> i32 {
    let root = repo::root();
    let mut problems = check_repository(&root);
    problems.extend(check_fixtures(&root));
    problems.sort();
    problems.dedup();
    for problem in &problems {
        eprintln!("docs: {problem}");
    }
    if problems.is_empty() { 0 } else { 1 }
}

/// Every check over the repository at `root`.
fn check_repository(root: &Path) -> Vec<String> {
    let mut problems = check_tree(root, &known_commands(), true);
    let read = |rel: &str| fs::read_to_string(root.join(rel)).map_err(|e| format!("{rel}: {e}"));
    let rules: Vec<&str> = crate::architecture::Rule::ALL
        .iter()
        .map(|rule| rule.id())
        .collect();
    match read(RULES_DOC) {
        Ok(doc) => problems.extend(check_rule_ids(&doc, &rules)),
        Err(err) => problems.push(err),
    }
    match (read(README), serve_help(root)) {
        (Ok(readme), Ok(help)) => problems.extend(check_serve_flags(&readme, &help)),
        (Err(err), _) | (_, Err(err)) => problems.push(err),
    }
    match (read(README), read(WARNING_REGISTRY)) {
        (Ok(readme), Ok(registry)) => {
            problems.extend(check_warning_codes(&readme, &registry));
        }
        (Err(err), _) | (_, Err(err)) => problems.push(err),
    }
    match (read(AGENTS), read(AGENT_SPEC)) {
        (Ok(agents), Ok(spec)) => problems.extend(check_agents_spec(&agents, &spec)),
        (Err(err), _) | (_, Err(err)) => problems.push(err),
    }
    problems
}

/// The checks that need only a tree of files: ADRs, xtask commands and links. Fixtures are
/// whole trees of this shape.
///
/// `require_adrs` is true for the repository itself, which must have `doc/adr` with at least
/// one ADR (a vanished directory must not pass as "nothing to check"); the fixture trees
/// that exercise other checks have none and pass false.
fn check_tree(root: &Path, known: &[String], require_adrs: bool) -> Vec<String> {
    let files = markdown_files(root);
    let mut problems = check_adrs(root, require_adrs);
    problems.extend(check_xtask_commands(root, &files, known));
    problems.extend(check_links(root, &files));
    problems.sort();
    problems
}

/// The subcommand names of `cargo xtask`, from the clap definition itself.
fn known_commands() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------

/// Directory names never searched: build output, dependencies, work state.
const SKIPPED_DIRS: &[&str] = &[".git", "target", "node_modules", ".work"];

/// Every `.md` file of the documentation under `root`, as sorted root-relative paths with `/`
/// separators. The fixture trees are not part of the repository's documentation.
///
/// In a git work tree the list is `git ls-files --cached --others --exclude-standard`: the
/// tracked files plus new files not yet added, minus everything ignored. Tracked-only would
/// let `check` pass before `git add` on a document that then breaks the next run; walking the
/// disk would read build output, scratch copies and other untracked clutter that git ignores
/// and that is not documentation. Outside git (the unit tests' temporary trees) it walks the
/// directory, skipping [`SKIPPED_DIRS`].
fn markdown_files(root: &Path) -> Vec<String> {
    let wanted = |rel: &str| {
        rel.ends_with(".md")
            && !rel.starts_with(&format!("{NEGATIVE_FIXTURES}/"))
            && !rel.starts_with(&format!("{POSITIVE_FIXTURES}/"))
    };
    let mut found = match git_files(root) {
        Some(listed) => listed
            .into_iter()
            .filter(|rel| wanted(rel) && root.join(rel).is_file())
            .collect(),
        None => {
            let mut walked = Vec::new();
            walk(root, root, &mut |rel| wanted(rel), &mut walked);
            walked
        }
    };
    found.sort();
    found.dedup();
    found
}

/// The files git knows under `root` (tracked, or untracked and not ignored), or `None` when
/// `root` is not inside a git work tree or git cannot run.
fn git_files(root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|rel| !rel.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// Collects root-relative files for which `keep` is true.
fn walk(root: &Path, dir: &Path, keep: &mut dyn FnMut(&str) -> bool, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                walk(root, &path, keep, out);
            }
        } else if kind.is_file() {
            let rel = relative(root, &path);
            if keep(&rel) {
                out.push(rel);
            }
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------------------
// Markdown scanning
// ---------------------------------------------------------------------------------------

/// A Markdown document split into what a reader sees as prose and what is code.
#[derive(Debug, Default)]
struct Parsed {
    /// `(line, text)` with code spans blanked to spaces; fenced lines are absent.
    prose: Vec<(usize, String)>,
    /// `(line, text)`: the lines inside fences and the contents of inline code spans.
    code: Vec<(usize, String)>,
    /// `(line, text)` of the ATX headings outside fences.
    headings: Vec<(usize, String)>,
}

fn parse_markdown(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut fence: Option<String> = None;
    for (index, line) in text.lines().enumerate() {
        let no = index + 1;
        let trimmed = line.trim_start();
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str())
                && trimmed.trim_end().chars().all(|c| open.starts_with(c))
            {
                fence = None;
            } else {
                parsed.code.push((no, line.to_owned()));
            }
            continue;
        }
        if let Some(marker) = fence_marker(trimmed) {
            fence = Some(marker);
            continue;
        }
        let (prose, spans) = blank_code_spans(line);
        parsed.code.extend(spans.into_iter().map(|s| (no, s)));
        if let Some(heading) = heading_text(line) {
            parsed.headings.push((no, heading));
        }
        parsed.prose.push((no, prose));
    }
    parsed
}

/// The opening fence of a code block (three or more backticks or tildes), if `line` is one.
fn fence_marker(line: &str) -> Option<String> {
    let first = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = line.chars().take_while(|c| *c == first).count();
    (run >= 3).then(|| first.to_string().repeat(run))
}

/// `line` with each inline code span replaced by spaces, and the spans' contents.
fn blank_code_spans(line: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|c| **c == '`').count();
        let mut j = i + run;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let n = chars[j..].iter().take_while(|c| **c == '`').count();
                if n == run {
                    close = Some(j);
                    break;
                }
                j += n;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                spans.push(chars[i + run..end].iter().collect());
                out.extend(std::iter::repeat_n(' ', end + run - i));
                i = end + run;
            }
            None => {
                out.extend(std::iter::repeat_n('`', run));
                i += run;
            }
        }
    }
    (out, spans)
}

/// The text of an ATX heading line.
fn heading_text(line: &str) -> Option<String> {
    let indent = line.len() - line.trim_start().len();
    if indent > 3 {
        return None;
    }
    let rest = line.trim_start();
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let text = rest[hashes..].strip_prefix(' ')?;
    Some(text.trim().trim_end_matches('#').trim_end().to_owned())
}

/// GitHub's anchor for a heading: lower case, punctuation dropped, spaces to hyphens.
fn slug(heading: &str) -> String {
    let mut text = String::new();
    let mut rest = heading;
    // `[label](url)` keeps only its label.
    while let Some(at) = rest.find("](") {
        text.push_str(&rest[..at]);
        match rest[at..].find(')') {
            Some(close) => rest = &rest[at + close + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    text.push_str(rest);
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.extend(c.to_lowercase());
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

/// The anchors a Markdown document defines: heading slugs (repeats get `-1`, `-2`, ...) and
/// explicit `id=` / `name=` attributes.
fn anchors(text: &str) -> HashSet<String> {
    let parsed = parse_markdown(text);
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut set = HashSet::new();
    for (_, heading) in &parsed.headings {
        let base = slug(heading);
        let n = seen.entry(base.clone()).or_insert(0);
        set.insert(if *n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        });
        *n += 1;
    }
    for attr in ["id=\"", "name=\""] {
        let mut rest = text;
        while let Some(at) = rest.find(attr) {
            rest = &rest[at + attr.len()..];
            if let Some(end) = rest.find('"') {
                set.insert(rest[..end].to_lowercase());
            }
        }
    }
    set
}

// ---------------------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------------------

/// Every relative link of the Markdown `files` must resolve to an existing file or directory
/// inside the repository, and an `#anchor` into a Markdown file to one of its headings.
fn check_links(root: &Path, files: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut anchor_cache: HashMap<String, HashSet<String>> = HashMap::new();
    for file in files {
        let Ok(text) = fs::read_to_string(root.join(file)) else {
            problems.push(format!("{file}: cannot read"));
            continue;
        };
        let parsed = parse_markdown(&text);
        for (no, line) in &parsed.prose {
            for target in link_targets(line) {
                if let Some(problem) = link_problem(root, file, &target, &mut anchor_cache) {
                    problems.push(format!("{file}:{no}: {problem}"));
                }
            }
        }
    }
    problems
}

/// The destinations of the inline links `](dest)` and reference definitions `[id]: dest` of
/// one prose line.
fn link_targets(line: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("](") {
        let (target, after) = link_destination(&rest[at + 2..]);
        if let Some(target) = target {
            targets.push(target);
        }
        rest = after;
    }
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() <= 3
        && trimmed.starts_with('[')
        && let Some(close) = trimmed.find("]:")
        && !trimmed[1..close].contains(']')
    {
        let dest = trimmed[close + 2..].trim_start();
        if !dest.is_empty() {
            targets.push(link_destination(dest).0.unwrap_or_default());
        }
    }
    targets.retain(|t| !t.is_empty());
    targets
}

/// Splits the text after `](` into the destination and what follows it.
fn link_destination(text: &str) -> (Option<String>, &str) {
    if let Some(inner) = text.strip_prefix('<') {
        return match inner.find('>') {
            Some(end) => (Some(inner[..end].to_owned()), &inner[end + 1..]),
            None => (None, inner),
        };
    }
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return (Some(text[..i].to_owned()), &text[i + 1..]),
            ')' => depth -= 1,
            c if c.is_whitespace() => {
                // a title follows: `(dest "title")`
                return (Some(text[..i].to_owned()), &text[i..]);
            }
            _ => {}
        }
    }
    (None, "")
}

/// Why `target`, written in `file`, does not resolve; `None` when it does or is not ours to
/// check (an external URL).
fn link_problem(
    root: &Path,
    file: &str,
    target: &str,
    cache: &mut HashMap<String, HashSet<String>>,
) -> Option<String> {
    if has_scheme(target) || target.starts_with("//") {
        return None;
    }
    let (path_part, anchor) = match target.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (target, None),
    };
    let path_part = percent_decode(path_part);
    let resolved = if path_part.is_empty() {
        file.to_owned()
    } else {
        let base = if let Some(absolute) = path_part.strip_prefix('/') {
            PathBuf::from(absolute)
        } else {
            Path::new(file)
                .parent()
                .unwrap_or(Path::new(""))
                .join(&path_part)
        };
        match normalize(&base) {
            Some(path) => path,
            None => {
                return Some(format!("link `{target}` leaves the repository"));
            }
        }
    };
    let on_disk = root.join(&resolved);
    if !on_disk.exists() {
        return Some(format!(
            "broken link `{target}`: `{resolved}` does not exist"
        ));
    }
    let anchor = anchor.filter(|a| !a.is_empty())?;
    if !(resolved.ends_with(".md") && on_disk.is_file()) {
        return None;
    }
    let defined = cache
        .entry(resolved.clone())
        .or_insert_with(|| anchors(&fs::read_to_string(&on_disk).unwrap_or_default()));
    (!defined.contains(&anchor.to_lowercase()))
        .then(|| format!("broken anchor `{target}`: `{resolved}` has no heading `#{anchor}`"))
}

fn has_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves `.` and `..` lexically; `None` when the path climbs out of the root.
fn normalize(path: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Some(parts.join("/"))
}

// ---------------------------------------------------------------------------------------
// xtask commands
// ---------------------------------------------------------------------------------------

/// Every `cargo xtask <word>` inside a code span or fence names a command in `known`.
fn check_xtask_commands(root: &Path, files: &[String], known: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(root.join(file)) else {
            continue;
        };
        for (no, code) in parse_markdown(&text).code {
            for command in xtask_mentions(&code) {
                if !known.contains(&command) {
                    problems.push(format!(
                        "{file}:{no}: `cargo xtask {command}` is not an xtask command; \
                         `cargo xtask --help` lists them"
                    ));
                }
            }
        }
    }
    problems
}

/// The words that follow `cargo xtask ` in `code`, ignoring placeholders and flags.
fn xtask_mentions(code: &str) -> Vec<String> {
    const PREFIX: &str = "cargo xtask ";
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find(PREFIX) {
        rest = &rest[at + PREFIX.len()..];
        let word: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if word.starts_with(|c: char| c.is_ascii_lowercase()) {
            found.push(word);
        }
    }
    found
}

// ---------------------------------------------------------------------------------------
// ADRs
// ---------------------------------------------------------------------------------------

/// The kinds of ADR status. What follows the kind (`(planner default)`, `. Amended by 0069`)
/// is annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusKind {
    Proposed,
    Accepted,
    Superseded(String),
}

impl StatusKind {
    fn shown(&self) -> String {
        match self {
            Self::Proposed => "Proposed".to_owned(),
            Self::Accepted => "Accepted".to_owned(),
            Self::Superseded(by) => format!("Superseded by {by}"),
        }
    }
}

/// Parses the leading status kind of `text`.
fn parse_status(text: &str) -> Result<StatusKind, String> {
    let text = text.trim();
    let boundary = |rest: &str| !rest.starts_with(|c: char| c.is_alphanumeric());
    for (word, kind) in [
        ("Proposed", StatusKind::Proposed),
        ("Accepted", StatusKind::Accepted),
    ] {
        if let Some(rest) = text.strip_prefix(word)
            && boundary(rest)
        {
            return Ok(kind);
        }
    }
    if let Some(rest) = text.strip_prefix("Superseded by ") {
        let rest = rest.strip_prefix('[').unwrap_or(rest);
        let number: String = rest.chars().take(4).collect();
        if number.len() == 4 && number.chars().all(|c| c.is_ascii_digit()) && boundary(&rest[4..]) {
            return Ok(StatusKind::Superseded(number));
        }
    }
    Err(format!(
        "invalid status `{text}`: it must start with `Proposed`, `Accepted` or `Superseded by NNNN`"
    ))
}

/// One ADR file.
struct Adr {
    file: String,
    number: String,
    title: Option<String>,
    status: Result<StatusKind, String>,
}

/// Reads `doc/adr/NNNN-*.md`: file name, title line and status section.
fn read_adrs(root: &Path) -> (Vec<Adr>, Vec<String>) {
    let mut problems = Vec::new();
    let mut files = Vec::new();
    walk(
        root,
        &root.join(ADR_DIR),
        &mut |rel| {
            rel.ends_with(".md")
                && rel.matches('/').count() == 2
                && !rel.ends_with("/README.md")
                && !rel.ends_with("/template.md")
        },
        &mut files,
    );
    files.sort();
    let mut adrs = Vec::new();
    for file in files {
        let name = file.rsplit('/').next().unwrap_or(&file);
        let valid = name.len() > 8
            && name[..4].chars().all(|c| c.is_ascii_digit())
            && name[4..].starts_with('-')
            && name[5..name.len() - 3]
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            problems.push(format!(
                "{file}:1: ADR file names are `NNNN-kebab-title.md`"
            ));
            continue;
        }
        let text = fs::read_to_string(root.join(&file)).unwrap_or_default();
        let title_line = text.lines().next().unwrap_or_default();
        let number = name[..4].to_owned();
        let title = match title_line
            .strip_prefix("# ")
            .and_then(|t| t.split_once(". "))
        {
            Some((n, title)) if n == number => Some(title.trim().to_owned()),
            Some((n, _)) => {
                problems.push(format!(
                    "{file}:1: title says ADR {n} but the file name says {number}"
                ));
                None
            }
            None => {
                problems.push(format!(
                    "{file}:1: the first line must be `# {number}. Title`"
                ));
                None
            }
        };
        let status = status_section(&text)
            .ok_or_else(|| "no `## Status` section with text".to_owned())
            .and_then(|s| parse_status(&s));
        if let Err(err) = &status {
            problems.push(format!("{file}:1: {err}"));
        }
        adrs.push(Adr {
            file,
            number,
            title,
            status,
        });
    }
    (adrs, problems)
}

/// The first paragraph under `## Status`.
fn status_section(text: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| l.trim() != "## Status");
    lines.next()?;
    let paragraph: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::trim)
        .collect();
    (!paragraph.is_empty()).then(|| paragraph.join(" "))
}

/// One row of the index table.
struct IndexRow {
    line: usize,
    text_number: String,
    target: String,
    title: String,
    status: String,
}

fn index_rows(index: &str) -> Vec<IndexRow> {
    let mut rows = Vec::new();
    for (i, line) in index.lines().enumerate() {
        let Some(inner) = line.strip_prefix("| [") else {
            continue;
        };
        let cells: Vec<&str> = inner.splitn(3, " | ").collect();
        let [first, title, status] = cells[..] else {
            continue;
        };
        let Some((text_number, rest)) = first.split_once("](") else {
            continue;
        };
        let target = rest.trim_end_matches(')').to_owned();
        rows.push(IndexRow {
            line: i + 1,
            text_number: text_number.to_owned(),
            target,
            title: title.trim().to_owned(),
            status: status.trim().trim_end_matches('|').trim().to_owned(),
        });
    }
    rows
}

/// The ADR checks: files against the index and against themselves.
fn check_adrs(root: &Path, require_adrs: bool) -> Vec<String> {
    let (adrs, mut problems) = read_adrs(root);
    if require_adrs {
        if !root.join(ADR_DIR).is_dir() {
            problems.push(format!("{ADR_DIR}:1: the ADR directory is missing"));
        } else if adrs.is_empty() {
            problems.push(format!("{ADR_DIR}:1: no ADRs found"));
        }
    }
    let mut by_number: BTreeMap<&str, Vec<&Adr>> = BTreeMap::new();
    for adr in &adrs {
        by_number.entry(adr.number.as_str()).or_default().push(adr);
    }
    for (number, same) in &by_number {
        for other in &same[1..] {
            problems.push(format!(
                "{}:1: ADR number {number} is also used by {}",
                other.file, same[0].file
            ));
        }
    }
    for adr in &adrs {
        if let Ok(StatusKind::Superseded(by)) = &adr.status
            && !by_number.contains_key(by.as_str())
        {
            problems.push(format!(
                "{}:1: superseded by ADR {by}, which does not exist",
                adr.file
            ));
        }
    }

    let Ok(index) = fs::read_to_string(root.join(ADR_INDEX)) else {
        // A tree without ADRs has no index to keep (the fixtures of the other checks).
        if !adrs.is_empty() {
            problems.push(format!("{ADR_INDEX}:1: cannot read the ADR index"));
        }
        return problems;
    };
    let mut listed: HashMap<String, usize> = HashMap::new();
    let mut previous: Option<String> = None;
    for row in index_rows(&index) {
        let file = format!("{ADR_DIR}/{}", row.target);
        let name_number = row.target.get(..4).unwrap_or_default().to_owned();
        if row.text_number != name_number {
            problems.push(format!(
                "{ADR_INDEX}:{}: row `[{}]({})` shows a number other than its file name's {name_number}",
                row.line, row.text_number, row.target
            ));
        }
        if let Some(first) = listed.insert(file.clone(), row.line) {
            problems.push(format!(
                "{ADR_INDEX}:{}: {file} is listed twice (first at line {first})",
                row.line
            ));
            continue;
        }
        if let Some(prev) = &previous
            && name_number < *prev
        {
            problems.push(format!(
                "{ADR_INDEX}:{}: rows must be in ascending order; {name_number} follows {prev}",
                row.line
            ));
        }
        previous = Some(name_number);
        let Some(adr) = adrs.iter().find(|a| a.file == file) else {
            problems.push(format!(
                "{ADR_INDEX}:{}: row links {file}, which is not an ADR file",
                row.line
            ));
            continue;
        };
        if let Some(title) = &adr.title
            && *title != row.title
        {
            problems.push(format!(
                "{ADR_INDEX}:{}: title `{}` differs from the file's `{title}`",
                row.line, row.title
            ));
        }
        match (parse_status(&row.status), &adr.status) {
            (Ok(shown), Ok(actual)) if shown != *actual => problems.push(format!(
                "{ADR_INDEX}:{}: status `{}` differs from the file's `{}`",
                row.line,
                shown.shown(),
                actual.shown()
            )),
            (Err(err), _) => problems.push(format!("{ADR_INDEX}:{}: {err}", row.line)),
            _ => {}
        }
    }
    for adr in &adrs {
        if !listed.contains_key(&adr.file) {
            problems.push(format!(
                "{}:1: ADR {} is not listed in {ADR_INDEX}",
                adr.file, adr.number
            ));
        }
    }
    problems
}

// ---------------------------------------------------------------------------------------
// Rule ids, serve flags, warning codes, AGENTS.md
// ---------------------------------------------------------------------------------------

/// Every rule id is named (in backticks) by the rules document and the document names no
/// other `ARCH-*` rule.
fn check_rule_ids(doc: &str, ids: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    for id in ids {
        if !doc.contains(&format!("`{id}`")) {
            problems.push(format!(
                "{RULES_DOC}:1: rule `{id}` of tools/xtask/src/architecture/rules.rs is not in the catalog"
            ));
        }
    }
    for (i, line) in doc.lines().enumerate() {
        for token in line.split('`').skip(1).step_by(2) {
            if token.starts_with("ARCH-")
                && token[5..].starts_with(|c: char| c.is_ascii_lowercase())
                && !ids.contains(&token)
            {
                problems.push(format!(
                    "{RULES_DOC}:{}: `{token}` is not a rule of rules.rs",
                    i + 1
                ));
            }
        }
    }
    problems
}

/// The long flags a `--help` text lists, in order.
fn help_flags(help: &str) -> Vec<String> {
    let mut flags = Vec::new();
    for line in help.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('-') {
            continue;
        }
        // The flag column ends at the first run of two spaces; a description may mention
        // other flags (`--foo`) that are not options of this command.
        let column = trimmed.split("  ").next().unwrap_or(trimmed);
        if let Some(at) = column.find("--") {
            let name: String = column[at + 2..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if !name.is_empty() && !flags.contains(&name) {
                flags.push(name);
            }
        }
    }
    flags
}

/// The long flags named in the first cell of the README's CLI reference table rows.
fn readme_flags(readme: &str) -> Vec<(usize, String)> {
    let mut in_section = false;
    let mut flags = Vec::new();
    for (i, line) in readme.lines().enumerate() {
        if let Some(heading) = heading_text(line) {
            in_section = line.starts_with("## ") && heading == "CLI reference"
                || in_section && line.starts_with("### ");
        }
        if !in_section || !line.starts_with('|') {
            continue;
        }
        let cell = line.split('|').nth(1).unwrap_or_default();
        let mut rest = cell;
        while let Some(at) = rest.find("--") {
            rest = &rest[at + 2..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if !name.is_empty() {
                flags.push((i + 1, name));
            }
        }
    }
    flags
}

/// Every flag of `serve --help` is in the README table, and the table has no other (the
/// top-level `--version` aside).
fn check_serve_flags(readme: &str, serve_help: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let documented = readme_flags(readme);
    let actual = help_flags(serve_help);
    for flag in &actual {
        if !documented.iter().any(|(_, name)| name == flag) {
            problems.push(format!(
                "{README}:1: `serve --{flag}` is in `site-preview serve --help` but not in the CLI reference table"
            ));
        }
    }
    for (line, name) in &documented {
        if name != "version" && !actual.contains(name) {
            problems.push(format!(
                "{README}:{line}: the CLI reference table lists `--{name}`, which `site-preview serve --help` does not"
            ));
        }
    }
    problems
}

/// The codes the registry source declares: `NAME = "code";` lines before its tests.
fn registry_codes(registry: &str) -> Vec<String> {
    let end = registry.find("#[cfg(test)]").unwrap_or(registry.len());
    registry[..end]
        .lines()
        .filter_map(|line| {
            let (name, value) = line.trim().split_once(" = \"")?;
            let code = value.strip_suffix("\";")?;
            (!name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))
            .then(|| code.to_owned())
        })
        .collect()
}

/// The codes in the first cells of the README "Warnings" table.
fn readme_warning_codes(readme: &str) -> Vec<(usize, String)> {
    let mut in_section = false;
    let mut codes = Vec::new();
    for (i, line) in readme.lines().enumerate() {
        if line.starts_with("## ") {
            in_section = line.trim() == "## Warnings";
        }
        if in_section
            && line.starts_with("| `")
            && let Some(code) = line[3..].split('`').next()
        {
            codes.push((i + 1, code.to_owned()));
        }
    }
    codes
}

/// The registry and the README warnings table list the same codes.
fn check_warning_codes(readme: &str, registry: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let declared = registry_codes(registry);
    if declared.is_empty() {
        problems.push(format!("{WARNING_REGISTRY}:1: no warning codes found"));
    }
    let documented = readme_warning_codes(readme);
    for code in &declared {
        if !documented.iter().any(|(_, c)| c == code) {
            problems.push(format!(
                "{README}:1: warning code `{code}` ({WARNING_REGISTRY}) is not in the Warnings table"
            ));
        }
    }
    for (line, code) in &documented {
        if !declared.contains(code) {
            problems.push(format!(
                "{README}:{line}: the Warnings table lists `{code}`, which {WARNING_REGISTRY} does not declare"
            ));
        }
    }
    problems
}

/// The non-empty lines of every ```sh block, trimmed (the logic of the spec's
/// `shellCommands`).
fn shell_commands(markdown: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_block = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if in_block {
            if trimmed.starts_with("```") {
                in_block = false;
            } else if !trimmed.is_empty() {
                commands.push(trimmed.to_owned());
            }
        } else if trimmed == "```sh" {
            in_block = true;
        }
    }
    commands
}

/// The spec's `EXERCISED` keys and the first cells of its header comment table.
fn spec_commands(spec: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut keys = BTreeSet::new();
    let mut in_map = false;
    for line in spec.lines() {
        if line.contains("const EXERCISED = new Map") {
            in_map = true;
        } else if in_map && line.trim_start().starts_with("])") {
            in_map = false;
        } else if in_map
            && let Some(rest) = line.trim_start().strip_prefix("['")
            && let Some((key, _)) = rest.split_once("', '")
        {
            keys.insert(key.to_owned());
        }
    }
    let mut table = BTreeSet::new();
    for line in spec.lines() {
        let Some(row) = line.strip_prefix("//   | ") else {
            continue;
        };
        let cell = row.split(" | ").next().unwrap_or_default().trim();
        if !cell.starts_with(':') && cell != "Command in AGENTS.md" && !cell.is_empty() {
            table.insert(cell.trim_end_matches('|').trim().to_owned());
        }
    }
    (keys, table)
}

/// AGENTS.md, the spec's `EXERCISED` map and its comment table list the same commands.
fn check_agents_spec(agents: &str, spec: &str) -> Vec<String> {
    let documented: BTreeSet<String> = shell_commands(agents).into_iter().collect();
    let (keys, table) = spec_commands(spec);
    let mut problems = Vec::new();
    if documented.is_empty() {
        problems.push(format!("{AGENTS}:1: no ```sh commands found"));
    }
    for command in &documented {
        if !keys.contains(command) {
            problems.push(format!(
                "{AGENTS}:1: command `{command}` has no row in the EXERCISED map of {AGENT_SPEC}"
            ));
        }
        if !table.contains(command) {
            problems.push(format!(
                "{AGENTS}:1: command `{command}` is not in the comment table of {AGENT_SPEC}"
            ));
        }
    }
    for command in keys.union(&table) {
        if !documented.contains(command) {
            problems.push(format!(
                "{AGENT_SPEC}:1: exercises `{command}`, which {AGENTS} does not document"
            ));
        }
    }
    problems.sort();
    problems.dedup();
    problems
}

// ---------------------------------------------------------------------------------------
// serve --help
// ---------------------------------------------------------------------------------------

/// The `--help` text of `site-preview serve`, from the binary built from this tree.
fn serve_help(root: &Path) -> Result<String, String> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "run",
            "--locked",
            "--quiet",
            "--package",
            "preview-cli",
            "--",
            "serve",
            "--help",
        ])
        .output()
        .map_err(|e| format!("cannot run cargo to read `serve --help`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`site-preview serve --help` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

/// Runs each fixture tree and compares its diagnostics with `expected.txt` (one per line;
/// empty for a positive fixture).
fn check_fixtures(root: &Path) -> Vec<String> {
    let known = known_commands();
    let mut problems = Vec::new();
    let mut negatives = 0;
    for (dir, must_fail) in [(NEGATIVE_FIXTURES, true), (POSITIVE_FIXTURES, false)] {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            problems.push(format!("{dir}: cannot list the fixtures"));
            continue;
        };
        let mut cases: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .collect();
        cases.sort_by_key(|e| e.file_name());
        for case in cases {
            let name = format!("{dir}/{}", case.file_name().to_string_lossy());
            let expected_text = fs::read_to_string(case.path().join("expected.txt"));
            let Ok(expected_text) = expected_text else {
                problems.push(format!("{name}: expected.txt is missing"));
                continue;
            };
            let mut expected: Vec<String> = expected_text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
            expected.sort();
            if must_fail {
                negatives += 1;
                if expected.is_empty() {
                    problems.push(format!(
                        "{name}: a negative fixture must expect a diagnostic"
                    ));
                }
            } else if !expected.is_empty() {
                problems.push(format!(
                    "{name}: a positive fixture must expect no diagnostic"
                ));
            }
            let actual = check_tree(&case.path(), &known, false);
            if actual != expected {
                problems.push(format!(
                    "{name}: expected diagnostics {expected:?} but got {actual:?}"
                ));
            }
        }
    }
    if negatives == 0 {
        problems.push(format!("{NEGATIVE_FIXTURES}: no negative fixtures"));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn adr(number: &str, title: &str, status: &str) -> String {
        format!(
            "# {number}. {title}\n\nDate: 2026-01-01\n\n## Status\n\n{status}\n\n## Context\n\nx\n"
        )
    }

    fn index(rows: &[(&str, &str, &str, &str)]) -> String {
        let mut text = String::from("# ADRs\n\n| ADR | Title | Status |\n|---|---|---|\n");
        for (number, file, title, status) in rows {
            text.push_str(&format!("| [{number}]({file}) | {title} | {status} |\n"));
        }
        text
    }

    #[test]
    fn should_pass_the_repository_when_the_docs_match_the_code() {
        // given
        let root = repo::root();

        // when
        let problems = check_repository(&root);

        // then
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn should_match_every_fixture_when_the_fixture_trees_are_checked() {
        // given
        let root = repo::root();

        // when
        let problems = check_fixtures(&root);

        // then
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn should_report_unindexed_adr_when_the_index_lacks_a_file() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/adr/0001-a.md",
            &adr("0001", "A", "Accepted"),
        );
        write(
            dir.path(),
            "doc/adr/0002-b.md",
            &adr("0002", "B", "Accepted"),
        );
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[("0001", "0001-a.md", "A", "Accepted")]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/0002-b.md:1: ADR 0002 is not listed in doc/adr/README.md"]
        );
    }

    #[test]
    fn should_report_missing_file_when_the_index_lists_an_unknown_adr() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[("0003", "0003-c.md", "C", "Accepted")]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/README.md:5: row links doc/adr/0003-c.md, which is not an ADR file"]
        );
    }

    #[test]
    fn should_report_both_files_when_two_adrs_share_a_number() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/adr/0005-a.md",
            &adr("0005", "A", "Accepted"),
        );
        write(
            dir.path(),
            "doc/adr/0005-b.md",
            &adr("0005", "B", "Accepted"),
        );
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[
                ("0005", "0005-a.md", "A", "Accepted"),
                ("0005", "0005-b.md", "B", "Accepted"),
            ]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/0005-b.md:1: ADR number 0005 is also used by doc/adr/0005-a.md"]
        );
    }

    #[test]
    fn should_report_the_title_number_when_it_differs_from_the_file_name() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/adr/0007-a.md",
            &adr("0008", "A", "Accepted"),
        );
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[("0007", "0007-a.md", "A", "Accepted")]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/0007-a.md:1: title says ADR 0008 but the file name says 0007"]
        );
    }

    #[test]
    fn should_report_invalid_status_when_an_adr_is_neither_proposed_nor_accepted() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "doc/adr/0001-a.md", &adr("0001", "A", "Draft"));
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[("0001", "0001-a.md", "A", "Accepted")]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            [
                "doc/adr/0001-a.md:1: invalid status `Draft`: it must start with `Proposed`, `Accepted` or `Superseded by NNNN`"
            ]
        );
    }

    #[test]
    fn should_report_status_difference_when_the_index_disagrees_with_the_file() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/adr/0001-a.md",
            &adr("0001", "A", "Accepted"),
        );
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[("0001", "0001-a.md", "A", "Proposed")]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/README.md:5: status `Proposed` differs from the file's `Accepted`"]
        );
    }

    #[test]
    fn should_report_duplicate_and_disordered_rows_when_the_index_repeats_an_adr() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/adr/0001-a.md",
            &adr("0001", "A", "Accepted"),
        );
        write(
            dir.path(),
            "doc/adr/0002-b.md",
            &adr("0002", "B", "Accepted"),
        );
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[
                ("0001", "0001-a.md", "A", "Accepted"),
                ("0002", "0002-b.md", "B", "Accepted"),
                ("0001", "0001-a.md", "A", "Accepted"),
            ]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/README.md:7: doc/adr/0001-a.md is listed twice (first at line 5)"]
        );
    }

    #[test]
    fn should_report_title_difference_when_the_index_title_drifted() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/adr/0001-a.md",
            &adr("0001", "New title", "Accepted"),
        );
        write(
            dir.path(),
            ADR_INDEX,
            &index(&[("0001", "0001-a.md", "Old title", "Accepted")]),
        );

        // when
        let problems = check_adrs(dir.path(), false);

        // then
        assert_eq!(
            problems,
            ["doc/adr/README.md:5: title `Old title` differs from the file's `New title`"]
        );
    }

    #[test]
    fn should_accept_annotated_statuses_when_the_kind_leads() {
        // given
        let texts = [
            "Proposed (planner default \u{2014} owner may override). Amended by ADR 0054",
            "Accepted",
            "Superseded by [0032](0032-x.md)",
            "Superseded by 0032",
        ];

        // when
        let kinds: Vec<_> = texts.iter().map(|t| parse_status(t)).collect();

        // then
        assert_eq!(kinds[0], Ok(StatusKind::Proposed));
        assert_eq!(kinds[1], Ok(StatusKind::Accepted));
        assert_eq!(kinds[2], Ok(StatusKind::Superseded("0032".into())));
        assert_eq!(kinds[3], Ok(StatusKind::Superseded("0032".into())));
        assert!(parse_status("Acceptedish").is_err());
        assert!(parse_status("Superseded by the next one").is_err());
    }

    #[test]
    fn should_report_unknown_command_when_a_code_span_names_one() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "README.md",
            "Run `cargo xtask check` or `cargo xtask frobnicate`.\n\n```sh\ncargo xtask quick\ncargo xtask <cmd>\n```\n\nThe cargo xtask prose is ignored.\n",
        );
        let known = vec!["check".to_owned(), "quick".to_owned()];

        // when
        let problems = check_xtask_commands(dir.path(), &["README.md".to_owned()], &known);

        // then
        assert_eq!(
            problems,
            [
                "README.md:1: `cargo xtask frobnicate` is not an xtask command; `cargo xtask --help` lists them"
            ]
        );
    }

    #[test]
    fn should_report_broken_link_when_the_target_file_is_missing() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "doc/a.md",
            "[ok](b.md), [gone](missing.md), [web](https://example.com/x), `[code](nope.md)`\n\n```\n[fenced](nope.md)\n```\n",
        );
        write(dir.path(), "doc/b.md", "# B\n");

        // when
        let problems = check_links(dir.path(), &["doc/a.md".to_owned(), "doc/b.md".to_owned()]);

        // then
        assert_eq!(
            problems,
            ["doc/a.md:1: broken link `missing.md`: `doc/missing.md` does not exist"]
        );
    }

    #[test]
    fn should_report_broken_anchor_when_the_heading_is_absent() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.md",
            "# Top\n\n## Two words, `code`\n\n[self](#two-words-code) [bad](#nope) [other](b.md#x-y) [bad2](b.md#zzz)\n",
        );
        write(dir.path(), "b.md", "# X y\n\n## X y\n\n[again](#x-y-1)\n");

        // when
        let problems = check_links(dir.path(), &["a.md".to_owned(), "b.md".to_owned()]);

        // then
        assert_eq!(
            problems,
            [
                "a.md:5: broken anchor `#nope`: `a.md` has no heading `#nope`",
                "a.md:5: broken anchor `b.md#zzz`: `b.md` has no heading `#zzz`",
            ]
        );
    }

    #[test]
    fn should_reject_a_link_that_leaves_the_repository_when_it_climbs_too_far() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "[up](../outside.md)\n");

        // when
        let problems = check_links(dir.path(), &["a.md".to_owned()]);

        // then
        assert_eq!(
            problems,
            ["a.md:1: link `../outside.md` leaves the repository"]
        );
    }

    #[test]
    fn should_check_reference_definitions_when_a_link_uses_one() {
        // given
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "See [x].\n\n[x]: gone.md \"title\"\n");

        // when
        let problems = check_links(dir.path(), &["a.md".to_owned()]);

        // then
        assert_eq!(
            problems,
            ["a.md:3: broken link `gone.md`: `gone.md` does not exist"]
        );
    }

    #[test]
    fn should_report_missing_rule_when_the_catalog_lacks_an_id() {
        // given
        let doc = "| `ARCH-one` | x |\n| `ARCH-stale` | y |\n";

        // when
        let problems = check_rule_ids(doc, &["ARCH-one", "ARCH-two"]);

        // then
        assert_eq!(
            problems,
            [
                "doc/architecture-rules.md:1: rule `ARCH-two` of tools/xtask/src/architecture/rules.rs is not in the catalog",
                "doc/architecture-rules.md:2: `ARCH-stale` is not a rule of rules.rs",
            ]
        );
    }

    #[test]
    fn should_report_undocumented_flag_when_serve_help_lists_one_the_readme_lacks() {
        // given
        let help = "Options:\n      --host <IP>  Address\n      --json       Machine output\n  -h, --help       Print help\n";
        let readme = "## CLI reference\n\n| Argument | Meaning |\n|:--|:--|\n| `--host <IP>` | a |\n| `-h`, `--help` | b |\n| `--stale` | c |\n| `-V`, `--version` | d |\n\n## Next\n\n| `--other` | not ours |\n";

        // when
        let problems = check_serve_flags(readme, help);

        // then
        assert_eq!(
            problems,
            [
                "README.md:1: `serve --json` is in `site-preview serve --help` but not in the CLI reference table",
                "README.md:7: the CLI reference table lists `--stale`, which `site-preview serve --help` does not",
            ]
        );
    }

    #[test]
    fn should_report_undocumented_warning_when_the_registry_has_a_new_code() {
        // given
        let registry = "warning_codes! {\n    /// d\n    A_ONE = \"a-one\";\n    A_TWO = \"a-two\";\n}\n\n#[cfg(test)]\nmod tests { const X = \"nope\"; }\n";
        let readme = "## Warnings\n\n| Code | Meaning |\n|:--|:--|\n| `a-one` | x |\n| `gone` | y |\n\n## HTTP\n\n| `other` | z |\n";

        // when
        let problems = check_warning_codes(readme, registry);

        // then
        assert_eq!(
            problems,
            [
                "README.md:1: warning code `a-two` (crates/preview-domain/src/warning_code.rs) is not in the Warnings table",
                "README.md:6: the Warnings table lists `gone`, which crates/preview-domain/src/warning_code.rs does not declare",
            ]
        );
    }

    #[test]
    fn should_report_missing_adrs_when_the_repository_has_none() {
        // given
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("doc/adr");
        fs::create_dir_all(&empty).unwrap();

        // when
        let missing_dir = check_adrs(&dir.path().join("elsewhere"), true);
        let no_adrs = check_adrs(dir.path(), true);
        let fixture = check_adrs(dir.path(), false);

        // then
        assert_eq!(missing_dir, ["doc/adr:1: the ADR directory is missing"]);
        assert_eq!(no_adrs, ["doc/adr:1: no ADRs found"]);
        assert!(fixture.is_empty(), "{fixture:?}");
    }

    #[test]
    fn should_ignore_flags_in_descriptions_when_reading_help() {
        // given
        let help = "Options:\n  -p, --port <PORT>  Port; not --other-port\n      --open     Open it\n          see --also-not\n  -h, --help     Print help\n";

        // when
        let flags = help_flags(help);

        // then
        assert_eq!(flags, ["port", "open", "help"]);
    }

    #[test]
    fn should_skip_ignored_files_when_the_tree_is_a_git_work_tree() {
        // given
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q"]);
        fs::write(dir.path().join(".gitignore"), "scratch/\n").unwrap();
        fs::create_dir_all(dir.path().join("scratch")).unwrap();
        fs::write(dir.path().join("scratch/old.md"), "# x\n").unwrap();
        fs::write(dir.path().join("new.md"), "# y\n").unwrap();

        // when
        let files = markdown_files(dir.path());

        // then
        assert_eq!(files, ["new.md"]);
    }

    #[test]
    fn should_report_agents_command_when_the_spec_does_not_exercise_it() {
        // given
        let agents =
            "text\n\n```sh\nsite-preview serve --json\nkill -TERM <pid>\n```\n\n```json\n{}\n```\n";
        let spec = "// header\n//   | Command in AGENTS.md | Exercised by |\n//   |:--|:--|\n//   | site-preview serve --json | start |\n//   | curl <url> | fetch |\nconst EXERCISED = new Map<string, string>([\n  ['site-preview serve --json', 'start'],\n  ['curl <url>', 'fetch'],\n]);\n";

        // when
        let problems = check_agents_spec(agents, spec);

        // then
        assert_eq!(
            problems,
            [
                "AGENTS.md:1: command `kill -TERM <pid>` has no row in the EXERCISED map of tests/e2e/browser/agent-workflow.spec.ts",
                "AGENTS.md:1: command `kill -TERM <pid>` is not in the comment table of tests/e2e/browser/agent-workflow.spec.ts",
                "tests/e2e/browser/agent-workflow.spec.ts:1: exercises `curl <url>`, which AGENTS.md does not document",
            ]
        );
    }

    #[test]
    fn should_make_github_anchors_when_headings_have_punctuation_and_code() {
        // given
        let headings = [
            "Machine output (`--json`)",
            "What it is not",
            "Q&A: part_2 [x](y)",
        ];

        // when
        let slugs: Vec<_> = headings.iter().map(|h| slug(h)).collect();

        // then
        assert_eq!(
            slugs,
            ["machine-output---json", "what-it-is-not", "qa-part_2-x"]
        );
    }
}
