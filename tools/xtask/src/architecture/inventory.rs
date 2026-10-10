//! Crate inventory (foundation.md section 3.2, rules 1 and 7): the policy, the workspace
//! members Cargo reports and the crates on disk must describe the same set of crates.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::POLICY_FILE;
use super::policy::{Category, Policy};
use super::report::{Diagnostic, Rule};

/// Directories whose children are scanned for crates, so a crate that is not a workspace
/// member is still found.
pub const SOURCE_DIRS: [&str; 2] = ["crates", "tools"];

/// A crate and its manifest, relative to the workspace root with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    /// Package name.
    pub name: String,
    /// Manifest path relative to the workspace root.
    pub manifest: String,
}

/// Category lists that are empty, and crates listed more than once.
pub fn check_categories(policy: &Policy) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut seen: BTreeMap<&str, Vec<Category>> = BTreeMap::new();
    for (category, list) in policy.workspace.lists() {
        let Some(list) = list else { continue };
        if list.is_empty() {
            out.push(Diagnostic::new(
                Rule::EmptySelector,
                format!(
                    "`[workspace] {}` is empty; list its crates or remove the key",
                    category.key()
                ),
                POLICY_FILE,
            ));
        }
        for name in list {
            seen.entry(name).or_default().push(category);
        }
    }
    for (name, categories) in seen {
        if categories.len() > 1 {
            let keys: Vec<_> = categories.iter().map(|c| c.key()).collect();
            out.push(Diagnostic::new(
                Rule::DuplicateCategory,
                format!(
                    "crate `{name}` is listed {} times: {}",
                    keys.len(),
                    keys.join(", ")
                ),
                POLICY_FILE,
            ));
        }
    }
    out
}

/// Compares the policy, the workspace members and the crates found on disk.
pub fn check_crates(policy: &Policy, members: &[Crate], disk: &[Crate]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let member_manifests: BTreeSet<&str> = members.iter().map(|c| c.manifest.as_str()).collect();
    for krate in disk {
        if !member_manifests.contains(krate.manifest.as_str()) {
            out.push(Diagnostic::new(
                Rule::NotAMember,
                format!(
                    "crate `{}` is not in `[workspace] members` of Cargo.toml",
                    krate.name
                ),
                &krate.manifest,
            ));
        }
    }
    let categorized: BTreeSet<&str> = policy
        .workspace
        .lists()
        .into_iter()
        .flat_map(|(_, list)| list.into_iter().flatten())
        .map(String::as_str)
        .collect();
    for krate in members {
        if !categorized.contains(krate.name.as_str()) {
            out.push(Diagnostic::new(
                Rule::UnknownCrate,
                format!(
                    "workspace crate `{}` is in no `[workspace]` category of {POLICY_FILE}",
                    krate.name
                ),
                &krate.manifest,
            ));
        }
    }
    let member_names: BTreeSet<&str> = members.iter().map(|c| c.name.as_str()).collect();
    for (place, name) in policy_names(policy) {
        if !member_names.contains(name) {
            out.push(Diagnostic::new(
                Rule::MissingCrate,
                format!("`{place}` names `{name}`, which is not a workspace crate"),
                POLICY_FILE,
            ));
        }
    }
    out
}

/// Every workspace crate name the policy mentions, with where it mentions it. Deduplicated,
/// so one stale name in one place yields one diagnostic.
fn policy_names(policy: &Policy) -> BTreeSet<(String, &str)> {
    let mut out = BTreeSet::new();
    for (category, list) in policy.workspace.lists() {
        for name in list.into_iter().flatten() {
            out.insert((format!("[workspace] {}", category.key()), name.as_str()));
        }
    }
    for (krate, deps) in &policy.dependencies.normal {
        out.insert(("[dependencies.normal]".to_owned(), krate.as_str()));
        for dep in deps {
            out.insert((format!("[dependencies.normal] {krate}"), dep.as_str()));
        }
    }
    for (kind, table) in policy.external.tables() {
        for krate in table.keys() {
            out.insert((format!("[external.{kind}]"), krate.as_str()));
        }
    }
    out
}

