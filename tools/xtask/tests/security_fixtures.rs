//! Black-box check of the dependency policy (foundation.md section 8): a mini workspace that
//! depends on a git source must fail the repository's own `deny.toml`, and the same workspace
//! without that dependency must pass it, so the failure is the policy and not the harness.
//!
//! The git source is a local repository, because the policy denies every git source and
//! `cargo metadata` has to be able to resolve it offline.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask lives two levels below the repository root")
        .to_path_buf()
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("cannot run {program}: {err}"))
}

fn run_ok(dir: &Path, program: &str, args: &[&str]) {
    let out = run(dir, program, args);
    assert!(
        out.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A one-crate git repository named `foo`, returned as its `file://` URL.
fn git_dependency(parent: &Path) -> String {
    let dir = parent.join("foo");
    fs::create_dir_all(dir.join("src")).expect("create dependency");
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"foo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         license = \"MIT OR Apache-2.0\"\n",
    )
    .expect("write manifest");
    fs::write(dir.join("src/lib.rs"), "").expect("write source");
    run_ok(&dir, "git", &["init", "--quiet"]);
    run_ok(&dir, "git", &["add", "."]);
    run_ok(
        &dir,
        "git",
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    format!("file://{}", dir.display())
}

/// A mini workspace with the repository's `deny.toml`; `dependency` is a `[dependencies]` line.
fn mini_workspace(parent: &Path, dependency: &str) -> PathBuf {
    let dir = parent.join("mini");
    fs::create_dir_all(dir.join("src")).expect("create workspace");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"mini\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             license = \"MIT OR Apache-2.0\"\npublish = false\n\n[dependencies]\n{dependency}\n"
        ),
    )
    .expect("write manifest");
    fs::write(dir.join("src/lib.rs"), "").expect("write source");
    fs::copy(repository_root().join("deny.toml"), dir.join("deny.toml")).expect("copy policy");
    run_ok(&dir, "cargo", &["generate-lockfile"]);
    dir
}

fn deny_sources(dir: &Path) -> Output {
    run(dir, "cargo", &["deny", "--locked", "check", "sources"])
}

#[test]
fn should_fail_sources_check_when_a_crate_depends_on_a_git_source() {
    // given
    let temp = tempfile::tempdir().expect("tempdir");
    let url = git_dependency(temp.path());
    let dir = mini_workspace(temp.path(), &format!("foo = {{ git = \"{url}\" }}"));

    // when
    let out = deny_sources(&dir);

    // then
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "git source accepted: {text}");
    assert!(text.contains("source-not-allowed"), "{text}");
}

#[test]
fn should_pass_sources_check_when_a_crate_has_no_dependencies() {
    // given
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = mini_workspace(temp.path(), "");

    // when
    let out = deny_sources(&dir);

    // then
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
