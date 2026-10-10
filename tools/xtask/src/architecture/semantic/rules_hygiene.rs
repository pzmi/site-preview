//! Hygiene and test quality (foundation.md sections 3.3 and 5.2; doc/tasks.md rules 4 and
//! 5): no unfinished code in production, no ignored tests, no unapproved lint suppression,
//! no sleeps in tests outside the bounded poll helper, and named, structured tests.

use std::collections::BTreeSet;

use super::super::POLICY_FILE;
use super::super::policy::{LintException, Policy};
use super::super::report::{Diagnostic, Rule};
use super::{Context, Facts, RefKind, Site};

/// Macros that mark unfinished code, with the clippy lint an exception names.
const UNFINISHED: &[(&str, &str)] = &[
    ("todo", "clippy::todo"),
    ("unimplemented", "clippy::unimplemented"),
    ("dbg", "clippy::dbg_macro"),
];

/// Calls that block a test on wall-clock time.
const SLEEPS: &[&[&str]] = &[
    &["std", "thread", "sleep"],
    &["std", "thread", "sleep_ms"],
    &["tokio", "time", "sleep"],
    &["tokio", "time", "sleep_until"],
];

/// The bounded polling helper (TEST-01): the one place test support may sleep, in its
/// production code. Package and file inside it.
const POLL_HELPER: (&str, &str) = ("preview-testkit", "src/poll.rs");

/// The hygiene rules over all packages: suppressions are matched against
/// `[[exceptions.lints]]`, and entries that match nothing are reported by [`Hygiene::finish`].
pub struct Hygiene<'a> {
    policy: &'a Policy,
    adr_exists: &'a dyn Fn(&str) -> bool,
    used: BTreeSet<usize>,
}

impl<'a> Hygiene<'a> {
    /// Rules for `policy`; `adr_exists` checks an exception's ADR.
    pub fn new(policy: &'a Policy, adr_exists: &'a dyn Fn(&str) -> bool) -> Self {
        Self {
            policy,
            adr_exists,
            used: BTreeSet::new(),
        }
    }

    /// The diagnostics for one package.
    pub fn check(&mut self, facts: &Facts) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        self.unfinished(facts, &mut out);
        self.suppressions(facts, &mut out);
        sleeps(facts, self.policy, &mut out);
        tests(facts, &mut out);
        out
    }

    /// Entries in `[[exceptions.lints]]` that no suppression used.
    pub fn finish(&self) -> Vec<Diagnostic> {
        self.policy
            .exceptions
            .lints
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.used.contains(i))
            .map(|(_, e)| {
                Diagnostic::new(
                    Rule::LintSuppression,
                    format!(
                        "[[exceptions.lints]] entry `{}` for `{}` (ADR {}) matches no suppression; remove it",
                        e.lint, e.path, e.adr
                    ),
                    POLICY_FILE,
                )
            })
            .collect()
    }

    /// Looks up the exception for `lint` in `site`'s file. `Ok` when one cites an existing
    /// ADR; otherwise the reason.
    fn excepted(&mut self, site: &Site, lint: &str) -> Result<(), String> {
        let found = self
            .policy
            .exceptions
            .lints
            .iter()
            .enumerate()
            .filter(|(_, e)| e.path == site.file && e.lint == lint)
            .collect::<Vec<(usize, &LintException)>>();
        for (i, _) in &found {
            self.used.insert(*i);
        }
        match found.iter().find(|(_, e)| (self.adr_exists)(&e.adr)) {
            Some(_) => Ok(()),
            None => match found.first() {
                Some((_, e)) => Err(format!(
                    "its [[exceptions.lints]] entry cites ADR {}, but no doc/adr/{}-*.md exists",
                    e.adr, e.adr
                )),
                None => Err(format!(
                    "add an [[exceptions.lints]] entry (path = \"{}\", lint = \"{lint}\") citing an ADR, or remove it",
                    site.file
                )),
            },
        }
    }

    fn unfinished(&mut self, facts: &Facts, out: &mut Vec<Diagnostic>) {
        for reference in &facts.references {
            if reference.kind != RefKind::Macro || reference.site.context != Context::Production {
                continue;
            }
            let hit = reference.resolved.iter().find_map(|candidate| {
                let (last, prefix) = candidate.split_last()?;
                let prelude = prefix.is_empty()
                    || (prefix.len() == 1
                        && matches!(prefix[0].as_str(), "std" | "core" | "alloc"));
                UNFINISHED.iter().find(|(name, _)| prelude && last == name)
            });
            if let Some((name, lint)) = hit
                && let Err(reason) = self.excepted(&reference.site, lint)
            {
                out.push(Diagnostic::new(
                    Rule::UnfinishedCode,
                    format!(
                        "{} calls `{name}!` in production code; {reason}",
                        facts.package
                    ),
                    reference.site.at(),
                ));
            }
        }
    }

    fn suppressions(&mut self, facts: &Facts, out: &mut Vec<Diagnostic>) {
        for attribute in &facts.attributes {
            if attribute.name == "ignore" {
                out.push(Diagnostic::new(
                    Rule::IgnoredTest,
                    format!(
                        "{} has `#[ignore]`: a test either runs or is deleted",
                        facts.package
                    ),
                    attribute.site.at(),
                ));
                continue;
            }
            for lint in &attribute.lints {
                if let Err(reason) = self.excepted(&attribute.site, lint) {
                    let shown = if attribute.name == "coverage" {
                        "coverage(off)".to_owned()
                    } else {
                        format!("{}({lint})", attribute.name)
                    };
                    out.push(Diagnostic::new(
                        Rule::LintSuppression,
                        format!("{} has `#[{shown}]`; {reason}", facts.package),
                        attribute.site.at(),
                    ));
                }
            }
        }
    }
}

