//! Pinned development tools: the manifest in `tools/versions.toml`, `doctor`,
//! `install-tools` and `require` (ADR 0010: a missing tool fails closed, never passes).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::process::Command;

/// The manifest, embedded so `cargo xtask` works from any directory.
const MANIFEST: &str = include_str!("../../versions.toml");

/// How a tool is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// `cargo install --locked <name>@<version>`.
    Cargo,
    /// `rustup toolchain install <version> --profile minimal`.
    Rustup,
    /// A manual step; the manifest holds the URL.
    Manual(String),
    /// Version owned by a lockfile; noted in the manifest, not checked.
    Lockfile(String),
}

/// One pinned tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// Manifest key, for example `cargo-nextest`.
    pub name: String,
    /// Exact pinned version.
    pub version: String,
    /// Installation method.
    pub method: Method,
    /// Program and arguments that print the installed version.
    pub probe: Vec<String>,
    /// Match the version as a substring of the probe output instead of as a token.
    pub contains: bool,
}

/// Result of checking one tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Installed at the pinned version.
    Ok,
    /// Not installed (probe could not run or failed).
    Missing,
    /// Installed with another version; carries the probe's first output line.
    Wrong(String),
}

/// Parses the manifest text into tools, sorted by name.
pub fn parse(text: &str) -> Result<Vec<Tool>, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("versions.toml: {e}"))?;
    let tools = table
        .get("tools")
        .and_then(toml::Value::as_table)
        .ok_or("versions.toml: missing [tools]")?;
    let mut out = BTreeMap::new();
    for (name, value) in tools {
        let t = value
            .as_table()
            .ok_or_else(|| format!("versions.toml: tools.{name} is not a table"))?;
        let field = |key: &str| {
            t.get(key)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("versions.toml: tools.{name}.{key} missing"))
        };
        let method = match field("method")?.as_str() {
            "cargo" => Method::Cargo,
            "rustup" => Method::Rustup,
            "manual" => Method::Manual(field("url")?),
            "lockfile" => Method::Lockfile(field("lockfile")?),
            other => {
                return Err(format!(
                    "versions.toml: tools.{name}: unknown method {other}"
                ));
            }
        };
        let probe: Vec<String> = t
            .get("probe")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if probe.is_empty() && !matches!(method, Method::Lockfile(_)) {
            return Err(format!("versions.toml: tools.{name}.probe missing"));
        }
        out.insert(
            name.clone(),
            Tool {
                name: name.clone(),
                version: field("version")?,
                method,
                probe,
                contains: t
                    .get("contains")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
            },
        );
    }
    Ok(out.into_values().collect())
}

/// The embedded manifest, parsed.
pub fn manifest() -> Result<Vec<Tool>, String> {
    parse(MANIFEST)
}

/// Whether `output` reports exactly `version`.
fn version_matches(output: &str, version: &str, contains: bool) -> bool {
    if contains {
        return output.contains(version);
    }
    output
        .split_whitespace()
        .any(|tok| tok.trim_start_matches('v') == version)
}

/// Probes a tool, optionally with a replacement `PATH` for the probe process.
pub fn check_with_path(tool: &Tool, path: Option<&OsStr>) -> Status {
    let Some((program, args)) = tool.probe.split_first() else {
        return Status::Ok;
    };
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    let Ok(output) = cmd.output() else {
        return Status::Missing;
    };
    if !output.status.success() {
        return Status::Missing;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if version_matches(&text, &tool.version, tool.contains) {
        Status::Ok
    } else {
        let first = text.lines().next().unwrap_or("").trim().to_owned();
        Status::Wrong(first)
    }
}

/// The actionable hint for a tool that is missing or at the wrong version.
pub fn install_hint(tool: &Tool) -> String {
    match &tool.method {
        Method::Cargo | Method::Rustup => format!("run 'cargo xtask install-tools {}'", tool.name),
        Method::Manual(url) => format!("install manually from {url}"),
        Method::Lockfile(file) => format!("run 'npm ci' for the lockfile {file}"),
    }
}

/// The failure line for a tool that is not usable.
fn describe(tool: &Tool, status: &Status) -> String {
    match status {
        Status::Ok => format!("ok {} {}", tool.name, tool.version),
        Status::Missing => format!(
            "missing {} {}: {}",
            tool.name,
            tool.version,
            install_hint(tool)
        ),
        Status::Wrong(found) => format!(
            "wrong version of {} (found '{found}', want {}): {}",
            tool.name,
            tool.version,
            install_hint(tool)
        ),
    }
}

fn require_in(tools: &[Tool], name: &str, path: Option<&OsStr>) -> Result<(), String> {
    let tool = tools
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| format!("unknown tool {name}: add it to tools/versions.toml"))?;
    match check_with_path(tool, path) {
        Status::Ok => Ok(()),
        status => Err(describe(tool, &status)),
    }
}

/// Fails with an install hint unless `name` is installed at its pinned version. Every stage
/// that invokes an external tool calls this first.
pub fn require(name: &str) -> Result<(), String> {
    require_in(&manifest()?, name, None)
}

