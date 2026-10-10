//! Domain purity, filesystem authority, layer isolation and `unsafe` (foundation.md section
//! 3.3, ADR 0012, ADR 0013, ADR 0035).
//!
//! The path rules apply to production code (see [`Context`]): tests may build temporary
//! sites and fixtures. `unsafe` is rejected everywhere.

use super::super::policy::Policy;
use super::super::report::{Diagnostic, Rule};
use super::{Context, Facts, Reference};

/// The crate whose code must stay pure.
const DOMAIN: &str = "preview-domain";

/// What the domain must not reach (ADR 0012): ambient I/O, processes, environment, threads,
/// the clock, randomness, async runtimes, HTTP and the outer layers.
const DOMAIN_FORBIDDEN: &[&[&str]] = &[
    &["std", "fs"],
    &["std", "net"],
    &["std", "process"],
    &["std", "env"],
    &["std", "thread"],
    &["std", "os"],
    &["std", "io", "stdin"],
    &["std", "io", "stdout"],
    &["std", "io", "stderr"],
    &["std", "time", "SystemTime", "now"],
    &["std", "time", "SystemTime", "elapsed"],
    &["std", "time", "Instant", "now"],
    &["std", "time", "Instant", "elapsed"],
    &["tokio"],
    &["axum"],
    &["hyper"],
    &["http"],
    &["notify"],
    &["rustix"],
    &["rand"],
    &["getrandom"],
    &["preview_application"],
    &["preview_adapter_fs"],
    &["preview_adapter_http"],
    &["preview_composition"],
    &["preview_cli"],
];

/// The only crate with ambient filesystem authority.
const FS_ADAPTER: &str = "preview-adapter-fs";

/// The bootstrap boundary that may open the root directory: package and file inside it.
const BOOTSTRAP: (&str, &str) = ("preview-composition", "src/bootstrap.rs");

/// Paths that obtain filesystem authority: opening, listing, watching or probing paths.
const FS_AUTHORITY: &[&[&str]] = &[
    &["std", "fs"],
    &["std", "os", "unix", "fs"],
    &["std", "os", "linux", "fs"],
    &["std", "os", "macos", "fs"],
    &["tokio", "fs"],
    &["rustix", "fs"],
    &["notify"],
    &["std", "path", "Path", "exists"],
    &["std", "path", "Path", "try_exists"],
    &["std", "path", "Path", "is_file"],
    &["std", "path", "Path", "is_dir"],
    &["std", "path", "Path", "is_symlink"],
    &["std", "path", "Path", "metadata"],
    &["std", "path", "Path", "symlink_metadata"],
    &["std", "path", "Path", "read_dir"],
    &["std", "path", "Path", "read_link"],
    &["std", "path", "Path", "canonicalize"],
    &["std", "path", "PathBuf", "exists"],
    &["std", "path", "PathBuf", "try_exists"],
    &["std", "path", "PathBuf", "is_file"],
    &["std", "path", "PathBuf", "is_dir"],
    &["std", "path", "PathBuf", "is_symlink"],
    &["std", "path", "PathBuf", "metadata"],
    &["std", "path", "PathBuf", "symlink_metadata"],
    &["std", "path", "PathBuf", "read_dir"],
    &["std", "path", "PathBuf", "read_link"],
    &["std", "path", "PathBuf", "canonicalize"],
];

/// Crates each inner layer must not name (foundation.md sections 2.2 and 3.3): adapters
/// never name a sibling adapter or composition, HTTP never names the watcher, and no
/// adapter technology appears in the application.
const ISOLATION: &[(&str, &[&str])] = &[
    (
        "preview-application",
        &[
            "axum",
            "hyper",
            "http",
            "tower",
            "notify",
            "rustix",
            "preview_adapter_fs",
            "preview_adapter_http",
            "preview_composition",
            "preview_cli",
        ],
    ),
    (
        "preview-adapter-http",
        &[
            "preview_adapter_fs",
            "preview_composition",
            "preview_cli",
            "notify",
        ],
    ),
    (
        "preview-adapter-fs",
        &[
            "preview_adapter_http",
            "preview_composition",
            "preview_cli",
            "axum",
            "hyper",
        ],
    ),
    ("preview-composition", &["preview_cli"]),
];

