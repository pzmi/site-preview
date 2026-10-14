//! `cargo xtask sbom [--out DIR]`: one CycloneDX SBOM per release archive (OPS-06, D23,
//! ADR 0078).
//!
//! For each of the four release targets it runs the pinned `cargo cyclonedx` over
//! `crates/preview-cli` with that target, without build-only dependencies, and writes
//! `DIR/site-preview-<target>.cdx.json` (default `DIR`: `target/sbom`). `cargo cyclonedx` writes
//! one file next to every workspace manifest; the `preview-cli` one is moved out and the others
//! are deleted, so the tree is left as it was. `SOURCE_DATE_EPOCH` is the commit time of `HEAD`
//! (or the value already set), so the same commit gives the same timestamp.
//!
//! Each SBOM is then read back and must describe `preview-cli` at the workspace version and list
//! every crate of `cargo tree -e normal --target <target>` (the graph `release-check` ships and
//! names in `THIRD-PARTY-NOTICES`): an SBOM that misses a shipped crate fails the command.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use yaml_rust2::{Yaml, YamlLoader};

use crate::release::{RELEASE_TARGETS, shipped_crates};

/// The CycloneDX specification version written (the newest `cargo-cyclonedx` 0.5 offers).
const SPEC_VERSION: &str = "1.5";

/// Runs the command and returns its exit code.
pub fn run(out: Option<&Path>) -> i32 {
    match generate_all(out) {
        Ok(written) => {
            for path in written {
                eprintln!("sbom: wrote {}", path.display());
            }
            0
        }
        Err(err) => {
            eprintln!("xtask: sbom: {err}");
            1
        }
    }
}

fn generate_all(out: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let root = crate::repo::root();
    crate::tools::require("cargo-cyclonedx")?;
    let out = out.map_or_else(|| root.join("target/sbom"), Path::to_path_buf);
    fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let epoch = source_date_epoch(&root)?.to_string();
    let members = member_dirs(&root)?;
    let mut written = Vec::new();
    for target in RELEASE_TARGETS {
        let stem = sbom_stem(target);
        let result = generate(&root, target, &stem, &epoch);
        let produced = root.join("crates/preview-cli").join(format!("{stem}.json"));
        let dest = out.join(format!("{stem}.json"));
        let moved = result.and_then(|()| {
            fs::rename(&produced, &dest).map_err(|e| {
                format!(
                    "cannot move {} to {}: {e}",
                    produced.display(),
                    dest.display()
                )
            })
        });
        // Every workspace member got a file of the same name; none of them is kept.
        for dir in &members {
            let _ = fs::remove_file(dir.join(format!("{stem}.json")));
        }
        moved?;
        let raw = fs::read_to_string(&dest)
            .map_err(|e| format!("cannot read {}: {e}", dest.display()))?;
        let root_text = root.display().to_string();
        let text = strip_checkout(&raw, &root_text);
        if let Some(problem) = checkout_problem(&text, &root_text) {
            return Err(format!("{}: {problem}", dest.display()));
        }
        fs::write(&dest, &text).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
        let shipped = shipped_crates(&root, target)?;
        let problems = sbom_problems(&text, env!("CARGO_PKG_VERSION"), &shipped);
        if !problems.is_empty() {
            return Err(format!("{}: {}", dest.display(), problems.join("; ")));
        }
        written.push(dest);
    }
    Ok(written)
}

/// `cargo cyclonedx` names each workspace crate's `bom-ref` by its absolute manifest
/// directory (`path+file:///home/runner/work/...`). The checkout directory is cut out of every
/// file URL, so the SBOM names no path of the machine that made it and is the same on any
/// machine (`should_drop_the_checkout_directory_when_bom_refs_are_absolute`).
fn strip_checkout(text: &str, root: &str) -> String {
    text.replace(&format!("file://{root}/"), "file:///")
}

/// Why `text` still names the checkout `root` after [`strip_checkout`], if it does: a
/// `cargo cyclonedx` that writes the path in a form other than a `file://` URL would otherwise
/// leak it unnoticed (`should_fail_when_the_checkout_path_survives_the_strip`).
fn checkout_problem(text: &str, root: &str) -> Option<String> {
    (!root.is_empty() && text.contains(root))
        .then(|| format!("still names the checkout directory {root}"))
}

