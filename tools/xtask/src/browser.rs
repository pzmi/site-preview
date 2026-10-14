//! The `browser` stage (E2E-04, D15): Playwright specs in `tests/e2e/browser/` against the real
//! binary. Node is a development tool only; the shipped binary never needs it (D8).
//!
//! The stage builds the binary, installs the locked npm packages (`npm ci`, scripts disabled
//! by the project's `.npmrc`) and runs `npx playwright test`. It never downloads a browser: a
//! missing Node or Chromium fails with an actionable message (D9), never a skip.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::process::Stage;
use crate::repo;

/// The Chromium build number the pinned `@playwright/test` expects (`tools/versions.toml`).
pub(crate) const CHROMIUM_BUILD: &str = "1194";

/// The browser stage. It needs Node and the Chromium build, so a missing or wrong one shows as
/// `UNAVAILABLE` and `doctor` lists both (ADR 0080).
pub fn stage() -> Stage {
    Stage::in_process(
        "browser",
        "cargo build --locked -p preview-cli, npm ci --prefix tests/e2e/browser, npx playwright test",
        run,
    )
    .needs("node")
    .and_needs(crate::tools::CHROMIUM)
}

/// The directory that holds Playwright's browsers: `PLAYWRIGHT_BROWSERS_PATH`, else the
/// per-user cache Playwright installs into.
fn browsers_dir(env_path: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match env_path {
        Some(path) if !path.is_empty() && path != "0" => Some(PathBuf::from(path)),
        _ => home.map(|home| Path::new(home).join(".cache/ms-playwright")),
    }
}

/// `Ok` when `dir` holds the headless Chromium build the pinned Playwright launches.
fn require_chromium(dir: Option<&Path>) -> Result<(), String> {
    let wanted = format!("chromium_headless_shell-{CHROMIUM_BUILD}");
    match dir {
        Some(dir) if dir.join(&wanted).is_dir() => Ok(()),
        Some(dir) => Err(format!(
            "missing Chromium build {CHROMIUM_BUILD} ({wanted}) in {}: point PLAYWRIGHT_BROWSERS_PATH at a \
             directory that has it (the agent sandbox has /opt/pw-browsers); on a developer machine or in CI \
             run 'npx --prefix tests/e2e/browser playwright install chromium' once. Never run it in the \
             agent sandbox",
            dir.display()
        )),
        None => Err(format!(
            "missing Chromium build {CHROMIUM_BUILD}: set PLAYWRIGHT_BROWSERS_PATH to a directory that has {wanted}"
        )),
    }
}

fn run_command(command: &mut Command, label: &str) -> i32 {
    eprintln!("xtask: stage browser: {label}");
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("xtask: cannot run {label}: {err}");
            1
        }
    }
}

/// [`require_chromium`] in the directory this environment points Playwright at.
pub(crate) fn require_installed_chromium() -> Result<(), String> {
    let env_path = std::env::var("PLAYWRIGHT_BROWSERS_PATH").ok();
    let home = std::env::var("HOME").ok();
    require_chromium(browsers_dir(env_path.as_deref(), home.as_deref()).as_deref())
}

fn run() -> i32 {
    if let Err(err) = require_installed_chromium() {
        eprintln!("xtask: {err}");
        return 1;
    }
    let root = repo::root();
    let project = root.join("tests/e2e/browser");
    let steps: [(&str, Command); 3] = [
        ("cargo build --locked -p preview-cli --bin site-preview", {
            let mut c = Command::new("cargo");
            c.args([
                "build",
                "--locked",
                "-p",
                "preview-cli",
                "--bin",
                "site-preview",
            ])
            .current_dir(&root);
            c
        }),
        ("npm ci --prefix tests/e2e/browser", {
            let mut c = Command::new("npm");
            c.args(["ci", "--prefix"]).arg(&project).current_dir(&root);
            c
        }),
        ("npx playwright test", {
            let mut c = Command::new("npx");
            c.args(["playwright", "test"]).current_dir(&project);
            c
        }),
    ];
    for (label, mut command) in steps {
        let code = run_command(&mut command, label);
        if code != 0 {
            return code;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_use_the_environment_path_when_playwright_browsers_path_is_set() {
        // given
        let env_path = Some("/opt/pw-browsers");

        // when
        let dir = browsers_dir(env_path, Some("/home/dev"));

        // then
        assert_eq!(dir, Some(PathBuf::from("/opt/pw-browsers")));
    }

    #[test]
    fn should_use_the_user_cache_when_playwright_browsers_path_is_unset() {
        // given
        let env_path = None;

        // when
        let dir = browsers_dir(env_path, Some("/home/dev"));

        // then
        assert_eq!(dir, Some(PathBuf::from("/home/dev/.cache/ms-playwright")));
    }

    #[test]
    fn should_fail_with_an_install_hint_when_chromium_is_missing() {
        // given
        let empty = tempfile::tempdir().expect("tempdir");

        // when
        let result = require_chromium(Some(empty.path()));

        // then
        let message = result.expect_err("no browser in an empty directory");
        assert!(message.contains("missing Chromium build 1194"), "{message}");
        assert!(message.contains("PLAYWRIGHT_BROWSERS_PATH"), "{message}");
    }

    #[test]
    fn should_accept_the_directory_when_the_headless_shell_build_is_present() {
        // given
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("chromium_headless_shell-1194")).expect("mkdir");

        // when
        let result = require_chromium(Some(dir.path()));

        // then
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn should_require_node_and_chromium_when_the_stage_is_built() {
        // given
        let stage = stage();

        // when
        let tools = (stage.tool, stage.also_tool);

        // then
        assert_eq!(tools, (Some("node"), Some("chromium")));
        assert_eq!(stage.name, "browser");
    }
}