/// The diagnostics of these rules for one package.
pub fn check(facts: &Facts, policy: &Policy) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let production = policy
        .workspace
        .production
        .as_ref()
        .is_some_and(|p| p.contains(&facts.package));
    let isolation = ISOLATION
        .iter()
        .find(|(name, _)| *name == facts.package)
        .map(|(_, crates)| *crates);
    for reference in &facts.references {
        if reference.site.context != Context::Production {
            continue;
        }
        if facts.package == DOMAIN
            && let Some(hit) = first_hit(reference, DOMAIN_FORBIDDEN)
        {
            out.push(Diagnostic::new(
                Rule::DomainPurity,
                format!(
                    "{DOMAIN} names `{}`{}: the domain is pure, with no I/O, clock, randomness, runtime or outer layer (ADR 0012)",
                    hit,
                    written(reference, &hit)
                ),
                reference.site.at(),
            ));
        }
        let bootstrap =
            facts.package == BOOTSTRAP.0 && facts.in_package(&reference.site) == BOOTSTRAP.1;
        if production
            && facts.package != FS_ADAPTER
            && !bootstrap
            && let Some(hit) = first_hit(reference, FS_AUTHORITY)
        {
            out.push(Diagnostic::new(
                Rule::FsAuthority,
                format!(
                    "{} names `{}`{}: ambient filesystem authority is allowed only in {FS_ADAPTER} and {}/{}",
                    facts.package,
                    hit,
                    written(reference, &hit),
                    BOOTSTRAP.0,
                    BOOTSTRAP.1
                ),
                reference.site.at(),
            ));
        }
        if let Some(crates) = isolation {
            let prefixes: Vec<[&str; 1]> = crates.iter().map(|c| [*c]).collect();
            let prefixes: Vec<&[&str]> = prefixes.iter().map(|p| p.as_slice()).collect();
            if let Some(hit) = first_hit(reference, &prefixes) {
                out.push(Diagnostic::new(
                    Rule::LayerIsolation,
                    format!(
                        "{} names `{}`{}: this layer must not reach {}",
                        facts.package,
                        hit,
                        written(reference, &hit),
                        crates.join(", ")
                    ),
                    reference.site.at(),
                ));
            }
        }
    }
    for (site, what) in &facts.unsafe_sites {
        out.push(Diagnostic::new(
            Rule::UnsafeCode,
            format!(
                "{} contains {what}; `unsafe` is forbidden in owned code (foundation.md section 3.3)",
                facts.package
            ),
            site.at(),
        ));
    }
    out
}

/// ` (written `x`)` when the source spells the hit differently.
fn written(reference: &Reference, hit: &str) -> String {
    if reference.written == hit {
        String::new()
    } else if reference.open_tail {
        format!(" (written `{}::$..` in a macro)", reference.written)
    } else {
        format!(" (written `{}`)", reference.written)
    }
}

/// The first candidate of `reference` under a forbidden prefix, rendered. A macro path
/// that continues with a metavariable hits when a forbidden prefix lies below it.
fn first_hit(reference: &Reference, forbidden: &[&[&str]]) -> Option<String> {
    reference.resolved.iter().find_map(|candidate| {
        forbidden.iter().find_map(|prefix| {
            let under = candidate.len() >= prefix.len()
                && candidate.iter().zip(prefix.iter()).all(|(a, b)| a == b);
            let above = reference.open_tail
                && candidate.len() < prefix.len()
                && candidate.iter().zip(prefix.iter()).all(|(a, b)| a == b);
            if under {
                Some(candidate.join("::"))
            } else if above {
                Some(prefix.join("::"))
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::super::{RefKind, Site};
    use super::*;

    fn reference(written: &str, resolved: &[&str], open_tail: bool) -> Reference {
        Reference {
            site: Site {
                file: "crates/x/src/lib.rs".to_owned(),
                line: 3,
                context: Context::Production,
            },
            written: written.to_owned(),
            resolved: resolved
                .iter()
                .map(|p| p.split("::").map(str::to_owned).collect())
                .collect(),
            open_tail,
            kind: RefKind::Path,
        }
    }

    #[test]
    fn should_hit_when_candidate_is_under_forbidden_prefix() {
        // given
        let aliased = reference("f::read", &["std::fs::read"], false);
        let allowed = reference("Instant", &["std::time::Instant"], false);

        // when
        let hit = first_hit(&aliased, DOMAIN_FORBIDDEN);
        let miss = first_hit(&allowed, DOMAIN_FORBIDDEN);

        // then
        assert_eq!(hit.as_deref(), Some("std::fs::read"));
        assert_eq!(miss, None);
    }

    #[test]
    fn should_hit_when_macro_path_may_expand_below_prefix() {
        // given
        let open = reference("std", &["std"], true);
        let closed = reference("std", &["std"], false);

        // when
        let hit = first_hit(&open, DOMAIN_FORBIDDEN);
        let miss = first_hit(&closed, DOMAIN_FORBIDDEN);

        // then
        assert_eq!(hit.as_deref(), Some("std::fs"));
        assert_eq!(miss, None);
    }

    #[test]
    fn should_mention_written_form_when_it_differs() {
        // given
        let aliased = reference("f::read", &["std::fs::read"], false);

        // when
        let text = written(&aliased, "std::fs::read");

        // then
        assert_eq!(text, " (written `f::read`)");
    }
}
