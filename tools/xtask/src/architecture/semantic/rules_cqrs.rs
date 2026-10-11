//! Command/query separation and observability (foundation.md sections 2.4 and 3.3, bullets
//! 4, 5 and 7; ADR 0042).
//!
//! - `ARCH-command-isolation`: production code in `preview-application`'s `commands` module
//!   names nothing under `crate::queries` and no HTTP type, also through re-exports.
//! - `ARCH-query-purity`: production code in its `queries` module names nothing under
//!   `crate::commands`, takes no `&mut` receiver and no `&mut` of a domain or application
//!   state type (or of a type it cannot tell apart from one), and calls no `ReloadState`
//!   method outside the read allowlist [`RELOAD_READS`].
//! - `ARCH-observability-isolation`: domain production code names no instrumentation crate,
//!   and in domain and application production code no instrumentation value reaches a
//!   decision (see `flow`).
//!
//! Selectors fail closed (`ARCH-missing-selector`): when `preview-application` is a
//! production member, its production code must have a `commands` and a `queries` module and
//! an `impl ReloadState`, so a rename cannot turn these rules into no-ops.

use super::super::policy::Policy;
use super::super::report::{Diagnostic, Rule};
use super::flow::{INSTRUMENTATION_CRATES, INSTRUMENTATION_MODULES};
use super::rules_purity::DOMAIN;
use super::{Context, Facts, Site, first_hit};

/// The crate whose modules the command and query rules select.
pub const APPLICATION: &str = "preview-application";

/// The command module of [`APPLICATION`].
pub const COMMANDS: &str = "commands";

/// The query module of [`APPLICATION`].
pub const QUERIES: &str = "queries";

/// What a command must not name: HTTP DTOs and the HTTP adapter.
const HTTP: &[&[&str]] = &[
    &["axum"],
    &["axum_core"],
    &["http"],
    &["http_body"],
    &["hyper"],
    &["tower"],
    &["tower_http"],
    &["preview_adapter_http"],
];

/// The latest-state channel whose mutating methods a query must not call.
pub const RELOAD_STATE: &str = "ReloadState";

/// The `ReloadState` methods a query may call; every other method of its `impl` blocks
/// (`publish_next` today, and any method added later) is treated as a mutation.
pub const RELOAD_READS: &[&str] = &["new", "current", "subscribe"];

/// Crates the domain must not name: instrumentation belongs to the outer layers.
const DOMAIN_INSTRUMENTATION: &[&[&str]] = &[
    &["tracing"],
    &["tracing_core"],
    &["tracing_subscriber"],
    &["tracing_log"],
    &["log"],
    &["metrics"],
    &["opentelemetry"],
];

/// The diagnostics of these rules for one package.
pub fn check(facts: &Facts, policy: &Policy) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if facts.package == APPLICATION {
        let production = policy
            .workspace
            .production
            .as_ref()
            .is_some_and(|p| p.contains(&facts.package));
        if production {
            out.extend(selector_problems(facts));
        }
        out.extend(commands(facts));
        out.extend(queries(facts));
    }
    if facts.package == DOMAIN {
        for reference in production(&facts.references, |r| &r.site) {
            if let Some(hit) = first_hit(reference, DOMAIN_INSTRUMENTATION) {
                out.push(Diagnostic::new(
                    Rule::ObservabilityIsolation,
                    format!(
                        "{DOMAIN} names `{hit}`: instrumentation stays outside the domain, so it cannot change a domain decision (ADR 0042)"
                    ),
                    reference.site.at(),
                ));
            }
        }
    }
    if facts.package == APPLICATION || facts.package == DOMAIN {
        for decision in production(&facts.decisions, |d| &d.site) {
            let inside_instrumentation = decision
                .module
                .iter()
                .any(|m| INSTRUMENTATION_MODULES.contains(&m.as_str()));
            if inside_instrumentation {
                continue;
            }
            out.push(Diagnostic::new(
                Rule::ObservabilityIsolation,
                format!(
                    "{} uses instrumentation {} in {}: a value from {} or a module named {} must not drive a branch or a returned decision (ADR 0042)",
                    facts.package,
                    decision.instrumentation,
                    decision.position,
                    INSTRUMENTATION_CRATES.join("/"),
                    INSTRUMENTATION_MODULES.join("/"),
                ),
                decision.site.at(),
            ));
        }
    }
    out
}

