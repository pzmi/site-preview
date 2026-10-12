//! The check catalog: every gate stage and `xtask` command, with its level, owner and whether
//! `check` includes it. `list-checks` prints it; `main` reads owners from it.

/// One catalog row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check {
    /// Unique id. Equals the subcommand name for a command row.
    pub id: &'static str,
    /// Test or gate level, as named in foundation.md section 5.
    pub level: &'static str,
    /// The command line that runs it.
    pub command: &'static str,
    /// Task id that introduces (or will introduce) the implementation.
    pub owner: &'static str,
    /// Whether `cargo xtask check` includes it. A stage that is not yet implemented is `false`
    /// (D18, ADR 0019); the owning task flips it when it registers the stage.
    pub in_check: bool,
}

const fn row(
    id: &'static str,
    level: &'static str,
    command: &'static str,
    owner: &'static str,
    in_check: bool,
) -> Check {
    Check {
        id,
        level,
        command,
        owner,
        in_check,
    }
}

/// All rows. `quick` runs the stage rows `fmt-check`, `architecture`, `clippy`, `unit-tests`
/// and `doctests`.
pub const CATALOG: &[Check] = &[
    row(
        "fmt-check",
        "static",
        "cargo fmt --all --check",
        "FND-03",
        true,
    ),
    row(
        "clippy",
        "static",
        "cargo clippy --locked --workspace --all-targets -- -D warnings",
        "FND-03",
        true,
    ),
    row(
        "unit-tests",
        "unit",
        "cargo nextest run --workspace --locked --profile ci",
        "TEST-01",
        true,
    ),
    row(
        "doctests",
        "unit",
        "cargo test --locked --workspace --doc",
        "TEST-01",
        true,
    ),
    row(
        "client-js",
        "unit",
        "node --test crates/*/assets/*.test.mjs",
        "HTTP-09",
        true,
    ),
    row(
        "browser",
        "e2e",
        "cargo build --locked -p preview-cli, npm ci --prefix tests/e2e/browser, npx playwright test",
        "E2E-04",
        true,
    ),
    row("fmt", "tooling", "cargo xtask fmt", "FND-03", false),
    row("quick", "gate", "cargo xtask quick", "FND-03", false),
    row(
        "architecture",
        "architecture",
        "cargo xtask architecture",
        "ARCH-01",
        true,
    ),
    row("test", "integration", "cargo xtask test", "OPS-01", false),
    row(
        "conformance",
        "conformance",
        "cargo xtask conformance",
        "FORMAL-01",
        false,
    ),
    row(
        "formal-typecheck",
        "conformance",
        "npx --prefix formal --no -- quint typecheck formal/*.qnt",
        "FORMAL-01",
        false,
    ),
    row(
        "formal-scenarios",
        "conformance",
        "npx --prefix formal --no -- quint test --backend typescript --main <module> --match <its runs> formal/scenarios.qnt",
        "FORMAL-01",
        false,
    ),
    row(
        "formal-simulate",
        "conformance",
        "npx --prefix formal --no -- quint run --backend typescript --step <relation> --invariants <all> --max-samples 1000 --max-steps 25 formal/<model>.qnt",
        "FORMAL-01",
        false,
    ),
    row(
        "coverage",
        "coverage",
        "cargo xtask coverage",
        "TEST-06",
        false,
    ),
    row(
        "security",
        "security",
        "cargo xtask security",
        "SEC-01",
        false,
    ),
    row(
        "deny",
        "security",
        "cargo deny --locked check",
        "SEC-01",
        true,
    ),
    row(
        "secrets",
        "security",
        "gitleaks git --no-banner --redact over the full history (after a git preflight)",
        "SEC-02",
        true,
    ),
    row("workflows", "security", "actionlint", "SEC-02", true),
    row(
        "action-pins",
        "security",
        "check .github/workflows and .github/actions for unpinned uses:",
        "SEC-02",
        true,
    ),
    row("check", "gate", "cargo xtask check", "OPS-01", false),
    row(
        "mutation",
        "mutation",
        "cargo xtask mutation",
        "TEST-08",
        false,
    ),
    row("fuzz", "fuzz", "cargo xtask fuzz", "SEC-04", false),
    row(
        "formal-verify",
        "formal",
        "cargo xtask formal-verify",
        "FORMAL-10",
        false,
    ),
    row(
        "release-check",
        "release",
        "cargo xtask release-check",
        "OPS-05",
        false,
    ),
    row(
        "list-checks",
        "tooling",
        "cargo xtask list-checks",
        "FND-03",
        false,
    ),
    row("doctor", "tooling", "cargo xtask doctor", "FND-04", false),
    row(
        "install-tools",
        "tooling",
        "cargo xtask install-tools",
        "FND-04",
        false,
    ),
];

