//! `cargo xtask release-check [--target T]`: the packaged-binary smoke of plan section 11 M5
//! (OPS-05, ADR 0076).
//!
//! It builds the release binary for `T` (default: the host), packages
//! `site-preview-<T>.tar.gz` with the binary, README, both licences and the generated
//! `THIRD-PARTY-NOTICES`, extracts that archive into a fresh temporary directory and tests the
//! **extracted** binary, never `cargo run`: it runs with `env -i PATH=/usr/bin:/bin` (cargo, node
//! and npm asserted absent from that path), answers `--version`, serves a copy of the fixture on
//! a dynamic port, serves `/`, sends an event-stream `reload` after a file edit, exits 0 on
//! SIGTERM, and leaves the served tree exactly as the edit left it.
//!
//! The notices are checked three ways, each failing the command: the archived copy must equal a
//! fresh `cargo about` run, every crate of the shipped graph (`cargo tree -e normal` for the
//! target) must have an entry in it, and the four MPL-2.0 crates of ADR 0043 must come with the
//! licence text and a source location (MPL-2.0 section 3.2). A missing `cargo-about` is an
//! install hint, never a skip (ADR 0010). OPS-06 calls this once per native runner.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::bench::server::{Opened, SseClient, authority_of, parse_ready_url};

/// The four release targets of plan section 11 M5, and the names of their archives.
pub const RELEASE_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];

/// The MPL-2.0 crates `deny.toml` admits by exception (ADR 0043).
const MPL_CRATES: [&str; 4] = ["cssparser", "cssparser-macros", "dtoa-short", "selectors"];

/// The `PATH` the extracted binary runs with.
const CLEAN_PATH: &str = "/usr/bin:/bin";

/// Programs that must not be reachable on [`CLEAN_PATH`]: the binary needs no toolchain.
const FORBIDDEN_PROGRAMS: [&str; 3] = ["cargo", "node", "npm"];

/// File name of the notices, in the repository's generator output and in the archive.
const NOTICES: &str = "THIRD-PARTY-NOTICES";

/// What an archive holds, below its one top directory, besides the binary.
const ARCHIVE_FILES: [&str; 5] = [
    "site-preview",
    "README.md",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    NOTICES,
];

/// Wait for the server to print `ready`, to exit after SIGTERM, or for a reload.
const DEADLINE: Duration = Duration::from_secs(30);

/// Runs the command and returns its exit code.
pub fn run(target: Option<&str>) -> i32 {
    match check(target) {
        Ok(summary) => {
            for line in summary {
                eprintln!("release-check: {line}");
            }
            0
        }
        Err(err) => {
            eprintln!("xtask: release-check: {err}");
            1
        }
    }
}

fn check(target_arg: Option<&str>) -> Result<Vec<String>, String> {
    let root = crate::repo::root();
    let target = match target_arg {
        Some(target) => target.to_owned(),
        None => host_target()?,
    };
    validate_target(&target)?;
    let mut summary = Vec::new();
    if !RELEASE_TARGETS.contains(&target.as_str()) {
        eprintln!("release-check: {target} is not one of the four release targets; smoke only");
    }
    crate::tools::require("cargo-about")?;
    ensure_target_installed(&target)?;

    let out_dir = root
        .join(target_dir_name())
        .join("release-check")
        .join(&target);
    let logs = out_dir.join("logs");
    reset_dir(&out_dir)?;
    fs::create_dir_all(&logs).map_err(|e| format!("cannot create {}: {e}", logs.display()))?;

    let binary = build(&root, &target)?;
    let shipped = shipped_crates(&root, &target)?;
    let notices = generate_notices(&root, &target)?;
    let archive = package(&root, &out_dir, &target, &binary, &notices)?;
    summary.push(format!("target {target}"));
    summary.push(format!("archive {}", archive.display()));

    let scratch = tempfile::tempdir().map_err(|e| format!("cannot make a temp dir: {e}"))?;
    let extracted = extract(&archive, scratch.path(), &target)?;
    summary.push(format!("members {}", ARCHIVE_FILES.join(", ")));

    let archived = fs::read_to_string(extracted.join(NOTICES))
        .map_err(|e| format!("cannot read the archived {NOTICES}: {e}"))?;
    let fresh = generate_notices(&root, &target)?;
    verify_notices(&archived, &fresh, &shipped)?;
    let mpl = MPL_CRATES
        .iter()
        .filter(|name| shipped.iter().any(|(n, _)| n == *name))
        .count();
    summary.push(format!(
        "{NOTICES}: matches a fresh cargo-about run; {} shipped crates listed; {mpl} MPL-2.0 crates with licence text and source",
        shipped.len()
    ));

    smoke(
        &root,
        &extracted.join("site-preview"),
        scratch.path(),
        &logs,
    )?;
    summary.push(
        "smoke: --version, serve --json --port 0, GET /, reload after an edit, SIGTERM exit 0, tree unchanged"
            .to_owned(),
    );
    Ok(summary)
}