/// `cargo xtask doctor`: checks every pinned tool, prints one line each, and exits non-zero
/// when any is missing or at the wrong version.
pub fn doctor() -> i32 {
    let tools = match manifest() {
        Ok(tools) => tools,
        Err(err) => {
            eprintln!("xtask: {err}");
            return 1;
        }
    };
    let mut bad = 0;
    for tool in &tools {
        if matches!(tool.method, Method::Lockfile(_)) {
            println!(
                "note {} {}: version owned by {}",
                tool.name,
                tool.version,
                match &tool.method {
                    Method::Lockfile(f) => f.as_str(),
                    _ => "",
                }
            );
            continue;
        }
        let status = check_with_path(tool, None);
        if status != Status::Ok {
            bad += 1;
        }
        println!("{}", describe(tool, &status));
    }
    if bad > 0 {
        eprintln!("xtask: doctor: {bad} tool(s) missing or at the wrong version");
        1
    } else {
        0
    }
}

/// `cargo xtask install-tools [tool...]`: installs the named tools, or all installable
/// ones. Binary tools print their manual step. Returns the exit code.
pub fn install(names: &[String]) -> i32 {
    let tools = match manifest() {
        Ok(tools) => tools,
        Err(err) => {
            eprintln!("xtask: {err}");
            return 1;
        }
    };
    let selected: Vec<&Tool> = if names.is_empty() {
        tools.iter().collect()
    } else {
        let mut picked = Vec::new();
        for name in names {
            match tools.iter().find(|t| &t.name == name) {
                Some(tool) => picked.push(tool),
                None => {
                    eprintln!("xtask: unknown tool {name}: see tools/versions.toml");
                    return 2;
                }
            }
        }
        picked
    };
    let mut failed = false;
    for tool in selected {
        let ran = match &tool.method {
            Method::Cargo => run(
                "cargo",
                &[
                    "install",
                    "--locked",
                    &format!("{}@{}", tool.name, tool.version),
                ],
            ),
            Method::Rustup => run(
                "rustup",
                &[
                    "toolchain",
                    "install",
                    &tool.version,
                    "--profile",
                    "minimal",
                ],
            ),
            Method::Manual(url) => {
                println!(
                    "manual step for {} {}: install from {url}",
                    tool.name, tool.version
                );
                continue;
            }
            Method::Lockfile(file) => {
                println!("{}: installed from {file} by 'npm ci'", tool.name);
                continue;
            }
        };
        if !ran {
            eprintln!("xtask: installing {} failed", tool.name);
            failed = true;
        } else if let Err(err) = require(&tool.name) {
            eprintln!("xtask: installed {} but it is not usable: {err}", tool.name);
            failed = true;
        } else {
            println!("ok {} {}", tool.name, tool.version);
        }
    }
    i32::from(failed)
}

fn run(program: &str, args: &[&str]) -> bool {
    eprintln!("xtask: {program} {}", args.join(" "));
    Command::new(program)
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nextest() -> Tool {
        manifest()
            .unwrap()
            .into_iter()
            .find(|t| t.name == "cargo-nextest")
            .unwrap()
    }

    #[test]
    fn should_fail_with_install_hint_when_tool_missing() {
        // given
        let tools = manifest().unwrap();
        let empty = tempfile::TempDir::new().unwrap();

        // when
        let result = require_in(&tools, "cargo-nextest", Some(empty.path().as_os_str()));

        // then
        let err = result.unwrap_err();
        assert!(
            err.starts_with("missing cargo-nextest ")
                && err.contains("run 'cargo xtask install-tools cargo-nextest'"),
            "{err}"
        );
    }

    #[test]
    fn should_parse_every_tool_when_manifest_is_loaded() {
        // given / when
        let tools = manifest().unwrap();

        // then
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        for want in [
            "cargo-nextest",
            "cargo-deny",
            "cargo-llvm-cov",
            "cargo-mutants",
            "cargo-fuzz",
            "gitleaks",
            "actionlint",
            "cargo-cyclonedx",
            "quint",
            "@playwright/test",
            "nightly",
            "apalache",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
    }

    #[test]
    fn should_use_exact_versions_when_manifest_is_loaded() {
        // given
        let tools = manifest().unwrap();

        // when
        let loose: Vec<_> = tools
            .iter()
            .filter(|t| {
                !t.version.chars().any(|c| c.is_ascii_digit())
                    || t.version.contains(['^', '~', '*', '>', '<'])
            })
            .collect();

        // then
        assert!(loose.is_empty(), "{loose:?}");
    }

    #[test]
    fn should_match_token_exactly_when_version_is_compared() {
        // given
        let output = "cargo-nextest 0.9.148 (abc 2026-10-01)";

        // when / then
        assert!(version_matches(output, "0.9.148", false));
        assert!(!version_matches(output, "0.9.14", false));
        assert!(version_matches("v8.30.1", "8.30.1", false));
    }

    #[test]
    fn should_report_wrong_version_when_probe_prints_another_version() {
        // given
        let mut tool = nextest();
        tool.probe = vec!["echo".into(), "cargo-nextest 0.0.1".into()];

        // when
        let status = check_with_path(&tool, None);

        // then
        assert_eq!(status, Status::Wrong("cargo-nextest 0.0.1".into()));
    }

    #[test]
    fn should_reject_unknown_method_when_manifest_is_parsed() {
        // given
        let text = "[tools.x]\nversion = \"1\"\nmethod = \"magic\"\nprobe = [\"x\"]\n";

        // when
        let err = parse(text).unwrap_err();

        // then
        assert!(err.contains("unknown method"), "{err}");
    }
}