/// Every catalog row, then one row per architecture rule id (ARCH-01..07): each runs in the
/// `architecture` stage, owned by the task that introduced it. `list-checks` prints these.
pub fn all_rows() -> Vec<Check> {
    let mut rows = CATALOG.to_vec();
    rows.extend(crate::architecture::Rule::ALL.iter().map(|rule| {
        row(
            rule.id(),
            "architecture",
            "cargo xtask architecture",
            rule.owner(),
            true,
        )
    }));
    rows
}

/// Finds a row by id.
pub fn find(id: &str) -> Option<&'static Check> {
    CATALOG.iter().find(|c| c.id == id)
}

/// Renders the catalog as a JSON array of objects.
pub fn to_json(checks: &[Check]) -> String {
    let rows: Vec<String> = checks
        .iter()
        .map(|c| {
            format!(
                "  {{\"id\": {}, \"level\": {}, \"command\": {}, \"owner\": {}, \"in_check\": {}}}",
                json_str(c.id),
                json_str(c.level),
                json_str(c.command),
                json_str(c.owner),
                c.in_check
            )
        })
        .collect();
    format!("[\n{}\n]\n", rows.join(",\n"))
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::process::Action;
    use clap::CommandFactory;
    use std::collections::HashSet;

    #[test]
    fn should_have_unique_ids_when_catalog_is_listed() {
        // given
        let mut seen = HashSet::new();

        // when
        let duplicates: Vec<_> = CATALOG.iter().filter(|c| !seen.insert(c.id)).collect();

        // then
        assert!(
            duplicates.is_empty(),
            "duplicate catalog ids: {duplicates:?}"
        );
    }

    #[test]
    fn should_list_every_subcommand_when_catalog_is_checked() {
        // given
        let cmd = Cli::command();

        // when
        let missing: Vec<_> = cmd
            .get_subcommands()
            .map(|s| s.get_name())
            .filter(|n| find(n).is_none())
            .collect();

        // then
        assert!(
            missing.is_empty(),
            "subcommands missing from catalog: {missing:?}"
        );
    }

    #[test]
    fn should_list_every_architecture_rule_when_checks_are_listed() {
        // given
        let rules = crate::architecture::Rule::ALL;

        // when
        let json = to_json(&all_rows());

        // then
        let mut seen = HashSet::new();
        assert!(all_rows().iter().all(|c| seen.insert(c.id)), "{json}");
        for rule in rules {
            let line = format!("\"id\": \"{}\"", rule.id());
            assert!(json.contains(&line), "{line} missing from {json}");
            assert!(rule.owner().starts_with("ARCH-0"), "{}", rule.owner());
        }
    }

    #[test]
    fn should_escape_quotes_when_rendering_json() {
        // given
        let check = row("a", "b", "say \"hi\"\\", "T-1", true);

        // when
        let json = to_json(&[check]);

        // then
        assert!(json.contains(r#""command": "say \"hi\"\\""#), "{json}");
    }

    #[test]
    fn should_show_the_command_check_runs_when_a_stage_is_listed() {
        // given
        let stages = crate::check::stages();

        // when
        let mismatched: Vec<_> = stages
            .iter()
            .filter_map(|stage| {
                let shown = match &stage.action {
                    Action::Command { program, args, .. } => std::iter::once(program.as_str())
                        .chain(args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                    Action::InProcess { display, .. } => (*display).to_owned(),
                };
                let listed = find(stage.name).map(|c| c.command);
                (listed != Some(shown.as_str())).then_some((stage.name, listed, shown))
            })
            .collect();

        // then
        assert!(mismatched.is_empty(), "{mismatched:?}");
    }
}