/// Production items of `items`, by their site.
fn production<T>(items: &[T], site: impl Fn(&T) -> &Site) -> impl Iterator<Item = &T> {
    items
        .iter()
        .filter(move |item| site(item).context == Context::Production)
}

/// Whether code at `site` in module `module` belongs to the application module `name`: by
/// module path in its target, or by file (`src/<name>.rs`, `src/<name>/..`). Either counts.
fn in_module(facts: &Facts, site: &Site, module: &[String], name: &str) -> bool {
    let file = facts.in_package(site);
    module.first().is_some_and(|m| m == name)
        || file == format!("src/{name}.rs")
        || file.starts_with(&format!("src/{name}/"))
}

/// Whether `path` lies under `crate::<module>`.
fn under_crate_module(path: &[String], module: &str) -> bool {
    path.len() >= 2 && path[0] == "crate" && path[1] == module
}

/// The first candidate or traversed module of `reference` under `crate::<module>`, rendered.
fn names_module(reference: &super::Reference, module: &str) -> Option<String> {
    reference
        .resolved
        .iter()
        .chain(&reference.through)
        .find(|p| under_crate_module(p, module))
        .map(|p| p.join("::"))
}

fn written_note(written: &str, hit: &str) -> String {
    if written == hit {
        String::new()
    } else {
        format!(" (written `{written}`)")
    }
}

fn commands(facts: &Facts) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for reference in production(&facts.references, |r| &r.site) {
        if !in_module(facts, &reference.site, &reference.module, COMMANDS) {
            continue;
        }
        let hit = names_module(reference, QUERIES)
            .map(|h| (h, "the read side"))
            .or_else(|| first_hit(reference, HTTP).map(|h| (h, "an HTTP type")));
        if let Some((hit, what)) = hit {
            out.push(Diagnostic::new(
                Rule::CommandIsolation,
                format!(
                    "{APPLICATION} command code names `{hit}`{}, {what}: commands import no query handler, read-side implementation or HTTP DTO (foundation.md section 3.3)",
                    written_note(&reference.written, &hit)
                ),
                reference.site.at(),
            ));
        }
    }
    out
}

/// The methods of `ReloadState` a query must not call.
fn reload_mutators(facts: &Facts) -> Vec<&str> {
    let mut out: Vec<&str> = facts
        .impl_methods
        .iter()
        .filter(|m| m.site.context == Context::Production && is_reload_state(&m.self_type))
        .map(|m| m.name.as_str())
        .filter(|name| !RELOAD_READS.contains(name))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn is_reload_state(candidates: &[Vec<String>]) -> bool {
    candidates.iter().any(|c| {
        c.first().is_some_and(|s| s == "crate") && c.last().is_some_and(|s| s == RELOAD_STATE)
    })
}

fn queries(facts: &Facts) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mutators = reload_mutators(facts);
    let mut push = |site: &Site, message: String| {
        out.push(Diagnostic::new(
            Rule::QueryPurity,
            format!("{APPLICATION} query code {message} (foundation.md section 3.3)"),
            site.at(),
        ));
    };
    for reference in production(&facts.references, |r| &r.site) {
        if !in_module(facts, &reference.site, &reference.module, QUERIES) {
            continue;
        }
        if let Some(hit) = names_module(reference, COMMANDS) {
            push(
                &reference.site,
                format!(
                    "names `{hit}`{}: a query never calls a command handler to implement a read",
                    written_note(&reference.written, &hit)
                ),
            );
            continue;
        }
        let mutation = reference.resolved.iter().find_map(|c| {
            c.windows(2)
                .find(|w| w[0] == RELOAD_STATE && mutators.contains(&w[1].as_str()))
                .map(|_| c.join("::"))
        });
        if let Some(hit) = mutation {
            push(
                &reference.site,
                format!(
                    "names `{hit}`{}: a query must not publish to the {RELOAD_STATE} (allowed: {})",
                    written_note(&reference.written, &hit),
                    RELOAD_READS.join(", ")
                ),
            );
        }
    }
    for call in production(&facts.method_calls, |c| &c.site) {
        if in_module(facts, &call.site, &call.module, QUERIES)
            && mutators.contains(&call.name.as_str())
        {
            push(
                &call.site,
                format!(
                    "calls `.{}(..)`, a {RELOAD_STATE} method outside the read allowlist ({}): a query must not mutate state (method calls are matched by name)",
                    call.name,
                    RELOAD_READS.join(", ")
                ),
            );
        }
    }
    for param in production(&facts.mut_params, |p| &p.site) {
        if !in_module(facts, &param.site, &param.module, QUERIES) {
            continue;
        }
        let state = param.resolved.iter().find(|c| {
            c.first()
                .is_some_and(|s| s == "crate" || s == "preview_domain")
        });
        let why = if param.receiver {
            Some("a `&mut self` receiver".to_owned())
        } else if param.generic {
            Some(format!(
                "`{}` by `&mut` of a generic or `Self` type, which may be state",
                param.written
            ))
        } else {
            state.map(|c| format!("`{}` as `&mut {}`", param.written, c.join("::")))
        };
        if let Some(why) = why {
            push(
                &param.site,
                format!(
                    "fn `{}` takes {why}: a query observes state and never mutates it",
                    param.function
                ),
            );
        }
    }
    out
}