/// `CARGO_TARGET_DIR` (relative to the root) or `target`.
fn target_dir_name() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from)
}

fn reset_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))
}

// ---------------------------------------------------------------------------------------------
// Targets and the build

/// The `host:` line of `rustc -vV`.
fn parse_host(rustc_vv: &str) -> Option<String> {
    rustc_vv
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
}

fn host_target() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| format!("cannot run rustc to find the host target: {e}"))?;
    parse_host(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "rustc -vV printed no host line".to_owned())
}

/// A target triple is joined into paths and archive names, so only triple characters pass.
fn validate_target(target: &str) -> Result<(), String> {
    let ok = !target.is_empty()
        && target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        && !target.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(format!("'{target}' is not a target triple"))
    }
}

/// The archive file name for `target`.
fn archive_name(target: &str) -> String {
    format!("site-preview-{target}.tar.gz")
}

/// Fails with the `rustup target add` hint when the target's standard library is missing.
fn ensure_target_installed(target: &str) -> Result<(), String> {
    let Ok(output) = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    else {
        // No rustup (a distro toolchain): the build reports a missing standard library itself.
        return Ok(());
    };
    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.lines().any(|line| line.trim() == target) {
        Ok(())
    } else {
        Err(format!(
            "the Rust target {target} is not installed: run 'rustup target add {target}'{}",
            musl_hint(target)
        ))
    }
}

/// Linux musl builds link with the distribution's musl C library; the workspace's own crates
/// are pure Rust, but a C-backed dependency (or the linker) would need `musl-tools`.
fn musl_hint(target: &str) -> &'static str {
    if target.ends_with("-musl") {
        " (and 'sudo apt-get install musl-tools' if linking fails)"
    } else {
        ""
    }
}

fn build(root: &Path, target: &str) -> Result<PathBuf, String> {
    eprintln!("release-check: building site-preview for {target}");
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "preview-cli",
            "--bin",
            "site-preview",
            "--target",
            target,
        ])
        .current_dir(root)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build --release --target {target} failed{}",
            musl_hint(target)
        ));
    }
    let binary = root
        .join(target_dir_name())
        .join(target)
        .join("release/site-preview");
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!("{} was not built", binary.display()))
    }
}

// ---------------------------------------------------------------------------------------------
// The shipped crate graph and the notices

/// `(name, version)` of every non-workspace crate in `cargo tree --prefix none --format "{p}"`
/// output. A line is `name vX.Y.Z`, optionally followed by `(proc-macro)`, a source in
/// parentheses (a workspace member's is its absolute path) and `(*)` for a repeated subtree.
fn parse_shipped(tree: &str) -> BTreeSet<(String, String)> {
    tree.lines()
        .filter(|line| !line.contains(" (/"))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let version = words.next()?.strip_prefix('v')?;
            Some((name.to_owned(), version.to_owned()))
        })
        .collect()
}

