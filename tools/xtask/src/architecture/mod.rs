//! `cargo xtask architecture`: the versioned dependency policy (foundation.md section 3.2)
//! checked against `cargo metadata` and the source tree, plus the negative fixtures of
//! section 3.4.
//!
//! Without `--manifest-path` it checks this repository and then confirms that every fixture
//! under [`FIXTURES_DIR`] fails with the rule its `expected.txt` names. With it, it checks
//! only that workspace, whose root must hold `architecture.toml` and a `Cargo.lock`.

mod fixtures;
mod graph;
mod inventory;
mod policy;
mod report;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, MetadataCommand};

use inventory::Crate;
use report::{Diagnostic, Rule};

/// The policy file at the workspace root.
pub const POLICY_FILE: &str = "architecture.toml";

/// Negative fixtures, relative to the repository root: one mini workspace per directory.
pub const FIXTURES_DIR: &str = "tests/architecture-invalid";

/// The result of checking one workspace.
#[derive(Debug)]
pub struct Outcome {
    /// Violations, sorted.
    pub diagnostics: Vec<Diagnostic>,
    /// Number of workspace crates checked.
    pub crates: usize,
}

/// Runs the command and returns the exit code: 0 clean, 1 on any violation.
pub fn run(manifest_path: Option<&Path>) -> i32 {
    let (manifest, with_fixtures) = match manifest_path {
        Some(path) => (path.to_path_buf(), false),
        None => (repository_manifest(), true),
    };
    let outcome = check(&manifest);
    for diagnostic in &outcome.diagnostics {
        eprintln!("{diagnostic}");
    }
    let mut failed = !outcome.diagnostics.is_empty();
    if failed {
        eprintln!(
            "xtask: architecture: {} violation(s)",
            outcome.diagnostics.len()
        );
    } else {
        println!(
            "architecture: ok: {} crates match {POLICY_FILE}",
            outcome.crates
        );
    }
    if with_fixtures {
        match fixtures::verify(&workspace_root(&manifest).join(FIXTURES_DIR)) {
            Ok(count) => println!("architecture: {count} negative fixtures fail as expected"),
            Err(errors) => {
                for error in errors {
                    eprintln!("xtask: architecture: {error}");
                }
                failed = true;
            }
        }
    }
    i32::from(failed)
}

/// Checks the workspace whose root manifest is `manifest`.
pub fn check(manifest: &Path) -> Outcome {
    // Cargo runs in the workspace root, so a relative manifest path would resolve twice.
    let manifest = std::path::absolute(manifest).unwrap_or_else(|_| manifest.to_path_buf());
    let mut diagnostics = Vec::new();
    let crates = check_into(&manifest, &mut diagnostics);
    diagnostics.sort();
    diagnostics.dedup();
    Outcome {
        diagnostics,
        crates,
    }
}

fn check_into(manifest: &Path, out: &mut Vec<Diagnostic>) -> usize {
    let root = workspace_root(manifest);
    let policy = match fs::read_to_string(root.join(POLICY_FILE)) {
        Ok(text) => match policy::parse(&text) {
            Ok(policy) => policy,
            Err(err) => {
                out.push(Diagnostic::new(Rule::PolicySchema, err, POLICY_FILE));
                return 0;
            }
        },
        Err(err) => {
            out.push(Diagnostic::new(
                Rule::PolicySchema,
                format!("cannot read: {err}"),
                POLICY_FILE,
            ));
            return 0;
        }
    };
    out.extend(inventory::check_categories(&policy));
    match inventory::check_members_on_disk(&root) {
        Ok(missing) if missing.is_empty() => {}
        Ok(missing) => {
            out.extend(missing);
            return 0;
        }
        Err(diagnostic) => {
            out.push(diagnostic);
            return 0;
        }
    }
    let metadata = match load_metadata(manifest, &root) {
        Ok(metadata) => metadata,
        Err(diagnostic) => {
            out.push(diagnostic);
            return 0;
        }
    };
    let members = workspace_crates(&metadata);
    match inventory::scan_source_tree(&root) {
        Ok(disk) => out.extend(inventory::check_crates(&policy, &members, &disk)),
        Err(diagnostic) => out.push(diagnostic),
    }
    let manifests: BTreeMap<String, String> = members
        .iter()
        .map(|c| (c.name.clone(), c.manifest.clone()))
        .collect();
    out.extend(graph::check(
        &policy,
        &graph::Graph::from_metadata(&metadata),
        &manifests,
    ));
    members.len()
}

fn load_metadata(manifest: &Path, root: &Path) -> Result<Metadata, Diagnostic> {
    MetadataCommand::new()
        .manifest_path(manifest)
        .current_dir(root)
        .other_options(vec!["--locked".to_owned()])
        .exec()
        .map_err(|err| {
            Diagnostic::new(
                Rule::Metadata,
                format!("cargo metadata failed: {}", scrub(&err.to_string(), root)),
                "Cargo.toml",
            )
        })
}

/// Workspace members with manifests relative to the workspace root.
fn workspace_crates(metadata: &Metadata) -> Vec<Crate> {
    metadata
        .workspace_packages()
        .into_iter()
        .map(|package| Crate {
            name: package.name.to_string(),
            manifest: relative(&metadata.workspace_root, &package.manifest_path),
        })
        .collect()
}

fn relative(
    root: &cargo_metadata::camino::Utf8Path,
    path: &cargo_metadata::camino::Utf8Path,
) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| path.to_string(), |p| p.as_str().replace('\\', "/"))
}

/// Puts Cargo's message on one line and replaces the absolute workspace root in it, so
/// output stays one diagnostic per line and path-relative.
fn scrub(message: &str, root: &Path) -> String {
    let mut out = message
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    for form in [fs::canonicalize(root).ok(), Some(root.to_path_buf())]
        .into_iter()
        .flatten()
    {
        let form = form.to_string_lossy().into_owned();
        if form.len() > 1 {
            out = out.replace(&form, ".");
        }
    }
    out
}

fn workspace_root(manifest: &Path) -> PathBuf {
    match manifest.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The root manifest of this repository, two levels above the xtask crate.
fn repository_manifest() -> PathBuf {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask.ancestors().nth(2).unwrap_or(xtask).join("Cargo.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_join_lines_and_replace_root_when_cargo_message_names_it() {
        // given
        let root = Path::new("/somewhere/ws");
        let message =
            "error: bad\n  failed to load manifest at /somewhere/ws/crates/a/Cargo.toml\n";

        // when
        let scrubbed = scrub(message, root);

        // then
        assert_eq!(
            scrubbed,
            "error: bad | failed to load manifest at ./crates/a/Cargo.toml"
        );
    }

    #[test]
    fn should_use_current_directory_when_manifest_has_no_parent() {
        // given
        let manifest = Path::new("Cargo.toml");

        // when
        let root = workspace_root(manifest);

        // then
        assert_eq!(root, PathBuf::from("."));
    }

    #[test]
    fn should_pass_when_repository_is_checked() {
        // given
        let manifest = repository_manifest();

        // when
        let outcome = check(&manifest);

        // then
        assert_eq!(outcome.diagnostics, []);
        assert!(outcome.crates >= 9, "{}", outcome.crates);
    }
}
