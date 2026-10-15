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
//! The notices are `cargo about`'s output followed by fixed sections for what the compiler adds
//! outside Cargo's graph: the Rust standard library on every target and musl libc on the
//! `*-linux-musl` targets (`tools/notices/`, ADR 0076). They are checked four ways, each failing
//! the command: the archived copy must equal a fresh run, every crate of the shipped graph
//! (`cargo tree -e normal` for the target) must have an entry in it, the four MPL-2.0 crates of
//! ADR 0043 must come with the licence text and a source location (MPL-2.0 section 3.2), and the
//! fixed sections the target needs must be present. A missing `cargo-about` is an install hint,
//! never a skip (ADR 0010). OPS-06 calls this once per native runner.
//!
//! The build and the archive are reproducible from the commit (ADR 0078): the build remaps the
//! checkout and `$CARGO_HOME` out of the binary (and fails if either path is still in it), and
//! the archive is a ustar of sorted entries with fixed modes, owner 0 and the commit time
//! (`SOURCE_DATE_EPOCH` when set), compressed with `gzip -n`.
//!
//! `--archive PATH` (with `--target T`) skips the build and the notices and runs the extraction
//! check and the smoke on an existing archive, such as one read back from a draft release by
//! OPS-06's `verify` job (ADR 0078).

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

/// The Rust standard library's notice, appended to every target's notices (ADR 0076).
const RUST_STD_NOTICE: &str = include_str!("../../notices/rust-std.txt");

/// musl libc's notice (its `COPYRIGHT`, MIT), appended for the `*-linux-musl` targets, whose
/// binaries link musl statically: the MIT terms require the notice in every copy.
const MUSL_NOTICE: &str = include_str!("../../notices/musl-libc.txt");

/// The heading line of [`RUST_STD_NOTICE`], which `release-check` requires in every archive.
const RUST_STD_HEADING: &str = "The Rust standard library (MIT OR Apache-2.0)";

/// The heading line of [`MUSL_NOTICE`], required in the archive of a `*-linux-musl` target.
const MUSL_HEADING: &str = "musl libc 1.2.5 (MIT)";

/// musl's copyright line, which must ship with a static musl binary.
const MUSL_COPYRIGHT: &str = "Copyright \u{a9} 2005-2020 Rich Felker, et al.";