/// The output file stem for `target`: `cargo cyclonedx` appends `.json`.
fn sbom_stem(target: &str) -> String {
    format!("site-preview-{target}.cdx")
}

/// The arguments of `cargo cyclonedx` for `target`.
fn cyclonedx_args<'a>(target: &'a str, stem: &'a str) -> Vec<&'a str> {
    vec![
        "cyclonedx",
        "--manifest-path",
        "crates/preview-cli/Cargo.toml",
        "--format",
        "json",
        "--spec-version",
        SPEC_VERSION,
        "--target",
        target,
        "--no-build-deps",
        "--override-filename",
        stem,
    ]
}

fn generate(root: &Path, target: &str, stem: &str, epoch: &str) -> Result<(), String> {
    let output = Command::new("cargo")
        .args(cyclonedx_args(target, stem))
        .env("SOURCE_DATE_EPOCH", epoch)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run cargo cyclonedx: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo cyclonedx --target {target} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The time a release artifact records: `SOURCE_DATE_EPOCH` when it is set (all digits), else
/// the committer time of `HEAD`. `release-check` stamps the archive entries with it and the
/// SBOMs carry it, so a rebuild of the same commit gives the same bytes.
pub fn source_date_epoch(root: &Path) -> Result<u64, String> {
    if let Some(set) = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return set
            .parse()
            .map_err(|_| format!("SOURCE_DATE_EPOCH '{set}' is not a number of seconds"));
    }
    commit_epoch(root)?
        .parse()
        .map_err(|e| format!("bad commit time: {e}"))
}

/// The committer time of `HEAD` in seconds, for `SOURCE_DATE_EPOCH`.
fn commit_epoch(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ct", "HEAD"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    let epoch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && !epoch.is_empty() && epoch.bytes().all(|b| b.is_ascii_digit()) {
        Ok(epoch)
    } else {
        Err("git log printed no commit time for HEAD".to_owned())
    }
}

/// The directory of every workspace member, where `cargo cyclonedx` writes a file.
fn member_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(root)
        .no_deps()
        .exec()
        .map_err(|e| format!("cargo metadata failed: {e}"))?;
    Ok(metadata
        .workspace_packages()
        .iter()
        .filter_map(|p| {
            p.manifest_path
                .parent()
                .map(|d| d.as_std_path().to_path_buf())
        })
        .collect())
}

