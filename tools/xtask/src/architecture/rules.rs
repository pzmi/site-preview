//! The rule catalog: every rule id the checker can report, a one-line description and the
//! negative fixtures under `tests/architecture-invalid/` that prove it (foundation.md
//! section 3.4: a rule without a verified negative case is rejected).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::fixtures::Expected;

macro_rules! rules {
    ($($variant:ident => $id:literal, $owner:literal, $description:literal, [$($fixture:literal),+ $(,)?];)+) => {
        /// Every rule the checker enforces. The id is what a diagnostic line starts with and
        /// what a negative fixture's `expected.txt` names.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Rule {
            $(#[doc = $description] $variant,)+
        }

        impl Rule {
            /// Every rule, in catalog order.
            pub const ALL: &[Rule] = &[$(Rule::$variant),+];

            /// The rule id, for example `ARCH-unknown-crate`.
            pub fn id(self) -> &'static str {
                match self { $(Rule::$variant => $id,)+ }
            }

            /// The task that introduced the rule.
            pub fn owner(self) -> &'static str {
                match self { $(Rule::$variant => $owner,)+ }
            }

            /// One line saying what the rule rejects.
            pub fn description(self) -> &'static str {
                match self { $(Rule::$variant => $description,)+ }
            }

            /// The negative fixtures whose `expected.txt` names this rule.
            pub fn fixtures(self) -> &'static [&'static str] {
                match self { $(Rule::$variant => &[$($fixture),+],)+ }
            }
        }
    };
}