/// The crates linked into the binary for `target`: normal dependencies of `preview-cli`, so no
/// dev- or build-only tool. Workspace members are the project's own code, not third parties.
fn shipped_crates(root: &Path, target: &str) -> Result<BTreeSet<(String, String)>, String> {
    let output = Command::new("cargo")
        .args([
            "tree",
            "--locked",
            "-p",
            "preview-cli",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--target",
            target,
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run cargo tree: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let shipped = parse_shipped(&String::from_utf8_lossy(&output.stdout));
    if shipped.is_empty() {
        return Err("cargo tree listed no third-party crate".to_owned());
    }
    Ok(shipped)
}

/// The notices `cargo about` generates for `target`, from `about.toml` and `about.hbs`.
fn generate_notices(root: &Path, target: &str) -> Result<String, String> {
    let output = Command::new("cargo")
        .args([
            "about",
            "generate",
            "--locked",
            "--fail",
            "--offline",
            "--manifest-path",
            "crates/preview-cli/Cargo.toml",
            "--config",
            "about.toml",
            "--target",
            target,
            "about.hbs",
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run cargo about: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo about generate failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut text = String::from_utf8(output.stdout)
        .map_err(|e| format!("cargo about printed invalid UTF-8: {e}"))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// The `(name, version)` of every entry line (`* name version ...`) of the notices.
fn listed_crates(notices: &str) -> BTreeSet<(String, String)> {
    notices
        .lines()
        .filter_map(|line| line.strip_prefix("* "))
        .filter_map(|rest| {
            let mut words = rest.split_whitespace();
            Some((words.next()?.to_owned(), words.next()?.to_owned()))
        })
        .collect()
}

/// Everything wrong with the archived notices, empty when they are sound.
fn notices_problems(
    archived: &str,
    fresh: &str,
    shipped: &BTreeSet<(String, String)>,
) -> Vec<String> {
    let mut problems = Vec::new();
    if archived != fresh {
        problems.push(format!(
            "the archived {NOTICES} differs from a fresh cargo-about run"
        ));
    }
    let listed = listed_crates(archived);
    for (name, version) in shipped.difference(&listed) {
        problems.push(format!("{NOTICES} has no entry for {name} {version}"));
    }
    let mpl_shipped: Vec<_> = shipped
        .iter()
        .filter(|(name, _)| MPL_CRATES.contains(&name.as_str()))
        .collect();
    if !mpl_shipped.is_empty() && !archived.contains("Mozilla Public License Version 2.0") {
        problems.push(format!("{NOTICES} lacks the MPL-2.0 licence text"));
    }
    for (name, version) in mpl_shipped {
        let source = format!("https://crates.io/crates/{name}/{version}");
        if !archived.contains(&source) {
            problems.push(format!("{NOTICES} gives no source location {source}"));
        }
    }
    problems
}

fn verify_notices(
    archived: &str,
    fresh: &str,
    shipped: &BTreeSet<(String, String)>,
) -> Result<(), String> {
    let problems = notices_problems(archived, fresh, shipped);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

// ---------------------------------------------------------------------------------------------
// Packaging

/// Extra `tar` flags that make the archive independent of the builder's user: GNU tar and
/// bsdtar (macOS) spell them differently.
fn tar_owner_flags(version_output: &str) -> Vec<&'static str> {
    if version_output.contains("GNU tar") {
        vec!["--owner=0", "--group=0", "--numeric-owner"]
    } else {
        vec!["--uid", "0", "--gid", "0", "--numeric-owner"]
    }
}

fn package(
    root: &Path,
    out_dir: &Path,
    target: &str,
    binary: &Path,
    notices: &str,
) -> Result<PathBuf, String> {
    let top = format!("site-preview-{target}");
    let stage = out_dir.join("stage");
    let dir = stage.join(&top);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let copy = |from: &Path, name: &str| {
        fs::copy(from, dir.join(name))
            .map(drop)
            .map_err(|e| format!("cannot copy {}: {e}", from.display()))
    };
    copy(binary, "site-preview")?;
    for name in ["README.md", "LICENSE-MIT", "LICENSE-APACHE"] {
        copy(&root.join(name), name)?;
    }
    fs::write(dir.join(NOTICES), notices).map_err(|e| format!("cannot write {NOTICES}: {e}"))?;

    let archive = out_dir.join(archive_name(target));
    let version = Command::new("tar")
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    let status = Command::new("tar")
        .args(tar_owner_flags(&String::from_utf8_lossy(&version.stdout)))
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage)
        .arg(&top)
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err("tar could not create the archive".to_owned());
    }
    fs::remove_dir_all(&stage).map_err(|e| format!("cannot remove {}: {e}", stage.display()))?;
    Ok(archive)
}

/// Extracts the archive below `scratch` and returns its top directory, after checking it holds
/// exactly [`ARCHIVE_FILES`].
fn extract(archive: &Path, scratch: &Path, target: &str) -> Result<PathBuf, String> {
    let into = scratch.join("extract");
    fs::create_dir_all(&into).map_err(|e| format!("cannot create {}: {e}", into.display()))?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(&into)
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err("tar could not extract the archive".to_owned());
    }
    let top = into.join(format!("site-preview-{target}"));
    let mut found: Vec<String> = fs::read_dir(&top)
        .map_err(|e| format!("the archive has no site-preview-{target}/ directory: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    let mut want: Vec<String> = ARCHIVE_FILES.iter().map(|s| (*s).to_owned()).collect();
    want.sort();
    if found != want {
        return Err(format!("the archive holds {found:?}, expected {want:?}"));
    }
    Ok(top)
}

// ---------------------------------------------------------------------------------------------
// The smoke run

/// A spawned server, killed and reaped by PID when dropped, so no failure path leaves one.
struct Spawned {
    child: Child,
}

impl Drop for Spawned {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Names from `programs` that exist as files in a directory of the colon-separated `path`.
fn programs_on_path(path: &str, programs: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        for program in programs {
            let candidate = Path::new(dir).join(program);
            if candidate.exists() {
                found.push(candidate.display().to_string());
            }
        }
    }
    found
}

fn assert_clean_path() -> Result<(), String> {
    let found = programs_on_path(CLEAN_PATH, &FORBIDDEN_PROGRAMS);
    if found.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} is on PATH={CLEAN_PATH}, so the smoke would not prove the binary stands alone: {}",
            FORBIDDEN_PROGRAMS.join(", "),
            found.join(", ")
        ))
    }
}