fn sleeps(facts: &Facts, policy: &Policy, out: &mut Vec<Diagnostic>) {
    let test_crate = policy
        .workspace
        .test_only
        .as_ref()
        .is_some_and(|t| t.contains(&facts.package));
    for reference in &facts.references {
        let test = reference.site.context == Context::Test || test_crate;
        let poll_helper = facts.package == POLL_HELPER.0
            && facts.in_package(&reference.site) == POLL_HELPER.1
            && reference.site.context == Context::Production;
        if !test || poll_helper {
            continue;
        }
        let hit = reference.resolved.iter().find(|candidate| {
            SLEEPS.iter().any(|sleep| {
                candidate.len() >= sleep.len()
                    && candidate.iter().zip(sleep.iter()).all(|(a, b)| a == b)
            })
        });
        if let Some(candidate) = hit {
            out.push(Diagnostic::new(
                Rule::TestSleep,
                format!(
                    "{} test code calls `{}`: wait on an event, an injected clock or `{}::poll::wait_until` instead",
                    facts.package,
                    candidate.join("::"),
                    POLL_HELPER.0.replace('-', "_")
                ),
                reference.site.at(),
            ));
        }
    }
}

fn tests(facts: &Facts, out: &mut Vec<Diagnostic>) {
    for test in &facts.tests {
        let Some((open, close)) = test.body else {
            out.push(Diagnostic::new(
                Rule::TestName,
                format!(
                    "{} defines test `{}` inside a macro body, where its name and structure cannot be checked; write it as a plain function",
                    facts.package, test.name
                ),
                test.site.at(),
            ));
            continue;
        };
        if !well_named(&test.name) {
            out.push(Diagnostic::new(
                Rule::TestName,
                format!(
                    "{} test `{}` does not match should_<outcome>_when_<condition> (^should_[a-z0-9_]+_when_[a-z0-9_]+$)",
                    facts.package, test.name
                ),
                test.site.at(),
            ));
        }
        let text = facts
            .sources
            .get(&test.site.file)
            .map_or("", String::as_str);
        let body: Vec<&str> = text
            .lines()
            .skip(open.saturating_sub(1))
            .take(close + 1 - open.min(close))
            .collect();
        if let Err(missing) = given_when_then(&body) {
            out.push(Diagnostic::new(
                Rule::TestStructure,
                format!(
                    "{} test `{}` lacks {missing}: write // given, // when and // then in that order",
                    facts.package, test.name
                ),
                test.site.at(),
            ));
        }
    }
}

/// Whether `name` matches `^should_[a-z0-9_]+_when_[a-z0-9_]+$`.
fn well_named(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("should_") else {
        return false;
    };
    name.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && rest
            .match_indices("_when_")
            .any(|(i, _)| i > 0 && i + "_when_".len() < rest.len())
}

/// Checks that `body` has `// given`, `// when` and `// then` comment lines, in order.
fn given_when_then(body: &[&str]) -> Result<(), String> {
    let mut from = 0;
    for marker in ["// given", "// when", "// then"] {
        let found = body[from..].iter().position(|line| {
            line.trim()
                .strip_prefix(marker)
                .is_some_and(|rest| rest.is_empty() || !rest.starts_with(char::is_alphanumeric))
        });
        match found {
            Some(at) => from += at + 1,
            None => return Err(format!("`{marker}`")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_name_when_it_has_outcome_and_condition() {
        // given
        let names = [
            ("should_pass_when_repository_is_checked", true),
            ("should_x_when_y", true),
            ("should_when_y", false),
            ("should_x_when_", false),
            ("it_works", false),
            ("should_Pass_when_y", false),
        ];

        // when
        let got: Vec<(&str, bool)> = names.iter().map(|(n, _)| (*n, well_named(n))).collect();

        // then
        assert_eq!(got, names);
    }

    #[test]
    fn should_require_markers_in_order_when_body_is_checked() {
        // given
        let good = ["{", "    // given", "    // when", "    // then: done", "}"];
        let reversed = ["    // when", "    // given", "    // then"];
        let missing = ["    // given", "    // when / then"];

        // when
        let results = [
            given_when_then(&good),
            given_when_then(&reversed),
            given_when_then(&missing),
        ];

        // then
        assert_eq!(
            results,
            [
                Ok(()),
                Err("`// when`".to_owned()),
                Err("`// then`".to_owned())
            ]
        );
    }
}