/// `ARCH-missing-selector` for the module and type selectors of these rules.
fn selector_problems(facts: &Facts) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for name in [COMMANDS, QUERIES] {
        let present = facts
            .modules
            .iter()
            .any(|m| m.first().is_some_and(|f| f == name));
        if !present {
            out.push(Diagnostic::new(
                Rule::MissingSelector,
                format!(
                    "the command/query rules select module `{name}` of {APPLICATION}, which its production code does not have: a rename would make them pass unchecked; rename the selector in rules_cqrs.rs too (ADR 0042)"
                ),
                format!("{}/src/{name}", facts.dir),
            ));
        }
    }
    let reload = facts
        .impl_methods
        .iter()
        .any(|m| m.site.context == Context::Production && is_reload_state(&m.self_type));
    if !reload {
        out.push(Diagnostic::new(
            Rule::MissingSelector,
            format!(
                "the query-purity rule selects the methods of `{RELOAD_STATE}` in {APPLICATION}, which has no production `impl {RELOAD_STATE}` (ADR 0042)"
            ),
            format!("{}/src", facts.dir),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::testing::{facts_for, policy, rendered};
    use super::*;

    const RELOAD: (&str, &str) = (
        "src/reload_state.rs",
        "pub struct ReloadState;\nimpl ReloadState {\n    pub fn current(&self) -> u64 { 0 }\n    pub fn publish_next(&self) {}\n}\n",
    );

    /// Diagnostics of these rules for the application with `files` (the first is the root).
    fn diagnostics(files: &[(&str, &str)]) -> Vec<String> {
        rendered(&check(&facts_for(APPLICATION, files), &policy()))
    }

    fn of(rule: Rule, got: &[String]) -> Vec<String> {
        got.iter()
            .filter(|d| d.starts_with(&format!("{}:", rule.id())))
            .cloned()
            .collect()
    }

    const ROOT: &str = "pub mod commands;\npub mod queries;\npub mod reload_state;\npub use commands::Apply;\npub use queries::Read;\n";

    #[test]
    fn should_pass_when_command_and_query_keep_their_sides() {
        // given
        let files = [
            ("src/lib.rs", ROOT),
            (
                "src/commands/mod.rs",
                "use crate::reload_state::ReloadState;\npub struct Apply;\nimpl Apply {\n    pub fn run(&mut self, s: &ReloadState) { s.publish_next(); }\n}\n",
            ),
            (
                "src/queries/mod.rs",
                "use core::fmt;\nuse crate::reload_state::ReloadState;\npub struct Read;\nimpl Read {\n    pub fn run(&self, s: &ReloadState) -> u64 { s.current() }\n}\nimpl fmt::Debug for Read {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(\"r\") }\n}\n",
            ),
            RELOAD,
        ];

        // when
        let got = diagnostics(&files);

        // then
        assert_eq!(got, Vec::<String>::new());
    }

    #[test]
    fn should_report_command_when_it_reaches_queries_through_a_reexport() {
        // given
        let files = [
            ("src/lib.rs", ROOT),
            (
                "src/commands/mod.rs",
                "pub struct Apply;\npub fn f(_: &crate::Read) {}\npub fn g(_: http::Request<()>) {}\n",
            ),
            ("src/queries/mod.rs", "pub struct Read;\n"),
            RELOAD,
        ];

        // when
        let got = of(Rule::CommandIsolation, &diagnostics(&files));

        // then
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            got[0].contains("`crate::queries::Read` (written `crate::Read`)"),
            "{got:?}"
        );
        assert!(got[1].contains("`http::Request`"), "{got:?}");
    }

    #[test]
    fn should_report_query_when_it_mutates_or_calls_a_command() {
        // given
        let files = [
            ("src/lib.rs", ROOT),
            ("src/commands/mod.rs", "pub struct Apply;\n"),
            (
                "src/queries/mod.rs",
                "use preview_domain::SiteState;\npub struct Read;\nimpl Read {\n    pub fn a(&mut self) {}\n    pub fn b(&self, s: &mut SiteState) {}\n    pub fn c<T>(&self, t: &mut T) {}\n    pub fn d(&self, r: &crate::reload_state::ReloadState) { r.publish_next(); }\n    pub fn e(&self) { let _ = crate::Apply; }\n    pub fn f(&self, r: &crate::reload_state::ReloadState) { crate::reload_state::ReloadState::publish_next(r); }\n    pub fn g(&self, buf: &mut String) { buf.push('x'); }\n}\n",
            ),
            RELOAD,
        ];

        // when
        let got = of(Rule::QueryPurity, &diagnostics(&files));

        // then
        let lines: Vec<&str> = got
            .iter()
            .map(|d| d.rsplit(':').next().unwrap_or(""))
            .collect();
        assert_eq!(lines, ["8)", "9)", "7)", "4)", "5)", "6)"], "{got:?}");
    }

    #[test]
    fn should_report_missing_selector_when_queries_module_is_renamed() {
        // given
        let files = [
            ("src/lib.rs", "pub mod commands;\npub mod reads;\n"),
            ("src/commands/mod.rs", ""),
            ("src/reads/mod.rs", ""),
        ];

        // when
        let got = of(Rule::MissingSelector, &diagnostics(&files));

        // then
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].contains("module `queries`"), "{got:?}");
        assert!(got[1].contains("impl ReloadState"), "{got:?}");
    }

    #[test]
    fn should_report_observability_when_domain_names_tracing_or_value_drives_decision() {
        // given
        let domain = facts_for(
            DOMAIN,
            &[("src/lib.rs", "pub fn f() { tracing::info!(\"x\"); }\n")],
        );
        let root = format!("{ROOT}pub mod observability;\n");
        let application = [
            ("src/lib.rs", root.as_str()),
            (
                "src/commands/mod.rs",
                "pub struct Apply;\npub fn run() -> bool {\n    let busy = crate::observability::depth() > 3;\n    if busy { return false; }\n    true\n}\n",
            ),
            ("src/queries/mod.rs", "pub struct Read;\n"),
            (
                "src/observability.rs",
                "pub fn depth() -> u8 { tracing::enabled!(tracing::Level::INFO) as u8 }\n",
            ),
            RELOAD,
        ];

        // when
        let from_domain = of(
            Rule::ObservabilityIsolation,
            &rendered(&check(&domain, &policy())),
        );
        let from_application = of(Rule::ObservabilityIsolation, &diagnostics(&application));

        // then
        assert_eq!(from_domain.len(), 1, "{from_domain:?}");
        assert!(
            from_domain[0].contains("`tracing::info`"),
            "{from_domain:?}"
        );
        assert_eq!(from_application.len(), 1, "{from_application:?}");
        assert!(
            from_application[0].contains("`busy` (bound from instrumentation) in an if condition")
                && from_application[0].ends_with("src/commands/mod.rs:4)"),
            "{from_application:?}"
        );
    }

    #[test]
    fn should_report_when_query_evades_through_glob_macro_typed_receiver_or_inline_module() {
        // given
        let files = [
            (
                "src/lib.rs",
                "pub mod commands;\npub mod reload_state;\npub use commands::Apply;\npub mod queries {\n    use crate::*;\n    pub struct Read;\n    impl Read {\n        pub fn a(&self) { let _ = Apply; }\n        pub fn b(&self, s: &reload_state::ReloadState) { let _ = format!(\"{:?}\", s.publish_next()); }\n        pub fn c(self: &mut Self) {}\n    }\n}\n",
            ),
            ("src/commands/mod.rs", "pub struct Apply;\n"),
            RELOAD,
        ];

        // when
        let got = of(Rule::QueryPurity, &diagnostics(&files));

        // then
        let lines: Vec<&str> = got
            .iter()
            .map(|d| d.rsplit(':').next().unwrap_or(""))
            .collect();
        assert_eq!(lines, ["8)", "9)", "10)"], "{got:?}");
    }
}