/// Everything wrong with an SBOM's text, empty when it is sound. JSON is read with the YAML
/// parser (JSON is YAML 1.2), so no JSON crate is added to `xtask`.
fn sbom_problems(text: &str, version: &str, shipped: &BTreeSet<(String, String)>) -> Vec<String> {
    let docs = match YamlLoader::load_from_str(text) {
        Ok(docs) => docs,
        Err(e) => return vec![format!("not JSON: {e}")],
    };
    let Some(doc) = docs.first() else {
        return vec!["empty".to_owned()];
    };
    let mut problems = Vec::new();
    if doc["bomFormat"].as_str() != Some("CycloneDX") {
        problems.push("bomFormat is not CycloneDX".to_owned());
    }
    if doc["specVersion"].as_str() != Some(SPEC_VERSION) {
        problems.push(format!("specVersion is not {SPEC_VERSION}"));
    }
    let component = &doc["metadata"]["component"];
    if component["name"].as_str() != Some("preview-cli")
        || component["version"].as_str() != Some(version)
    {
        problems.push(format!(
            "the described component is not preview-cli {version}"
        ));
    }
    let listed: BTreeSet<(String, String)> = match &doc["components"] {
        Yaml::Array(items) => items
            .iter()
            .filter_map(|c| {
                Some((
                    c["name"].as_str()?.to_owned(),
                    c["version"].as_str()?.to_owned(),
                ))
            })
            .collect(),
        _ => {
            problems.push("no components array".to_owned());
            BTreeSet::new()
        }
    };
    for (name, version) in shipped.difference(&listed) {
        problems.push(format!(
            "no component for the shipped crate {name} {version}"
        ));
    }
    problems
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

    const SBOM: &str = r#"{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "metadata": { "component": { "name": "preview-cli", "version": "0.1.0" } },
  "components": [
    { "type": "library", "name": "clap", "version": "4.6.7" },
    { "type": "library", "name": "selectors", "version": "0.36.1" }
  ]
}"#;

    #[test]
    fn should_accept_an_sbom_when_it_lists_every_shipped_crate() {
        // given
        let shipped = set(&[("clap", "4.6.7"), ("selectors", "0.36.1")]);

        // when
        let problems = sbom_problems(SBOM, "0.1.0", &shipped);

        // then
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn should_fail_when_a_shipped_crate_is_missing_from_the_sbom() {
        // given
        let shipped = set(&[("clap", "4.6.7"), ("serde", "1.0.229")]);

        // when
        let problems = sbom_problems(SBOM, "0.1.0", &shipped);

        // then
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("serde 1.0.229"), "{problems:?}");
    }

    #[test]
    fn should_fail_when_the_sbom_describes_another_component_or_format() {
        // given
        let other = SBOM
            .replace("\"preview-cli\"", "\"xtask\"")
            .replace("\"CycloneDX\"", "\"SPDX\"");

        // when
        let problems = sbom_problems(&other, "0.1.0", &BTreeSet::new());
        let garbage = sbom_problems("{", "0.1.0", &BTreeSet::new());

        // then
        assert!(
            problems.iter().any(|p| p.contains("bomFormat")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("preview-cli 0.1.0")),
            "{problems:?}"
        );
        assert!(garbage[0].starts_with("not JSON"), "{garbage:?}");
    }

    #[test]
    fn should_drop_the_checkout_directory_when_bom_refs_are_absolute() {
        // given
        let text = r#""bom-ref": "path+file:///home/runner/work/sp/sp/crates/preview-cli#0.1.0""#;

        // when
        let stripped = strip_checkout(text, "/home/runner/work/sp/sp");

        // then
        assert_eq!(
            stripped,
            r#""bom-ref": "path+file:///crates/preview-cli#0.1.0""#
        );
    }

    #[test]
    fn should_fail_when_the_checkout_path_survives_the_strip() {
        // given
        let root = "/home/runner/work/sp/sp";
        let leaked = r#""bom-ref": "path+file:///home/runner/work/sp/sp/crates/preview-cli#0.1.0", "x": "/home/runner/work/sp/sp/Cargo.toml""#;

        // when
        let stripped = strip_checkout(leaked, root);

        // then
        assert!(checkout_problem(&stripped, root).is_some_and(|p| p.contains(root)));
        assert_eq!(
            checkout_problem(&strip_checkout(leaked, "/nowhere"), "/nowhere"),
            None
        );
    }

    #[test]
    fn should_describe_the_cli_for_one_target_without_build_deps_when_args_are_built() {
        // given
        let target = "aarch64-apple-darwin";
        let stem = sbom_stem(target);

        // when
        let args = cyclonedx_args(target, &stem);

        // then
        assert_eq!(stem, "site-preview-aarch64-apple-darwin.cdx");
        let pairs: Vec<_> = args.windows(2).map(|w| (w[0], w[1])).collect();
        assert!(pairs.contains(&("--manifest-path", "crates/preview-cli/Cargo.toml")));
        assert!(pairs.contains(&("--target", target)));
        assert!(pairs.contains(&("--format", "json")));
        assert!(args.contains(&"--no-build-deps"));
    }

    #[test]
    fn should_run_cargo_cyclonedx_from_cargo_when_the_tool_is_pinned() {
        // given
        let tools = crate::tools::manifest().unwrap();

        // when
        let pinned = tools.iter().find(|t| t.name == "cargo-cyclonedx");

        // then
        let pinned = pinned.expect("cargo-cyclonedx is pinned in tools/versions.toml");
        assert_eq!(pinned.method, crate::tools::Method::Cargo);
        assert_eq!(pinned.probe, ["cargo-cyclonedx", "cyclonedx", "--version"]);
    }
}