/// A command for the extracted binary with the clean environment (`env -i PATH=...`).
fn clean_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command.env_clear().env("PATH", CLEAN_PATH);
    command
}

/// Relative path (with `/`) to bytes of every file below `root`.
type Snapshot = BTreeMap<String, Vec<u8>>;

fn snapshot(root: &Path) -> Result<Snapshot, String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Snapshot) -> Result<(), String> {
        let entries =
            fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            let kind = entry
                .file_type()
                .map_err(|e| format!("cannot stat {name}: {e}"))?;
            if kind.is_dir() {
                walk(&entry.path(), &format!("{name}/"), out)?;
            } else if kind.is_file() {
                let bytes =
                    fs::read(entry.path()).map_err(|e| format!("cannot read {name}: {e}"))?;
                out.insert(name, bytes);
            } else {
                return Err(format!("{name} is neither a file nor a directory"));
            }
        }
        Ok(())
    }
    let mut out = Snapshot::new();
    walk(root, "", &mut out)?;
    Ok(out)
}

/// Writes `files` below `root`, creating directories.
fn write_snapshot(root: &Path, files: &Snapshot) -> Result<(), String> {
    for (name, bytes) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Differences between two snapshots, empty when they are equal.
fn snapshot_diff(expected: &Snapshot, actual: &Snapshot) -> Vec<String> {
    let mut diff = Vec::new();
    for (name, bytes) in expected {
        match actual.get(name) {
            None => diff.push(format!("{name} was removed")),
            Some(now) if now != bytes => diff.push(format!("{name} changed")),
            Some(_) => {}
        }
    }
    for name in actual.keys().filter(|n| !expected.contains_key(*n)) {
        diff.push(format!("{name} was added"));
    }
    diff
}

fn smoke(root: &Path, binary: &Path, scratch: &Path, logs: &Path) -> Result<(), String> {
    assert_clean_path()?;

    let version = clean_command(binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {} --version: {e}", binary.display()))?;
    let version_text = String::from_utf8_lossy(&version.stdout);
    let wanted = format!("site-preview {}", env!("CARGO_PKG_VERSION"));
    if !version.status.success() || version_text.trim() != wanted {
        return Err(format!(
            "--version printed '{}' (exit {:?}), expected '{wanted}'",
            version_text.trim(),
            version.status.code()
        ));
    }

    let fixture = root.join("tests/fixtures/site");
    let original = snapshot(&fixture)?;
    let site = scratch.join("site");
    write_snapshot(&site, &original)?;
    let mut expected = original.clone();

    let stderr_log = fs::File::create(logs.join("stderr.log"))
        .map_err(|e| format!("cannot create the stderr log: {e}"))?;
    let mut server = Spawned {
        child: clean_command(binary)
            .args(["serve", "--json", "--port", "0"])
            .current_dir(&site)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr_log)
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", binary.display()))?,
    };
    let stdout = server
        .child
        .stdout
        .take()
        .ok_or("the server's stdout is not piped")?;
    let lines = spawn_line_reader(stdout, logs.join("stdout.log"))?;

    let ready = lines
        .recv_timeout(DEADLINE)
        .map_err(|e| format!("no `ready` line within {DEADLINE:?}: {e}"))?;
    let url = parse_ready_url(&ready).ok_or_else(|| format!("not a `ready` event: {ready}"))?;
    let authority = authority_of(&url).ok_or_else(|| format!("no authority in {url}"))?;

    let (status, body) = http_get(&authority, "/")?;
    let page = String::from_utf8_lossy(&body).to_lowercase();
    if status != 200 || !page.contains("<html") || !page.contains("__site_preview") {
        return Err(format!(
            "GET / answered {status}; the page must be HTML with the reload client injected"
        ));
    }

    let mut events = match SseClient::open(&authority)? {
        Opened::Stream(client) => client,
        Opened::Refused(status) => return Err(format!("the event stream answered {status}")),
    };
    events.wait_for("state")?;
    let edited = edit_until_reload(&site, &mut expected, &mut events)?;
    eprintln!("release-check: edited {edited}; received a reload event");

    terminate(&mut server)?;
    let after = snapshot(&site)?;
    let diff = snapshot_diff(&expected, &after);
    if !diff.is_empty() {
        return Err(format!(
            "serving changed the site tree: {}",
            diff.join(", ")
        ));
    }
    if snapshot(&fixture)? != original {
        return Err("the repository fixture changed".to_owned());
    }
    Ok(())
}

/// Forwards the child's stdout lines to a channel and a log file, until it closes.
fn spawn_line_reader(
    stdout: impl Read + Send + 'static,
    log: PathBuf,
) -> Result<mpsc::Receiver<String>, String> {
    use std::io::{BufRead, BufReader};
    let mut file =
        fs::File::create(&log).map_err(|e| format!("cannot create the stdout log: {e}"))?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = writeln!(file, "{line}");
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    Ok(receiver)
}

/// Appends a comment to the fixture's `index.html` until a `reload` event arrives. The watcher
/// may still be registering right after `ready`, so an edit nobody saw is repeated (a bounded
/// number of times) rather than waited on for the whole deadline.
fn edit_until_reload(
    site: &Path,
    expected: &mut Snapshot,
    events: &mut SseClient,
) -> Result<&'static str, String> {
    const FILE: &str = "index.html";
    const ATTEMPTS: u32 = 3;
    for attempt in 1..=ATTEMPTS {
        let page = expected
            .get_mut(FILE)
            .ok_or("the fixture has no index.html")?;
        page.extend_from_slice(format!("\n<!-- release-check edit {attempt} -->\n").as_bytes());
        fs::write(site.join(FILE), &*page).map_err(|e| format!("cannot edit {FILE}: {e}"))?;
        if events
            .wait_for_within("reload", Duration::from_secs(10))
            .is_ok()
        {
            return Ok(FILE);
        }
    }
    Err(format!(
        "no `reload` event after {ATTEMPTS} edits of {FILE}"
    ))
}