/// Runs the command and returns its exit code. With `archive`, the build and packaging are
/// skipped and that existing archive is extracted and smoked instead (OPS-06's `verify` job).
pub fn run(target: Option<&str>, archive: Option<&Path>) -> i32 {
    let result = match archive {
        Some(archive) => check_archive(target, archive),
        None => check(target),
    };
    match result {
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

    let epoch = crate::sbom::source_date_epoch(&root)?;
    let cargo_home = cargo_home()?;
    let binary = build(&root, &cargo_home, &target)?;
    let leaked = build_path_problems(
        &fs::read(&binary).map_err(|e| format!("cannot read {}: {e}", binary.display()))?,
        &[&root, &cargo_home],
    );
    if !leaked.is_empty() {
        return Err(format!(
            "the binary still names a build path: {}",
            leaked.join(", ")
        ));
    }
    let shipped = shipped_crates(&root, &target)?;
    let notices = full_notices(&root, &target, &logs.join("notices.log"))?;
    let archive = package(&root, &out_dir, &target, &binary, &notices, epoch)?;
    summary.push(format!("target {target}"));
    summary.push(
        "binary: names neither the checkout nor $CARGO_HOME (--remap-path-prefix)".to_owned(),
    );
    summary.push(format!(
        "archive {} (ustar, sorted, mtime {epoch}, owner 0, gzip -n)",
        archive.display()
    ));

    let scratch = tempfile::tempdir().map_err(|e| format!("cannot make a temp dir: {e}"))?;
    let extracted = extract(&archive, scratch.path(), &target)?;
    summary.push(format!("members {}", ARCHIVE_FILES.join(", ")));

    let archived = fs::read_to_string(extracted.join(NOTICES))
        .map_err(|e| format!("cannot read the archived {NOTICES}: {e}"))?;
    let fresh = full_notices(&root, &target, &logs.join("notices-fresh.log"))?;
    verify_notices(&archived, &fresh, &shipped, &target)?;
    let mpl = MPL_CRATES
        .iter()
        .filter(|name| shipped.iter().any(|(n, _)| n == *name))
        .count();
    summary.push(format!(
        "{NOTICES}: matches a fresh cargo-about run; {} shipped crates listed; {mpl} MPL-2.0 crates with licence text and source; fixed sections: {}",
        shipped.len(),
        fixed_section_headings(&target).join(", ")
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

/// `release-check --target T --archive PATH`: the smoke of an archive built elsewhere, such as
/// one downloaded from a draft release. Nothing is built and `cargo-about` is not needed; the
/// archive must carry the name of `T`, hold exactly [`ARCHIVE_FILES`] below
/// `site-preview-T/`, and its extracted binary must pass the same [`smoke`] as a fresh build.
/// The notices are not regenerated here: the release's `SHA256SUMS` ties the archive to the
/// build job that checked them.
fn check_archive(target_arg: Option<&str>, archive: &Path) -> Result<Vec<String>, String> {
    let root = crate::repo::root();
    let target = target_arg.ok_or("--archive needs --target, the triple the archive is for")?;
    validate_target(target)?;
    archive_name_problem(target, archive).map_or(Ok(()), Err)?;
    if !archive.is_file() {
        return Err(format!("{} is not a file", archive.display()));
    }
    let logs = root
        .join(target_dir_name())
        .join("release-check")
        .join(target)
        .join("logs");
    // Only the logs are reset: the archive itself may live in the target's output directory.
    reset_dir(&logs)?;

    let scratch = tempfile::tempdir().map_err(|e| format!("cannot make a temp dir: {e}"))?;
    let extracted = extract(archive, scratch.path(), target)?;
    smoke(
        &root,
        &extracted.join("site-preview"),
        scratch.path(),
        &logs,
    )?;
    Ok(vec![
        format!("target {target}"),
        format!("archive {} (existing, not rebuilt)", archive.display()),
        format!("members {}", ARCHIVE_FILES.join(", ")),
        "smoke: --version, serve --json --port 0, GET /, reload after an edit, SIGTERM exit 0, tree unchanged"
            .to_owned(),
    ])
}

/// Why `archive` cannot be the archive of `target`, if it cannot: its file name must be
/// exactly [`archive_name`], so an archive of one platform is never smoked as another's.
fn archive_name_problem(target: &str, archive: &Path) -> Option<String> {
    let want = archive_name(target);
    match archive.file_name().and_then(|n| n.to_str()) {
        Some(name) if name == want => None,
        Some(name) => Some(format!(
            "the archive for {target} is named {want}, not {name}"
        )),
        None => Some(format!("{} names no archive file", archive.display())),
    }
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

/// Linux musl builds link with the musl C library the Rust target ships (self-contained
/// `libc.a` and start files) through the system `cc`; no `musl-gcc` is needed while no
/// dependency compiles C (`Cargo.lock` has no `cc` crate; ADR 0078). A C-backed dependency would
/// bring `musl-tools` back, and this hint says so.
fn musl_hint(target: &str) -> &'static str {
    if target.ends_with("-musl") {
        " (a dependency that compiles C needs 'sudo apt-get install musl-tools', see ADR 0078)"
    } else {
        ""
    }
}

/// `$CARGO_HOME`, or `$HOME/.cargo` as Cargo defaults it.
fn cargo_home() -> Result<PathBuf, String> {
    if let Some(home) = std::env::var_os("CARGO_HOME").filter(|h| !h.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".cargo"))
        .ok_or_else(|| "neither CARGO_HOME nor HOME is set".to_owned())
}

/// The rustc flags of the release build: the registry sources under `$CARGO_HOME` and the
/// checkout are remapped to fixed names, so the binary carries no path of the machine that
/// built it (panic locations, debug info) and two builders produce the same bytes. Passed as
/// `CARGO_ENCODED_RUSTFLAGS` (separated by 0x1f), so a path with a space stays one argument
/// (`should_remap_cargo_home_and_the_checkout_when_release_flags_are_built`).
fn remap_flags(root: &Path, cargo_home: &Path) -> String {
    [
        format!("--remap-path-prefix={}=/cargo", cargo_home.display()),
        format!("--remap-path-prefix={}=/build", root.display()),
    ]
    .join("\u{1f}")
}

/// Each of `paths` that still occurs in `bytes`, the binary: a remap that missed one.
fn build_path_problems(bytes: &[u8], paths: &[&Path]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .filter(|p| !p.is_empty() && bytes.windows(p.len()).any(|w| w == p.as_bytes()))
        .collect()
}

fn build(root: &Path, cargo_home: &Path, target: &str) -> Result<PathBuf, String> {
    eprintln!("release-check: building site-preview for {target}");
    let status = Command::new("cargo")
        // Only the remap flags: the release build takes no RUSTFLAGS from the environment.
        .env("CARGO_ENCODED_RUSTFLAGS", remap_flags(root, cargo_home))
        .env_remove("RUSTFLAGS")
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
pub fn shipped_crates(root: &Path, target: &str) -> Result<BTreeSet<(String, String)>, String> {
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

/// The arguments of the one network step before the notices: every crate of `Cargo.lock`.
///
/// `cargo metadata`, which `cargo about` runs, resolves the whole lockfile, while the release
/// build downloads only the target's normal graph; on a cold registry cache `--offline` then
/// fails. Fetching first keeps the generator offline and deterministic, and this is the only step
/// that touches the network (`should_fetch_the_lockfile_before_generating_offline_when_notices_are_made`).
fn fetch_args() -> Vec<&'static str> {
    vec!["fetch", "--locked"]
}

/// The arguments of `cargo about generate` for `target`.
fn generate_args(target: &str) -> Vec<&str> {
    vec![
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
    ]
}

/// The complete `THIRD-PARTY-NOTICES` for `target`: `cargo about`'s output, then the fixed
/// sections of [`fixed_sections`]. The generator's stderr goes to `log` (`logs/`), kept on
/// failure by the workflow.
fn full_notices(root: &Path, target: &str, log: &Path) -> Result<String, String> {
    let mut text = generate_notices(root, target, log)?;
    text.push_str(&fixed_sections(target));
    Ok(text)
}

/// The sections no Cargo metadata describes: the Rust standard library, built into every
/// binary, and musl libc, linked statically into the `*-linux-musl` ones.
fn fixed_sections(target: &str) -> String {
    let mut text = format!("\n{RUST_STD_NOTICE}");
    if is_musl(target) {
        text.push_str(&format!("\n{MUSL_NOTICE}"));
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The headings of [`fixed_sections`] for `target`.
fn fixed_section_headings(target: &str) -> Vec<&'static str> {
    if is_musl(target) {
        vec![RUST_STD_HEADING, MUSL_HEADING]
    } else {
        vec![RUST_STD_HEADING]
    }
}

fn is_musl(target: &str) -> bool {
    target.ends_with("-linux-musl")
}

/// Runs `command`, appending its stderr to `log`, and returns its stdout. A failure names the
/// log and quotes the end of the stderr.
fn run_logged(mut command: Command, what: &str, log: &Path) -> Result<Vec<u8>, String> {
    let output = command
        .output()
        .map_err(|e| format!("cannot run {what}: {e}"))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| format!("cannot open {}: {e}", log.display()))?;
    writeln!(file, "$ {what}").map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    file.write_all(&output.stderr)
        .map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        Err(format!(
            "{what} failed (stderr in {}): {}",
            log.display(),
            tail.join("\n").trim()
        ))
    }
}

/// The notices `cargo about` generates for `target`, from `about.toml` and `about.hbs`.
fn generate_notices(root: &Path, target: &str, log: &Path) -> Result<String, String> {
    let mut fetch = Command::new("cargo");
    fetch.args(fetch_args()).current_dir(root);
    run_logged(fetch, "cargo fetch", log)?;
    let mut about = Command::new("cargo");
    about.args(generate_args(target)).current_dir(root);
    let stdout = run_logged(about, "cargo about generate", log)?;
    let mut text =
        String::from_utf8(stdout).map_err(|e| format!("cargo about printed invalid UTF-8: {e}"))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// The `(name, version)` of every entry line of the notices: exactly
/// `* <name> <version> https://crates.io/crates/<name>/<version>`, so prose that happens to
/// start with `* ` is never counted as an entry
/// (`should_count_only_entries_with_their_crates_io_address_when_notices_are_read`).
fn listed_crates(notices: &str) -> BTreeSet<(String, String)> {
    notices
        .lines()
        .filter_map(|line| line.strip_prefix("* "))
        .filter_map(|rest| {
            let mut words = rest.split_whitespace();
            let (name, version, source) = (words.next()?, words.next()?, words.next()?);
            let anchored = source == format!("https://crates.io/crates/{name}/{version}")
                && words.next().is_none();
            anchored.then(|| (name.to_owned(), version.to_owned()))
        })
        .collect()
}

/// Everything wrong with the archived notices, empty when they are sound.
fn notices_problems(
    archived: &str,
    fresh: &str,
    shipped: &BTreeSet<(String, String)>,
    target: &str,
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
    let lines: BTreeSet<&str> = archived.lines().map(str::trim_end).collect();
    for heading in fixed_section_headings(target) {
        if !lines.contains(heading) {
            problems.push(format!("{NOTICES} lacks the section '{heading}'"));
        }
    }
    if is_musl(target) && !archived.contains(MUSL_COPYRIGHT) {
        problems.push(format!("{NOTICES} lacks musl's copyright notice"));
    }
    problems
}

fn verify_notices(
    archived: &str,
    fresh: &str,
    shipped: &BTreeSet<(String, String)>,
    target: &str,
) -> Result<(), String> {
    let problems = notices_problems(archived, fresh, shipped, target);
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

/// The `tar` arguments that write `entries` (already sorted) into `tar_path`, flavour-neutral:
/// ustar (no extended headers, so no atime, ctime or xattrs), no recursion (the entry order is
/// the given one, not the file system's), owner 0 (`tar_owner_flags`).
fn tar_create_args(
    owner_flags: &[&'static str],
    tar_path: &Path,
    stage: &Path,
    entries: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = owner_flags.iter().map(|f| (*f).to_owned()).collect();
    args.extend(
        ["--format=ustar", "--no-recursion", "--no-xattrs", "-cf"]
            .iter()
            .map(|f| (*f).to_owned()),
    );
    args.push(tar_path.display().to_string());
    args.push("-C".to_owned());
    args.push(stage.display().to_string());
    args.extend(entries.iter().cloned());
    args
}

/// Sets the mode and modification time of a staged entry, so the archive does not depend on the
/// builder's umask or clock.
fn normalise_entry(path: &Path, mode: u32, mtime: std::time::SystemTime) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("cannot chmod {}: {e}", path.display()))?;
    fs::File::open(path)
        .and_then(|f| f.set_modified(mtime))
        .map_err(|e| format!("cannot set the time of {}: {e}", path.display()))
}

/// Packs `site-preview-<target>/` reproducibly: every entry has mode 0755 (directory, binary)
/// or 0644, owner 0, the modification time `epoch` (the commit time, or `SOURCE_DATE_EPOCH`),
/// in sorted order, and gzip records no name or time (`-n`). The same inputs give the same
/// bytes on any machine (`should_write_the_same_bytes_when_the_same_inputs_are_packaged_twice`).
fn package(
    root: &Path,
    out_dir: &Path,
    target: &str,
    binary: &Path,
    notices: &str,
    epoch: u64,
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

    let mtime = std::time::UNIX_EPOCH + Duration::from_secs(epoch);
    let mut files: Vec<&str> = ARCHIVE_FILES.to_vec();
    files.sort_unstable();
    for name in &files {
        let mode = if *name == "site-preview" {
            0o755
        } else {
            0o644
        };
        normalise_entry(&dir.join(name), mode, mtime)?;
    }
    // Last, so writing the files does not move the directory's time again.
    normalise_entry(&dir, 0o755, mtime)?;
    let mut entries = vec![top.clone()];
    entries.extend(files.iter().map(|name| format!("{top}/{name}")));

    let archive = out_dir.join(archive_name(target));
    let tar_path = out_dir.join(format!("site-preview-{target}.tar"));
    let version = Command::new("tar")
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    let owner = tar_owner_flags(&String::from_utf8_lossy(&version.stdout));
    let status = Command::new("tar")
        .args(tar_create_args(&owner, &tar_path, &stage, &entries))
        // macOS: no AppleDouble `._` entries.
        .env("COPYFILE_DISABLE", "1")
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err("tar could not create the archive".to_owned());
    }
    let status = Command::new("gzip")
        .args(["-n", "-9", "-f"])
        .arg(&tar_path)
        .status()
        .map_err(|e| format!("cannot run gzip: {e}"))?;
    if !status.success() || !archive.is_file() {
        return Err("gzip could not compress the archive".to_owned());
    }
    fs::remove_dir_all(&stage).map_err(|e| format!("cannot remove {}: {e}", stage.display()))?;
    Ok(archive)
}

/// What is wrong with an archive's `tar -tvzf` listing, empty when it holds exactly one
/// directory `top` and the regular files [`ARCHIVE_FILES`] in it. The type is the first
/// character of the mode column (`d`, `-`; a symlink is `l`, a hard link `h` or a second `-`
/// line naming its target), the name is the last field; GNU tar and bsdtar both list this way.
/// A name with a space, an absolute or `..` path, an extra top-level entry or a second copy of
/// a file all fail (`should_refuse_a_listing_when_it_holds_a_symlink_or_an_extra_entry`).
fn listing_problems(listing: &str, top: &str) -> Vec<String> {
    let mut want: BTreeSet<(char, String)> = ARCHIVE_FILES
        .iter()
        .map(|name| ('-', format!("{top}/{name}")))
        .collect();
    want.insert(('d', top.to_owned()));
    let mut seen = Vec::new();
    let mut problems = Vec::new();
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        let kind = line.chars().next().unwrap_or(' ');
        let name = line.split_whitespace().last().unwrap_or("");
        let name = name
            .strip_prefix("./")
            .unwrap_or(name)
            .trim_end_matches('/');
        if kind != 'd' && kind != '-' {
            problems.push(format!(
                "{name} is not a regular file or a directory ({line})"
            ));
        }
        seen.push((kind, name.to_owned()));
    }
    let found: BTreeSet<(char, String)> = seen.iter().cloned().collect();
    if found.len() != seen.len() {
        problems.push("an entry is listed twice".to_owned());
    }
    for (kind, name) in found.difference(&want) {
        problems.push(format!("unexpected entry {name} ({kind})"));
    }
    for (kind, name) in want.difference(&found) {
        problems.push(format!("missing entry {name} ({kind})"));
    }
    problems
}

/// Lists the archive, checks the listing ([`listing_problems`]), extracts it below `scratch`,
/// checks the extracted tree again (one top directory, exactly [`ARCHIVE_FILES`], each a
/// regular file) and returns the top directory.
fn extract(archive: &Path, scratch: &Path, target: &str) -> Result<PathBuf, String> {
    let top_name = format!("site-preview-{target}");
    let listed = Command::new("tar")
        .arg("-tvzf")
        .arg(archive)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !listed.status.success() {
        return Err("tar could not list the archive".to_owned());
    }
    let problems = listing_problems(&String::from_utf8_lossy(&listed.stdout), &top_name);
    if !problems.is_empty() {
        return Err(format!("the archive is malformed: {}", problems.join("; ")));
    }
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
    let names = |dir: &Path| -> Result<Vec<String>, String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Ok(names)
    };
    let tops = names(&into)?;
    if tops != [top_name.clone()] {
        return Err(format!(
            "the archive's top level holds {tops:?}, expected [{top_name:?}]"
        ));
    }
    let top = into.join(&top_name);
    let found = names(&top)?;
    let mut want: Vec<String> = ARCHIVE_FILES.iter().map(|s| (*s).to_owned()).collect();
    want.sort();
    if found != want {
        return Err(format!("the archive holds {found:?}, expected {want:?}"));
    }
    for name in &found {
        let meta =
            fs::symlink_metadata(top.join(name)).map_err(|e| format!("cannot stat {name}: {e}"))?;
        if !meta.file_type().is_file() {
            return Err(format!("{name} in the archive is not a regular file"));
        }
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

    let url = wait_for_ready(&lines, DEADLINE)?;
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

/// The URL of the first `ready` event among the server's stdout lines. JSON lines before it
/// (a warning, say) are skipped; a line that is not JSON, or no `ready` within `deadline`,
/// fails (`should_skip_json_warnings_when_waiting_for_ready`).
fn wait_for_ready(lines: &mpsc::Receiver<String>, deadline: Duration) -> Result<String, String> {
    let until = Instant::now() + deadline;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        let line = lines
            .recv_timeout(left)
            .map_err(|e| format!("no `ready` line within {deadline:?}: {e}"))?;
        if let Some(url) = parse_ready_url(&line) {
            return Ok(url);
        }
        if !line.trim_start().starts_with('{') {
            return Err(format!("a line before `ready` is not JSON: {line}"));
        }
    }
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

    #[test]
    fn should_fetch_the_lockfile_before_generating_offline_when_notices_are_made() {
        // given
        let fetch = fetch_args();

        // when
        let generate = generate_args("x86_64-unknown-linux-musl");

        // then
        assert_eq!(fetch, ["fetch", "--locked"]);
        assert!(generate.contains(&"--offline") && generate.contains(&"--locked"));
        assert_eq!(&generate[..2], ["about", "generate"]);
    }

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

    const LINUX: &str = "x86_64-unknown-linux-musl";
    const MAC: &str = "aarch64-apple-darwin";

    /// Notices as `cargo about` and [`fixed_sections`] would write them for `target`.
    fn notices_for(entries: &[(&str, &str)], target: &str) -> String {
        let mut text = String::from("Mozilla Public License Version 2.0\n");
        for (name, version) in entries {
            text.push_str(&format!(
                "* {name} {version} https://crates.io/crates/{name}/{version}\n"
            ));
        }
        text.push_str(&fixed_sections(target));
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
        let notices = notices_for(
            &[
                ("clap", "4.6.7"),
                ("serde_derive", "1.0.229"),
                ("selectors", "0.36.1"),
            ],
            LINUX,
        );

        // when
        let problems = notices_problems(&notices, &notices, &shipped, LINUX);

        // then
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn should_fail_when_a_crate_entry_is_deleted_from_the_archived_notices() {
        // given
        let shipped = parse_shipped(TREE);
        let fresh = notices_for(
            &[
                ("clap", "4.6.7"),
                ("serde_derive", "1.0.229"),
                ("selectors", "0.36.1"),
            ],
            MAC,
        );
        let archived: String = fresh
            .lines()
            .filter(|line| !line.starts_with("* clap "))
            .map(|line| format!("{line}\n"))
            .collect();

        // when
        let problems = notices_problems(&archived, &fresh, &shipped, MAC);

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
        let notices = notices_for(&[("clap", "4.6.7"), ("selectors", "0.36.1")], LINUX);

        // when
        let problems = notices_problems(&notices, &notices, &shipped, LINUX);

        // then
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("serde_derive 1.0.229"));
    }

    #[test]
    fn should_fail_when_an_mpl_crate_has_no_licence_text_or_source() {
        // given
        let shipped = set(&[("selectors", "0.36.1")]);
        let notices = format!(
            "* selectors 0.36.1 https://crates.io/crates/selectors/0.36\n{}",
            fixed_sections(MAC)
        );

        // when
        let problems = notices_problems(&notices, &notices, &shipped, MAC);

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
        let notices = notices_for(&[("clap", "4.6.7")], MAC)
            .replace("Mozilla Public License Version 2.0\n", "");

        // when
        let problems = notices_problems(&notices, &notices, &shipped, MAC);

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
    fn should_refuse_an_archive_named_for_another_target_when_smoking_an_existing_one() {
        // given
        let target = "x86_64-apple-darwin";

        // when
        let right = archive_name_problem(
            target,
            Path::new("dl/site-preview-x86_64-apple-darwin.tar.gz"),
        );
        let wrong = archive_name_problem(
            target,
            Path::new("dl/site-preview-aarch64-apple-darwin.tar.gz"),
        );
        let none = archive_name_problem(target, Path::new("/"));

        // then
        assert_eq!(right, None);
        assert!(wrong.is_some_and(|p| p.contains("not site-preview-aarch64-apple-darwin.tar.gz")));
        assert!(none.is_some());
    }

    #[test]
    fn should_refuse_an_existing_archive_when_it_holds_an_extra_file() {
        // given
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().join("stage/site-preview-x86_64-apple-darwin");
        fs::create_dir_all(&top).unwrap();
        for name in ARCHIVE_FILES.iter().chain(&["install.sh"]) {
            fs::write(top.join(name), b"x").unwrap();
        }
        let archive = dir.path().join("site-preview-x86_64-apple-darwin.tar.gz");
        let packed = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(dir.path().join("stage"))
            .arg("site-preview-x86_64-apple-darwin")
            .status()
            .unwrap();
        assert!(packed.success());
        let scratch = tempfile::tempdir().unwrap();

        // when
        let result = extract(&archive, scratch.path(), "x86_64-apple-darwin");

        // then
        let message = result.unwrap_err();
        assert!(message.contains("install.sh"), "{message}");
    }

    #[test]
    fn should_count_only_entries_with_their_crates_io_address_when_notices_are_read() {
        // given
        let notices = "\
* clap 4.6.7 https://crates.io/crates/clap/4.6.7
* serde 1.0.229 https://crates.io/crates/clap/4.6.7
* bullet point in some licence text
* tokio 1.0.0 https://crates.io/crates/tokio/1.0.0 extra
  * nested 1.0.0 https://crates.io/crates/nested/1.0.0
";

        // when
        let listed = listed_crates(notices);

        // then
        assert_eq!(listed, set(&[("clap", "4.6.7")]));
    }

    #[test]
    fn should_require_the_fixed_sections_when_the_target_needs_them() {
        // given
        let shipped = set(&[("clap", "4.6.7")]);
        let mac = notices_for(&[("clap", "4.6.7")], MAC);
        let linux = notices_for(&[("clap", "4.6.7")], LINUX);

        // when
        let mac_on_linux = notices_problems(&mac, &mac, &shipped, LINUX);
        let mac_ok = notices_problems(&mac, &mac, &shipped, MAC);
        let linux_ok = notices_problems(&linux, &linux, &shipped, LINUX);
        let without_std = linux.replace(RUST_STD_HEADING, "Rust");
        let no_std = notices_problems(&without_std, &without_std, &shipped, LINUX);

        // then
        assert!(
            mac_on_linux.iter().any(|p| p.contains(MUSL_HEADING)),
            "{mac_on_linux:?}"
        );
        assert!(
            mac_on_linux.iter().any(|p| p.contains("musl's copyright")),
            "{mac_on_linux:?}"
        );
        assert!(mac_ok.is_empty(), "{mac_ok:?}");
        assert!(linux_ok.is_empty(), "{linux_ok:?}");
        assert!(
            no_std.iter().any(|p| p.contains(RUST_STD_HEADING)),
            "{no_std:?}"
        );
    }

    #[test]
    fn should_ship_the_notice_texts_when_the_fixed_sections_are_read() {
        // given
        let (std_text, musl_text) = (RUST_STD_NOTICE, MUSL_NOTICE);

        // when
        let std_lines: Vec<&str> = std_text.lines().collect();
        let musl_lines: Vec<&str> = musl_text.lines().collect();

        // then
        assert!(std_lines.contains(&RUST_STD_HEADING));
        assert!(std_text.contains("Copyright (c) The Rust Project Contributors"));
        assert!(std_text.contains("Permission is hereby granted, free of charge"));
        assert!(musl_lines.contains(&MUSL_HEADING));
        assert!(musl_text.contains(MUSL_COPYRIGHT));
        assert!(
            musl_text.contains("The above copyright notice and this permission notice shall be")
        );
        assert!(!fixed_sections(MAC).contains(MUSL_HEADING));
        assert!(fixed_sections(LINUX).contains(MUSL_HEADING));
    }

    #[test]
    fn should_remap_cargo_home_and_the_checkout_when_release_flags_are_built() {
        // given
        let (root, home) = (Path::new("/w/site preview"), Path::new("/h/.cargo"));

        // when
        let flags = remap_flags(root, home);

        // then
        let args: Vec<&str> = flags.split('\u{1f}').collect();
        assert_eq!(
            args,
            [
                "--remap-path-prefix=/h/.cargo=/cargo",
                "--remap-path-prefix=/w/site preview=/build"
            ]
        );
    }

    #[test]
    fn should_report_a_build_path_when_the_binary_still_names_it() {
        // given
        let binary = b"\x7fELF...panicked at /h/.cargo/registry/src/x.rs...";

        // when
        let leaked = build_path_problems(binary, &[Path::new("/w/sp"), Path::new("/h/.cargo")]);

        // then
        assert_eq!(leaked, ["/h/.cargo"]);
        assert!(build_path_problems(b"/cargo/registry", &[Path::new("/h/.cargo")]).is_empty());
    }

    /// A fake repository root with the files [`package`] copies.
    fn fake_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for name in ["README.md", "LICENSE-MIT", "LICENSE-APACHE", "site-preview"] {
            fs::write(root.path().join(name), format!("{name}\n")).unwrap();
        }
        root
    }

    #[test]
    fn should_write_the_same_bytes_when_the_same_inputs_are_packaged_twice() {
        // given
        use std::os::unix::fs::PermissionsExt;
        let root = fake_root();
        let binary = root.path().join("site-preview");
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let first = package(root.path(), one.path(), MAC, &binary, "n\n", 1_700_000_000).unwrap();
        // A different builder: other times and modes on the inputs.
        for name in ["README.md", "site-preview"] {
            let path = root.path().join(name);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::File::open(&path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(5))
                .unwrap();
        }

        // when
        let second = package(root.path(), two.path(), MAC, &binary, "n\n", 1_700_000_000).unwrap();

        // then
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        let listing = Command::new("tar")
            .arg("-tvzf")
            .arg(&second)
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&listing.stdout);
        assert!(
            listing_problems(&listing, "site-preview-aarch64-apple-darwin").is_empty(),
            "{listing}"
        );
        let scratch = tempfile::tempdir().unwrap();
        let top = extract(&second, scratch.path(), MAC).unwrap();
        let mode = fs::metadata(top.join("site-preview"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn should_refuse_a_listing_when_it_holds_a_symlink_or_an_extra_entry() {
        // given
        let top = "site-preview-x86_64-apple-darwin";
        let mut good = format!("drwxr-xr-x 0/0 0 2026-10-13 10:00 {top}/\n");
        for name in ARCHIVE_FILES {
            good.push_str(&format!("-rw-r--r-- 0/0 5 2026-10-13 10:00 {top}/{name}\n"));
        }
        let bsd = good.replace("0/0", "0 0     ");
        let symlink = good.replace(
            &format!("-rw-r--r-- 0/0 5 2026-10-13 10:00 {top}/README.md"),
            &format!("lrwxrwxrwx 0/0 0 2026-10-13 10:00 {top}/README.md -> /etc/passwd"),
        );
        let extra = format!("{good}-rw-r--r-- 0/0 5 2026-10-13 10:00 other.txt\n");
        let twice = format!("{good}-rw-r--r-- 0/0 5 2026-10-13 10:00 {top}/README.md\n");

        // when
        let results = [&good, &bsd, &symlink, &extra, &twice].map(|l| listing_problems(l, top));

        // then
        assert!(results[0].is_empty(), "{:?}", results[0]);
        assert!(results[1].is_empty(), "{:?}", results[1]);
        assert!(
            results[2].iter().any(|p| p.contains("not a regular file")),
            "{:?}",
            results[2]
        );
        assert!(
            results[3]
                .iter()
                .any(|p| p.contains("unexpected entry other.txt")),
            "{:?}",
            results[3]
        );
        assert!(
            results[4].iter().any(|p| p.contains("listed twice")),
            "{:?}",
            results[4]
        );
    }

    #[test]
    fn should_skip_json_warnings_when_waiting_for_ready() {
        // given
        let (send, lines) = mpsc::channel();
        send.send(r#"{"event":"warning","code":"W1"}"#.to_owned())
            .unwrap();
        send.send(r#"{"event":"ready","url":"http://127.0.0.1:4000/"}"#.to_owned())
            .unwrap();
        let (send_bad, bad) = mpsc::channel();
        send_bad.send("Serving on port 4000".to_owned()).unwrap();
        drop(send_bad);

        // when
        let url = wait_for_ready(&lines, Duration::from_secs(5));
        let refused = wait_for_ready(&bad, Duration::from_secs(5));

        // then
        assert_eq!(url.as_deref(), Ok("http://127.0.0.1:4000/"));
        assert!(refused.is_err_and(|e| e.contains("not JSON")));
    }

    /// The release workflow, parsed (JSON-compatible YAML, so `on` stays a string key).
    fn release_workflow() -> (String, yaml_rust2::Yaml) {
        let text = fs::read_to_string(crate::repo::root().join(".github/workflows/release.yml"))
            .expect("release.yml");
        let doc = yaml_rust2::YamlLoader::load_from_str(&text)
            .expect("release.yml parses")
            .remove(0);
        (text, doc)
    }

    fn keys(map: &yaml_rust2::Yaml) -> Vec<String> {
        map.as_hash()
            .map(|h| {
                h.keys()
                    .filter_map(|k| k.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The scopes a `permissions` value grants `write` (a string such as `write-all` counts).
    fn write_scopes(permissions: &yaml_rust2::Yaml) -> Vec<String> {
        match permissions {
            yaml_rust2::Yaml::String(all) if all != "read-all" => vec![all.clone()],
            yaml_rust2::Yaml::Hash(h) => h
                .iter()
                .filter(|(_, v)| v.as_str() == Some("write"))
                .filter_map(|(k, _)| k.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn should_trigger_only_on_v_tags_and_dispatch_when_release_yml_is_read() {
        // given
        let (_, doc) = release_workflow();

        // when
        let on = &doc["on"];

        // then
        assert_eq!(
            keys(on),
            ["push", "workflow_dispatch"],
            "no pull_request or other trigger"
        );
        assert_eq!(
            keys(&on["push"]),
            ["tags"],
            "a tag push, never a branch push"
        );
        let tags: Vec<_> = on["push"]["tags"]
            .as_vec()
            .unwrap()
            .iter()
            .filter_map(|t| t.as_str())
            .collect();
        assert_eq!(tags, ["v*"]);
        let inputs = &on["workflow_dispatch"]["inputs"];
        assert_eq!(inputs["tag"]["required"].as_bool(), Some(true));
        assert_eq!(inputs["dry_run"]["default"].as_bool(), Some(true));
    }

    #[test]
    fn should_grant_write_permissions_only_to_publish_when_release_yml_is_read() {
        // given
        let (_, doc) = release_workflow();

        // when
        let top = write_scopes(&doc["permissions"]);
        let jobs = doc["jobs"].as_hash().expect("jobs");
        let mut writers = BTreeMap::new();
        let mut undeclared = Vec::new();
        for (id, job) in jobs {
            let id = id.as_str().unwrap().to_owned();
            if job["permissions"].is_badvalue() {
                undeclared.push(id.clone());
            }
            let scopes = write_scopes(&job["permissions"]);
            if !scopes.is_empty() {
                writers.insert(id, scopes);
            }
        }

        // then
        assert!(top.is_empty(), "workflow-level write: {top:?}");
        assert!(
            undeclared.is_empty(),
            "jobs without permissions: {undeclared:?}"
        );
        let want: BTreeMap<String, Vec<String>> = BTreeMap::from([(
            "publish".to_owned(),
            vec!["contents".into(), "id-token".into(), "attestations".into()],
        )]);
        assert_eq!(writers, want);
    }

    #[test]
    fn should_restore_no_cache_and_use_no_secret_when_release_yml_is_read() {
        // given
        let (text, _) = release_workflow();

        // when
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .map(|l| format!("{l}\n"))
            .collect();

        // then
        for banned in [
            "actions/cache",
            "setup-rust",
            "secrets.",
            "cargo publish",
            "pull_request",
            "git push",
            "git tag",
            "--draft=false",
        ] {
            assert!(!code.contains(banned), "release.yml uses {banned}");
        }
        let create = code
            .lines()
            .find(|l| l.contains("args=(--draft"))
            .expect("the gh release create arguments");
        assert!(create.contains("--verify-tag"), "{create}");
        assert_eq!(code.matches("gh release create").count(), 1);
    }

    #[test]
    fn should_run_the_four_targets_on_native_runners_when_release_yml_is_read() {
        // given
        let (_, doc) = release_workflow();

        // when
        let pairs = |job: &str| -> BTreeSet<(String, String)> {
            doc["jobs"][job]["strategy"]["matrix"]["include"]
                .as_vec()
                .expect("a matrix")
                .iter()
                .map(|leg| {
                    (
                        leg["target"].as_str().unwrap().to_owned(),
                        leg["os"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };

        // then
        let want: BTreeSet<(String, String)> = [
            ("aarch64-apple-darwin", "macos-15"),
            ("x86_64-apple-darwin", "macos-15-intel"),
            ("aarch64-unknown-linux-musl", "ubuntu-24.04-arm"),
            ("x86_64-unknown-linux-musl", "ubuntu-24.04"),
        ]
        .iter()
        .map(|(t, o)| ((*t).to_owned(), (*o).to_owned()))
        .collect();
        assert_eq!(pairs("build"), want);
        assert_eq!(pairs("verify"), want);
        assert!(
            want.iter()
                .all(|(t, _)| RELEASE_TARGETS.contains(&t.as_str()))
        );
    }

    /// Every `run` script of `job`.
    fn run_scripts(doc: &yaml_rust2::Yaml, job: &str) -> Vec<String> {
        doc["jobs"][job]["steps"]
            .as_vec()
            .expect("steps")
            .iter()
            .filter_map(|step| step["run"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn should_build_nothing_in_publish_when_release_yml_is_read() {
        // given
        let (_, doc) = release_workflow();

        // when
        let steps = doc["jobs"]["publish"]["steps"].as_vec().expect("steps");
        let uses: Vec<&str> = steps.iter().filter_map(|s| s["uses"].as_str()).collect();
        let scripts = run_scripts(&doc, "publish");

        // then
        for script in &scripts {
            let words: BTreeSet<&str> = script
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .collect();
            for banned in ["cargo", "rustup", "install-tools", "npm", "pip", "make"] {
                assert!(!words.contains(banned), "publish runs {banned}: {script}");
            }
        }
        for action in &uses {
            let name = action.split('@').next().unwrap_or("");
            assert!(
                [
                    "actions/download-artifact",
                    "actions/upload-artifact",
                    "actions/attest-build-provenance"
                ]
                .contains(&name),
                "publish uses {action}"
            );
        }
        let sbom = &doc["jobs"]["sbom"];
        assert!(write_scopes(&sbom["permissions"]).is_empty());
        assert!(
            sbom["needs"].as_str() == Some("build")
                || sbom["needs"]
                    .as_vec()
                    .is_some_and(|n| n.iter().any(|x| x.as_str() == Some("build")))
        );
        assert!(
            run_scripts(&doc, "sbom")
                .iter()
                .any(|s| s.contains("cargo xtask sbom"))
        );
        let needs: Vec<&str> = doc["jobs"]["publish"]["needs"]
            .as_vec()
            .expect("needs")
            .iter()
            .filter_map(|n| n.as_str())
            .collect();
        assert!(
            needs.contains(&"sbom") && needs.contains(&"build"),
            "{needs:?}"
        );
    }

    /// Each `(path, value)` of a scalar string below `node`, the path the mapping keys.
    fn strings(
        node: &yaml_rust2::Yaml,
        path: &mut Vec<String>,
        out: &mut Vec<(Vec<String>, String)>,
    ) {
        match node {
            yaml_rust2::Yaml::String(value) => out.push((path.clone(), value.clone())),
            yaml_rust2::Yaml::Array(items) => {
                for item in items {
                    strings(item, path, out);
                }
            }
            yaml_rust2::Yaml::Hash(map) => {
                for (key, value) in map {
                    path.push(key.as_str().unwrap_or("?").to_owned());
                    strings(value, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    #[test]
    fn should_read_inputs_and_the_ref_name_only_in_env_values_when_release_yml_is_read() {
        // given
        let (text, doc) = release_workflow();
        let mut found = Vec::new();
        strings(&doc, &mut Vec::new(), &mut found);

        // when
        let tainted: Vec<_> = found
            .iter()
            .filter(|(_, v)| v.contains("${{ inputs.") || v.contains("${{ github.ref_name"))
            .collect();

        // then
        assert!(!tainted.is_empty(), "the test sees the env values");
        for (path, value) in &tainted {
            let in_env = path.len() >= 2 && path[path.len() - 2] == "env";
            let whole = value.trim().starts_with("${{") && value.trim().ends_with("}}");
            assert!(in_env && whole, "{} = {value}", path.join("."));
        }
        let raw = text.matches("${{ inputs.").count() + text.matches("${{ github.ref_name").count();
        assert_eq!(raw, tainted.len(), "an expression outside a string value");
    }

    #[test]
    fn should_pin_the_signer_and_the_tag_when_attestations_are_verified() {
        // given
        let (text, doc) = release_workflow();

        // when
        let verify = run_scripts(&doc, "verify").join("\n");
        let publish = run_scripts(&doc, "publish").join("\n");
        let prepare = run_scripts(&doc, "prepare").join("\n");

        // then
        for script in [&verify, &publish] {
            assert!(
                script.contains(
                    "--signer-workflow \"$GITHUB_REPOSITORY/.github/workflows/release.yml\""
                ) || script.contains("--signer-workflow $GH_REPO/.github/workflows/release.yml"),
                "{script}"
            );
            assert!(script.contains("--source-ref"), "{script}");
            assert!(script.contains("refs/tags/$TAG"), "{script}");
        }
        assert!(text.contains("dist/SHA256SUMS"), "SHA256SUMS is attested");
        assert!(
            publish.contains("git/ref/tags/"),
            "publish re-reads the tag"
        );
        assert!(publish.contains("--prerelease"));
        assert!(prepare.contains("check-runs"), "prepare checks Required");
        assert_eq!(
            doc["jobs"]["prepare"]["permissions"]["checks"].as_str(),
            Some("read")
        );
        let ancestry = prepare
            .lines()
            .find(|l| l.contains("merge-base --is-ancestor"))
            .expect("the ancestry check");
        assert!(
            !ancestry.starts_with(' '),
            "ancestry is checked in both modes: {ancestry}"
        );
    }

    #[test]
    fn should_attach_and_verify_the_provenance_bundle_when_release_yml_is_read() {
        // given
        let (_, doc) = release_workflow();
        let steps = doc["jobs"]["publish"]["steps"].as_vec().expect("steps");

        // when
        let position = |found: &dyn Fn(&yaml_rust2::Yaml) -> bool| {
            steps.iter().position(found).expect("the step")
        };
        let attest = position(&|s| {
            s["uses"]
                .as_str()
                .is_some_and(|u| u.starts_with("actions/attest-build-provenance@"))
        });
        let attach = position(&|s| s["env"]["BUNDLE"].as_str().is_some());
        let draft = position(&|s| {
            s["run"]
                .as_str()
                .is_some_and(|r| r.contains("gh release create"))
        });
        let verify = run_scripts(&doc, "verify").join("\n");

        // then
        assert_eq!(steps[attest]["id"].as_str(), Some("attest"));
        assert_eq!(
            steps[attach]["env"]["BUNDLE"].as_str(),
            Some("${{ steps.attest.outputs.bundle-path }}")
        );
        assert!(
            steps[attach]["run"]
                .as_str()
                .is_some_and(|r| r.contains("> dist/site-preview.intoto.jsonl")),
            "the bundle is written into dist/, which the draft uploads"
        );
        assert!(
            attest < attach && attach < draft,
            "{attest} {attach} {draft}"
        );
        assert!(
            verify.contains("--bundle \"$RUNNER_TEMP/published/site-preview.intoto.jsonl\""),
            "{verify}"
        );
        assert!(
            verify.contains("SHA256SUMS|site-preview.intoto.jsonl)"),
            "SHA256SUMS cannot list the bundle that attests it: {verify}"
        );
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