rules! {
    PolicySchema => "ARCH-policy-schema", "ARCH-01",
        "architecture.toml is missing, malformed, has an unknown field or another schema version.",
        ["policy-schema-version", "policy-unknown-field"];
    Metadata => "ARCH-metadata", "ARCH-01",
        "cargo metadata or a crate manifest cannot be read, including a cyclic workspace.",
        ["normal-cycle", "unreadable-member-manifest"];
    MissingMember => "ARCH-missing-member", "ARCH-01",
        "A [workspace] members entry has no Cargo.toml on disk.",
        ["missing-member"];
    NotAMember => "ARCH-not-a-member", "ARCH-01",
        "A crate under crates/ or tools/ is not a workspace member.",
        ["not-a-member"];
    UnknownCrate => "ARCH-unknown-crate", "ARCH-01",
        "A workspace crate is in no policy category.",
        ["unknown-crate"];
    MissingCrate => "ARCH-missing-crate", "ARCH-01",
        "The policy names a crate that is not in the workspace.",
        ["missing-crate"];
    EmptySelector => "ARCH-empty-selector", "ARCH-01",
        "A policy category is present but empty.",
        ["empty-selector"];
    DuplicateCategory => "ARCH-duplicate-category", "ARCH-01",
        "A crate is listed in more than one category, or twice in one.",
        ["duplicate-category"];
    ForbiddenEdge => "ARCH-forbidden-edge", "ARCH-02",
        "A normal workspace edge is not in [dependencies.normal].",
        ["cli-bypasses-composition", "reversed-edge"];
    UndeclaredBuildDep => "ARCH-undeclared-build-dep", "ARCH-02",
        "A build edge to a workspace crate is not in [dependencies.build].",
        ["undeclared-build-dep"];
    ForbiddenDevEdge => "ARCH-forbidden-dev-edge", "ARCH-02",
        "A dev edge targets a crate that is neither test_only nor in the normal allowlist.",
        ["forbidden-dev-edge"];
    ProductionClosure => "ARCH-production-closure", "ARCH-02",
        "A production crate reaches a test_only or tool_only crate over normal or build edges.",
        ["testkit-in-production", "transitive-test-only"];
    PolicyCycle => "ARCH-policy-cycle", "ARCH-02",
        "The [dependencies] allowlist contains a cycle.",
        ["policy-cycle"];
    UnlistedExternal => "ARCH-unlisted-external", "ARCH-03",
        "A direct third-party dependency is not in [external.<kind>] for its crate.",
        ["domain-uses-tokio", "target-specific-external", "unlisted-build-external", "unlisted-external"];
    ExternalFeature => "ARCH-external-feature", "ARCH-03",
        "A third-party edge enables a feature outside its [external.<kind>] list.",
        ["application-tokio-rt-feature"];
    ExternalSource => "ARCH-external-source", "ARCH-03",
        "A package is not from crates.io and has no [exceptions.sources] entry citing an existing ADR.",
        ["git-dependency", "path-outside-workspace", "source-exception-missing-adr"];
    SourceParse => "ARCH-source-parse", "ARCH-06",
        "A reachable source file cannot be analysed: it does not parse, a module file is missing, or an include!/#[path] is not a literal.",
        ["include-non-literal", "unparseable-source"];
    DomainPurity => "ARCH-domain-purity", "ARCH-06",
        "preview-domain production code names I/O, process, env, thread, clock, randomness, a runtime, HTTP or an outer layer.",
        ["cfg-attr-path-default", "domain-aliased-fs", "domain-fs-via-macro-head", "domain-std-fs", "domain-std-net", "domain-tokio-via-macro-path"];
    FsAuthority => "ARCH-fs-authority", "ARCH-06",
        "Production code outside preview-adapter-fs and preview-composition/src/bootstrap.rs names filesystem authority.",
        ["ambient-fs-in-application"];
    LayerIsolation => "ARCH-layer-isolation", "ARCH-06",
        "An adapter names a sibling adapter, composition or watcher, or the application names adapter technology.",
        ["composition-names-fs-adapter", "http-names-fs-adapter"];
    UnsafeCode => "ARCH-unsafe-code", "ARCH-06",
        "Owned code contains an unsafe block, fn, impl, trait, extern block or unsafe attribute.",
        ["unsafe-block"];
    MissingSelector => "ARCH-missing-selector", "ARCH-07",
        "A crate or file the semantic rules name is missing and not listed in [semantic] absent, absent lists a member, production composition lacks src/bootstrap.rs, or production application lacks its commands or queries module or impl ReloadState.",
        ["missing-bootstrap", "missing-query-selector", "missing-selector"];
    LintConfig => "ARCH-lint-config", "ARCH-07",
        "A lint is relaxed outside source: a member's [lints] is not workspace = true, a workspace lint is allowed, rustflags allow or cap lints, or clippy.toml allows.",
        ["clippy-toml-allow", "member-lints-override", "rustflags-allow"];
    UnfinishedCode => "ARCH-unfinished-code", "ARCH-07",
        "Production code calls todo!, unimplemented! or dbg! without an [exceptions.lints] entry.",
        ["todo-in-production"];
    IgnoredTest => "ARCH-ignored-test", "ARCH-07",
        "Code carries an #[ignore] attribute.",
        ["ignored-test"];
    LintSuppression => "ARCH-lint-suppression", "ARCH-07",
        "An allow, expect or coverage(off) attribute lacks an [exceptions.lints] entry citing an existing ADR, or an entry matches nothing.",
        ["allow-with-missing-adr", "stale-lint-exception", "unjustified-allow"];
    TestSleep => "ARCH-test-sleep", "ARCH-07",
        "Test code sleeps outside the bounded poll helper preview-testkit/src/poll.rs.",
        ["sleep-in-test"];
    TestName => "ARCH-test-name", "ARCH-07",
        "A test function's name does not match should_<outcome>_when_<condition>, or a macro hides a test.",
        ["badly-named-proptest", "badly-named-test"];
    TestStructure => "ARCH-test-structure", "ARCH-07",
        "A test function's body lacks the // given, // when and // then comments in that order.",
        ["missing-given-when-then"];
    CommandIsolation => "ARCH-command-isolation", "ARCH-09",
        "preview-application command code names crate::queries or an HTTP type (axum, http, hyper, tower, preview_adapter_http), also through re-exports.",
        ["command-imports-query", "command-names-http-dto"];
    QueryPurity => "ARCH-query-purity", "ARCH-09",
        "preview-application query code names crate::commands, takes &mut self or &mut of domain/application state, or calls a ReloadState method outside its read allowlist.",
        ["query-calls-command", "query-mutates-state"];
    ObservabilityIsolation => "ARCH-observability-isolation", "ARCH-09",
        "The domain names an instrumentation crate, or a tracing/log/observability/metrics value reaches a branch condition or returned value in domain or application code.",
        ["instrumentation-drives-decision"];
}