/// SIGTERM by PID, then waits for exit 0.
fn terminate(server: &mut Spawned) -> Result<(), String> {
    let pid = server.child.id();
    let sent = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|e| format!("cannot run kill: {e}"))?;
    if !sent.success() {
        return Err(format!("kill -TERM {pid} failed"));
    }
    let deadline = Instant::now() + DEADLINE;
    loop {
        match server.child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!("the server exited with {status} after SIGTERM"));
            }
            Ok(None) if Instant::now() < deadline => sleep(Duration::from_millis(50)),
            Ok(None) => {
                return Err(format!(
                    "the server did not exit within {DEADLINE:?} of SIGTERM"
                ));
            }
            Err(e) => return Err(format!("cannot wait for the server: {e}")),
        }
    }
}

/// A plain `GET` over TCP: the status and the body.
fn http_get(authority: &str, path: &str) -> Result<(u16, Vec<u8>), String> {
    let mut stream =
        TcpStream::connect(authority).map_err(|e| format!("cannot connect to {authority}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("timeout: {e}"))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("cannot send the request: {e}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("cannot read the response: {e}"))?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the response has no header end")?;
    let head = String::from_utf8_lossy(&response[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("bad status line in '{head}'"))?;
    Ok((status, response[split + 4..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        items
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    const TREE: &str = "\
preview-cli v0.1.0 (/work/crates/preview-cli)
clap v4.6.7
serde_derive v1.0.229 (proc-macro)
selectors v0.36.1
preview-domain v0.1.0 (/work/crates/preview-domain)
clap v4.6.7 (*)
";

    fn notices_for(entries: &[(&str, &str)]) -> String {
        let mut text = String::from("Mozilla Public License Version 2.0\n");
        for (name, version) in entries {
            text.push_str(&format!(
                "* {name} {version} (https://crates.io/crates/{name}/{version})\n"
            ));
        }
        text
    }

    #[test]
    fn should_name_the_archive_after_the_target_when_packaging() {
        // given
        let target = "aarch64-unknown-linux-musl";

        // when
        let name = archive_name(target);

        // then
        assert_eq!(name, "site-preview-aarch64-unknown-linux-musl.tar.gz");
    }

    #[test]
    fn should_cover_the_four_plan_targets_when_release_targets_are_listed() {
        // given
        let names: Vec<_> = RELEASE_TARGETS.iter().map(|t| archive_name(t)).collect();

        // when
        let expected = [
            "site-preview-aarch64-apple-darwin.tar.gz",
            "site-preview-x86_64-apple-darwin.tar.gz",
            "site-preview-aarch64-unknown-linux-musl.tar.gz",
            "site-preview-x86_64-unknown-linux-musl.tar.gz",
        ];

        // then
        assert_eq!(names, expected);
    }

    #[test]
    fn should_refuse_a_path_like_target_when_validating() {
        // given
        let bad = ["", "../x", "a/b", "x y", ".hidden"];

        // when
        let results: Vec<_> = bad.iter().map(|t| validate_target(t).is_ok()).collect();

        // then
        assert!(results.iter().all(|ok| !ok), "{results:?}");
        assert!(validate_target("x86_64-unknown-linux-musl").is_ok());
    }

    #[test]
    fn should_read_the_host_triple_when_rustc_verbose_output_is_parsed() {
        // given
        let text = "rustc 1.97.0\nbinary: rustc\nhost: x86_64-unknown-linux-gnu\nrelease: 1.97.0\n";

        // when
        let host = parse_host(text);

        // then
        assert_eq!(host.as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(parse_host("no host here"), None);
    }

    #[test]
    fn should_skip_workspace_members_and_repeats_when_the_tree_is_parsed() {
        // given
        let tree = TREE;

        // when
        let shipped = parse_shipped(tree);

        // then
        assert_eq!(
            shipped,
            set(&[
                ("clap", "4.6.7"),
                ("serde_derive", "1.0.229"),
                ("selectors", "0.36.1")
            ])
        );
    }

    #[test]
    fn should_accept_notices_when_every_shipped_crate_is_listed() {
        // given
        let shipped = parse_shipped(TREE);
        let notices = notices_for(&[
            ("clap", "4.6.7"),
            ("serde_derive", "1.0.229"),
            ("selectors", "0.36.1"),
        ]);

        // when
        let problems = notices_problems(&notices, &notices, &shipped);

        // then
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn should_fail_when_a_crate_entry_is_deleted_from_the_archived_notices() {
        // given
        let shipped = parse_shipped(TREE);
        let fresh = notices_for(&[
            ("clap", "4.6.7"),
            ("serde_derive", "1.0.229"),
            ("selectors", "0.36.1"),
        ]);
        let archived: String = fresh
            .lines()
            .filter(|line| !line.starts_with("* clap "))
            .map(|line| format!("{line}\n"))
            .collect();

        // when
        let problems = notices_problems(&archived, &fresh, &shipped);

        // then
        assert!(
            problems
                .iter()
                .any(|p| p.contains("no entry for clap 4.6.7")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("differs from a fresh")),
            "{problems:?}"
        );
    }

    #[test]
    fn should_fail_when_a_listed_crate_is_missing_even_if_the_copy_is_fresh() {
        // given
        let shipped = parse_shipped(TREE);
        let notices = notices_for(&[("clap", "4.6.7"), ("selectors", "0.36.1")]);

        // when
        let problems = notices_problems(&notices, &notices, &shipped);

        // then
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("serde_derive 1.0.229"));
    }

    #[test]
    fn should_fail_when_an_mpl_crate_has_no_licence_text_or_source() {
        // given
        let shipped = set(&[("selectors", "0.36.1")]);
        let notices = "* selectors 0.36.1\n";

        // when
        let problems = notices_problems(notices, notices, &shipped);

        // then
        assert!(problems.iter().any(|p| p.contains("MPL-2.0 licence text")));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("https://crates.io/crates/selectors/0.36.1"))
        );
    }

    #[test]
    fn should_not_demand_mpl_text_when_no_mpl_crate_ships() {
        // given
        let shipped = set(&[("clap", "4.6.7")]);
        let notices = "* clap 4.6.7\n";

        // when
        let problems = notices_problems(notices, notices, &shipped);

        // then
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn should_pick_owner_flags_for_the_tar_flavour_when_tar_is_versioned() {
        // given
        let gnu = "tar (GNU tar) 1.35";
        let bsd = "bsdtar 3.5.3 - libarchive 3.5.3";

        // when
        let (gnu_flags, bsd_flags) = (tar_owner_flags(gnu), tar_owner_flags(bsd));

        // then
        assert!(gnu_flags.contains(&"--owner=0"));
        assert!(bsd_flags.contains(&"--uid"));
    }

    #[test]
    fn should_report_programs_found_when_path_holds_them() {
        // given
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("node"), b"").unwrap();
        let path = dir.path().display().to_string();

        // when
        let found = programs_on_path(&path, &FORBIDDEN_PROGRAMS);
        let none = programs_on_path(&path, &["cargo", "npm"]);

        // then
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("node"));
        assert!(none.is_empty());
    }

    #[test]
    fn should_detect_changed_added_and_removed_files_when_snapshots_differ() {
        // given
        let mut before = Snapshot::new();
        before.insert("a".into(), b"1".to_vec());
        before.insert("b".into(), b"2".to_vec());
        let mut after = Snapshot::new();
        after.insert("a".into(), b"x".to_vec());
        after.insert("c".into(), b"3".to_vec());

        // when
        let diff = snapshot_diff(&before, &after);

        // then
        assert_eq!(diff, ["a changed", "b was removed", "c was added"]);
        assert!(snapshot_diff(&before, &before).is_empty());
    }

    #[test]
    fn should_round_trip_a_tree_when_it_is_written_and_read_back() {
        // given
        let dir = tempfile::tempdir().unwrap();
        let mut files = Snapshot::new();
        files.insert("index.html".into(), b"<html>".to_vec());
        files.insert("guide/page.html".into(), b"<p>".to_vec());

        // when
        write_snapshot(dir.path(), &files).unwrap();
        let read = snapshot(dir.path()).unwrap();

        // then
        assert_eq!(read, files);
    }

    #[test]
    fn should_parse_status_and_body_when_a_response_is_read() {
        // given
        let raw = b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\n\r\n<html>";

        // when
        let parsed = parse_response(raw).unwrap();

        // then
        assert_eq!(parsed, (200, b"<html>".to_vec()));
        assert!(parse_response(b"garbage").is_err());
    }

    #[test]
    fn should_run_cargo_about_from_cargo_when_the_tool_is_pinned() {
        // given
        let tools = crate::tools::manifest().unwrap();

        // when
        let about = tools.iter().find(|t| t.name == "cargo-about");

        // then
        let about = about.expect("cargo-about is pinned in tools/versions.toml");
        assert_eq!(about.method, crate::tools::Method::Cargo);
        assert_eq!(about.probe, ["cargo-about", "about", "--version"]);
        assert_eq!(about.features.as_deref(), Some("cli"));
    }

    #[test]
    fn should_give_an_install_hint_when_cargo_about_is_missing() {
        // given
        let empty = tempfile::tempdir().unwrap();

        // when
        let result = crate::tools::require_on("cargo-about", empty.path().as_os_str());

        // then
        let message = result.unwrap_err();
        assert!(
            message.contains("cargo xtask install-tools cargo-about"),
            "{message}"
        );
    }
}