/// `[workspace] members` entries of the root manifest that have no `Cargo.toml`. Checked
/// before `cargo metadata`, which cannot load such a workspace at all.
pub fn check_members_on_disk(root: &Path) -> Result<Vec<Diagnostic>, Diagnostic> {
    let manifest = read_toml(root, "Cargo.toml")?;
    let members = manifest
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for member in &members {
        let Some(path) = member.as_str() else {
            continue;
        };
        if !root.join(path).join("Cargo.toml").is_file() {
            out.push(Diagnostic::new(
                Rule::MissingMember,
                format!("workspace member `{path}` has no Cargo.toml (members are explicit paths)"),
                "Cargo.toml",
            ));
        }
    }
    Ok(out)
}

/// Every crate in a direct child of [`SOURCE_DIRS`], whether or not it is a member.
pub fn scan_source_tree(root: &Path) -> Result<Vec<Crate>, Diagnostic> {
    let mut out = Vec::new();
    for dir in SOURCE_DIRS {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        let mut children: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().join("Cargo.toml").is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .collect();
        children.sort();
        for child in children {
            let manifest = format!("{dir}/{child}/Cargo.toml");
            let table = read_toml(root, &manifest)?;
            if let Some(name) = table
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
            {
                out.push(Crate {
                    name: name.to_owned(),
                    manifest,
                });
            }
        }
    }
    Ok(out)
}

fn read_toml(root: &Path, relative: &str) -> Result<toml::Table, Diagnostic> {
    let text = fs::read_to_string(root.join(relative))
        .map_err(|e| Diagnostic::new(Rule::Metadata, format!("cannot read: {e}"), relative))?;
    text.parse().map_err(|e: toml::de::Error| {
        Diagnostic::new(
            Rule::Metadata,
            format!("not valid TOML: {}", e.message()),
            relative,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::policy;

    fn policy(text: &str) -> Policy {
        policy::parse(text).unwrap()
    }

    fn krate(name: &str, dir: &str) -> Crate {
        Crate {
            name: name.to_owned(),
            manifest: format!("{dir}/{name}/Cargo.toml"),
        }
    }

    const TWO: &str = "schema = 1\n[workspace]\nproduction = [\"a\"]\ntool_only = [\"t\"]\n[dependencies.normal]\na = []\nt = []\n";

    #[test]
    fn should_pass_when_policy_members_and_disk_agree() {
        // given
        let crates = [krate("a", "crates"), krate("t", "tools")];

        // when
        let mut diagnostics = check_categories(&policy(TWO));
        diagnostics.extend(check_crates(&policy(TWO), &crates, &crates));

        // then
        assert_eq!(diagnostics, []);
    }

    #[test]
    fn should_report_crate_once_when_listed_in_two_categories() {
        // given
        let text = TWO.replace("tool_only = [\"t\"]", "tool_only = [\"t\", \"a\"]");

        // when
        let diagnostics = check_categories(&policy(&text));

        // then
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::DuplicateCategory);
        assert!(diagnostics[0].message.contains("production, tool_only"));
    }

    #[test]
    fn should_report_empty_selector_when_category_list_is_empty() {
        // given
        let text = TWO.replace("tool_only = [\"t\"]", "tool_only = []");

        // when
        let diagnostics = check_categories(&policy(&text));

        // then
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::EmptySelector);
    }

    #[test]
    fn should_report_not_a_member_when_disk_crate_is_not_in_members() {
        // given
        let members = [krate("a", "crates"), krate("t", "tools")];
        let disk = [
            krate("a", "crates"),
            krate("t", "tools"),
            krate("x", "crates"),
        ];

        // when
        let diagnostics = check_crates(&policy(TWO), &members, &disk);

        // then
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::NotAMember);
        assert_eq!(diagnostics[0].at, "crates/x/Cargo.toml");
    }

    #[test]
    fn should_report_missing_crate_when_dependency_table_names_a_stale_crate() {
        // given
        let text = TWO.replace("a = []", "a = [\"gone\"]");
        let crates = [krate("a", "crates"), krate("t", "tools")];

        // when
        let diagnostics = check_crates(&policy(&text), &crates, &crates);

        // then
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, Rule::MissingCrate);
        assert!(diagnostics[0].message.contains("[dependencies.normal] a"));
    }

    #[test]
    fn should_report_missing_crate_when_external_table_names_a_stale_crate() {
        // given
        let text = format!("{TWO}[external.dev]\ngone = {{ x = \"*\" }}\n");
        let crates = [krate("a", "crates"), krate("t", "tools")];

        // when
        let diagnostics = check_crates(&policy(&text), &crates, &crates);

        // then
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("[external.dev]"));
    }
}