/// Compares the catalog with the negative fixtures in `dir`: every rule has fixtures, each
/// exists and names its rule, and every fixture belongs to a rule. Returns the problems.
pub fn completeness_problems(dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let mut named: BTreeMap<String, String> = BTreeMap::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.filter_map(Result::ok).filter(|e| e.path().is_dir()) {
                let case = entry.file_name().to_string_lossy().into_owned();
                let parsed = fs::read_to_string(entry.path().join("expected.txt"))
                    .map_err(|e| e.to_string())
                    .and_then(|text| Expected::parse(&text));
                match parsed {
                    Ok(expected) => {
                        named.insert(case, expected.rule);
                    }
                    Err(err) => problems.push(format!("fixture `{case}`: expected.txt: {err}")),
                }
            }
        }
        Err(err) => problems.push(format!("cannot list negative fixtures: {err}")),
    }
    let mut claimed = BTreeSet::new();
    for &rule in Rule::ALL {
        for &fixture in rule.fixtures() {
            claimed.insert(fixture);
            match named.get(fixture) {
                None => problems.push(format!(
                    "rule {} ({}) has no negative fixture `{fixture}`",
                    rule.id(),
                    rule.description()
                )),
                Some(id) if id != rule.id() => problems.push(format!(
                    "fixture `{fixture}` is cataloged for {} but its expected.txt names {id}",
                    rule.id()
                )),
                Some(_) => {}
            }
        }
    }
    for case in named.keys() {
        if !claimed.contains(case.as_str()) {
            problems.push(format!("fixture `{case}` is not in the rule catalog"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::{FIXTURES_DIR, repository_root};

    fn fixtures_dir() -> std::path::PathBuf {
        repository_root().join(FIXTURES_DIR)
    }

    /// Copies only the `expected.txt` files, which is all the completeness check reads.
    fn copy_without(source: &Path, target: &Path, skip: &str) {
        for entry in fs::read_dir(source).unwrap().map(Result::unwrap) {
            let name = entry.file_name();
            if !entry.path().is_dir() || name == skip {
                continue;
            }
            fs::create_dir(target.join(&name)).unwrap();
            fs::copy(
                entry.path().join("expected.txt"),
                target.join(&name).join("expected.txt"),
            )
            .unwrap();
        }
    }

    #[test]
    fn should_have_negative_fixture_when_rule_is_declared() {
        // given
        let dir = fixtures_dir();

        // when
        let problems = completeness_problems(&dir);

        // then
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn should_fail_completeness_when_any_negative_fixture_is_removed() {
        // given
        let source = fixtures_dir();
        let cases: Vec<&str> = Rule::ALL
            .iter()
            .flat_map(|r| r.fixtures())
            .copied()
            .collect();

        // when
        let passing: Vec<&str> = cases
            .iter()
            .copied()
            .filter(|case| {
                let copy = tempfile::TempDir::new().unwrap();
                copy_without(&source, copy.path(), case);
                completeness_problems(copy.path()).is_empty()
            })
            .collect();

        // then
        assert!(cases.len() >= 25, "{}", cases.len());
        assert!(passing.is_empty(), "removal not detected: {passing:?}");
    }

    #[test]
    fn should_report_orphan_when_fixture_is_not_cataloged() {
        // given
        let copy = tempfile::TempDir::new().unwrap();
        copy_without(&fixtures_dir(), copy.path(), "");
        fs::create_dir(copy.path().join("stray")).unwrap();
        fs::write(
            copy.path().join("stray/expected.txt"),
            "rule: ARCH-metadata\nat: Cargo.toml\n",
        )
        .unwrap();

        // when
        let problems = completeness_problems(copy.path());

        // then
        assert_eq!(problems, ["fixture `stray` is not in the rule catalog"]);
    }

    #[test]
    fn should_have_unique_ids_when_catalog_is_listed() {
        // given
        let ids: Vec<&str> = Rule::ALL.iter().map(|r| r.id()).collect();

        // when
        let unique: BTreeSet<&str> = ids.iter().copied().collect();

        // then
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
        assert!(ids.iter().all(|id| id.starts_with("ARCH-")));
        assert!(
            Rule::ALL
                .iter()
                .all(|r| r.description().ends_with('.') && !r.description().contains('\n'))
        );
    }
}
