//! Domain purity, filesystem authority, layer isolation and `unsafe` (foundation.md section
//! 3.3, ADR 0012, ADR 0013, ADR 0035).
//!
//! The path rules apply to production code (see [`Context`]): tests may build temporary
//! sites and fixtures. `unsafe` is rejected everywhere.

use super::super::policy::Policy;
use super::super::report::{Diagnostic, Rule};
use super::{Context, Facts, Reference, first_hit};

/// The crate whose code must stay pure.
pub const DOMAIN: &str = "preview-domain";

/// What the domain must not reach (ADR 0012): ambient I/O, processes, environment, threads,
/// the clock, randomness, async runtimes, HTTP, console output and the outer layers. Bare
/// names are prelude macros (`println!`, `env!`).
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
    &["std", "collections", "hash_map", "RandomState"],
    &["std", "hash", "RandomState"],
    &["print"],
    &["println"],
    &["eprint"],
    &["eprintln"],
    &["env"],
    &["option_env"],
    &["std", "print"],
    &["std", "println"],
    &["std", "eprint"],
    &["std", "eprintln"],
    &["std", "option_env"],
    &["core", "env"],
    &["core", "option_env"],
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
pub const FS_ADAPTER: &str = "preview-adapter-fs";

/// The bootstrap boundary that may open the root directory: package and file inside it.
pub const BOOTSTRAP: (&str, &str) = ("preview-composition", "src/bootstrap.rs");

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

/// Crates each layer must not name (foundation.md sections 2.2 and 3.3): adapters never
/// name a sibling adapter or composition, HTTP never names the watcher, no adapter
/// technology appears in the application, and the CLI names only composition.
pub const ISOLATION: &[(&str, &[&str])] = &[
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
    (
        "preview-composition",
        &["preview_cli", "preview_adapter_fs"],
    ),
    (
        "preview-cli",
        &[
            "preview_domain",
            "preview_application",
            "preview_adapter_fs",
            "preview_adapter_http",
            "preview_testkit",
            "preview_conformance",
        ],
    ),
];

/// Where a layer may still name a crate its [`ISOLATION`] entry forbids: composition builds
/// the filesystem adapter only at the bootstrap boundary, so the capability is handed on and
/// nothing else in composition reaches the adapter's path-taking API.
const ISOLATION_EXEMPT: &[(&str, &str, &str)] = &[(
    "preview-composition",
    "preview_adapter_fs",
    "src/bootstrap.rs",
)];

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
        let file = facts.in_package(&reference.site);
        if facts.package == DOMAIN
            && let Some(hit) = first_hit(reference, DOMAIN_FORBIDDEN)
        {
            out.push(Diagnostic::new(
                Rule::DomainPurity,
                format!(
                    "{DOMAIN} names `{}`{}: the domain is pure, with no I/O, clock, randomness, runtime, console or outer layer (ADR 0012)",
                    hit,
                    written(reference, &hit)
                ),
                reference.site.at(),
            ));
        }
        let bootstrap = facts.package == BOOTSTRAP.0 && file == BOOTSTRAP.1;
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
            let prefixes: Vec<[&str; 1]> = crates
                .iter()
                .filter(|c| {
                    !ISOLATION_EXEMPT
                        .iter()
                        .any(|(p, k, f)| *p == facts.package && k == *c && *f == file)
                })
                .map(|c| [*c])
                .collect();
            let prefixes: Vec<&[&str]> = prefixes.iter().map(|p| p.as_slice()).collect();
            if let Some(hit) = first_hit(reference, &prefixes) {
                out.push(Diagnostic::new(
                    Rule::LayerIsolation,
                    format!(
                        "{} names `{}`{}: this layer must not reach {}{}",
                        facts.package,
                        hit,
                        written(reference, &hit),
                        crates.join(", "),
                        exemption_note(&facts.package)
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

fn exemption_note(package: &str) -> String {
    ISOLATION_EXEMPT
        .iter()
        .filter(|(p, _, _)| *p == package)
        .map(|(_, krate, file)| format!(" ({krate} only in {file})"))
        .collect()
}

/// ` (written `x`)` when the source spells the hit differently.
fn written(reference: &Reference, hit: &str) -> String {
    if reference.written == hit {
        String::new()
    } else if reference.open_tail {
        format!(" (written `{}::$..` in a macro)", reference.written)
    } else if reference.open_head {
        format!(" (written `$..::{}` in a macro)", reference.written)
    } else {
        format!(" (written `{}`)", reference.written)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{facts_for, policy, rendered};
    use super::super::{RefKind, Site};
    use super::*;

    fn reference(written: &str, resolved: &[&str], open: (bool, bool)) -> Reference {
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
            open_tail: open.0,
            open_head: open.1,
            kind: RefKind::Path,
            module: Vec::new(),
            through: Vec::new(),
        }
    }

    /// Diagnostics of these rules for `package` with `files` (the first is `src/lib.rs`).
    fn diagnostics(package: &str, files: &[(&str, &str)]) -> Vec<String> {
        rendered(&check(&facts_for(package, files), &policy()))
    }

    #[test]
    fn should_hit_when_candidate_is_under_forbidden_prefix() {
        // given
        let aliased = reference("f::read", &["std::fs::read"], (false, false));
        let allowed = reference("Instant", &["std::time::Instant"], (false, false));

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
        let open = reference("std", &["std"], (true, false));
        let closed = reference("std", &["std"], (false, false));

        // when
        let hit = first_hit(&open, DOMAIN_FORBIDDEN);
        let miss = first_hit(&closed, DOMAIN_FORBIDDEN);

        // then
        assert_eq!(hit.as_deref(), Some("std::fs"));
        assert_eq!(miss, None);
    }

    #[test]
    fn should_hit_when_macro_head_is_a_metavariable() {
        // given
        let tail = reference("fs::read", &["fs::read"], (false, true));
        let harmless = reference("new", &["new"], (false, true));

        // when
        let hit = first_hit(&tail, DOMAIN_FORBIDDEN);
        let miss = first_hit(&harmless, DOMAIN_FORBIDDEN);

        // then
        assert_eq!(hit.as_deref(), Some("std::fs"));
        assert_eq!(miss, None);
    }

    #[test]
    fn should_mention_written_form_when_it_differs() {
        // given
        let aliased = reference("f::read", &["std::fs::read"], (false, false));

        // when
        let text = written(&aliased, "std::fs::read");

        // then
        assert_eq!(text, " (written `f::read`)");
    }

    #[test]
    fn should_report_domain_when_reexport_is_cross_module_or_test_gated() {
        // given
        let files = [
            (
                "src/lib.rs",
                "mod a;\nmod b;\npub fn load() {\n    let _ = crate::b::g::read(\"x\");\n}\n",
            ),
            ("src/a.rs", "#[cfg(test)]\npub use std::fs as f;\n"),
            ("src/b.rs", "pub(crate) use crate::a::f as g;\n"),
        ];

        // when
        let got = diagnostics(DOMAIN, &files);

        // then
        assert!(
            got.iter().any(|d| d.starts_with("ARCH-domain-purity")
                && d.contains("`std::fs::read` (written `crate::b::g::read`)")
                && d.ends_with("src/lib.rs:4)")),
            "{got:?}"
        );
    }

    #[test]
    fn should_report_domain_when_console_env_macros_or_random_state_are_used() {
        // given
        let files = [(
            "src/lib.rs",
            "use std::collections::hash_map::RandomState;\npub fn f() {\n    println!(\"x\");\n    let _ = env!(\"HOME\");\n}\n",
        )];

        // when
        let got = diagnostics(DOMAIN, &files);

        // then
        let lines: Vec<&str> = got
            .iter()
            .filter(|d| d.starts_with("ARCH-domain-purity"))
            .map(|d| d.rsplit(':').next().unwrap_or(""))
            .collect();
        assert_eq!(lines, ["1)", "3)", "4)"], "{got:?}");
    }

    #[test]
    fn should_report_fs_authority_when_application_probes_paths_or_uses_tokio_fs() {
        // given
        let files = [(
            "src/lib.rs",
            "use std::path::Path;\npub async fn f(p: &Path) -> bool {\n    let _ = tokio::fs::read(p).await;\n    Path::exists(p)\n}\n",
        )];

        // when
        let got = diagnostics("preview-application", &files);

        // then
        let hits: Vec<&String> = got
            .iter()
            .filter(|d| d.starts_with("ARCH-fs-authority"))
            .collect();
        assert_eq!(hits.len(), 2, "{got:?}");
        assert!(hits[0].contains("`tokio::fs::read`"), "{hits:?}");
        assert!(hits[1].contains("`std::path::Path::exists`"), "{hits:?}");
    }

    #[test]
    fn should_report_isolation_when_layer_names_a_forbidden_crate() {
        // given
        let cases = [
            ("preview-application", "pub fn f(_: axum::Router) {}\n"),
            (
                "preview-adapter-fs",
                "pub use preview_adapter_http::Server;\n",
            ),
            (
                "preview-composition",
                "pub use preview_adapter_fs::RootDir;\n",
            ),
            ("preview-cli", "pub use preview_domain::Revision;\n"),
        ];

        // when
        let got: Vec<Vec<String>> = cases
            .iter()
            .map(|(package, source)| diagnostics(package, &[("src/lib.rs", source)]))
            .collect();

        // then
        for (case, found) in cases.iter().zip(&got) {
            assert!(
                found.iter().any(|d| d.starts_with("ARCH-layer-isolation")),
                "{case:?}: {found:?}"
            );
        }
    }

    #[test]
    fn should_allow_fs_adapter_when_composition_names_it_in_bootstrap() {
        // given
        let files = [
            ("src/lib.rs", "mod bootstrap;\npub use bootstrap::start;\n"),
            (
                "src/bootstrap.rs",
                "pub fn start() -> std::io::Result<()> {\n    let _ = std::fs::read_dir(\".\")?;\n    preview_adapter_fs::open()\n}\n",
            ),
        ];

        // when
        let got = diagnostics("preview-composition", &files);

        // then
        assert_eq!(got, Vec::<String>::new());
    }

    #[test]
    fn should_report_each_kind_when_code_is_unsafe() {
        // given
        let files = [(
            "src/lib.rs",
            "pub unsafe fn a() {}\nstruct S;\nunsafe impl Send for S {}\nunsafe extern \"C\" {}\n#[unsafe(no_mangle)]\npub fn b() {}\nunsafe trait T {}\n",
        )];

        // when
        let got = diagnostics("preview-adapter-fs", &files);

        // then
        let kinds: Vec<&str> = got
            .iter()
            .filter(|d| d.starts_with("ARCH-unsafe-code"))
            .filter_map(|d| d.split(" contains ").nth(1))
            .filter_map(|d| d.split(';').next())
            .collect();
        assert_eq!(
            kinds,
            [
                "an `unsafe fn`",
                "an `unsafe impl`",
                "an `extern` block",
                "an `#[unsafe(..)]` attribute",
                "an `unsafe trait`"
            ],
            "{got:?}"
        );
    }

    #[test]
    fn should_report_domain_when_macro_passes_a_crate_to_a_metavariable_head() {
        // given
        let files = [(
            "src/lib.rs",
            "macro_rules! via {\n    ($k:ident) => {\n        $k::fs::read(\"x\")\n    };\n}\npub fn f() {\n    let _ = via!(std);\n}\n",
        )];

        // when
        let got = diagnostics(DOMAIN, &files);

        // then
        assert!(
            got.iter()
                .any(|d| d.starts_with("ARCH-domain-purity") && d.ends_with("src/lib.rs:3)")),
            "{got:?}"
        );
    }

    #[test]
    fn should_report_domain_when_macro_path_continues_with_metavariable() {
        // given
        let files = [(
            "src/lib.rs",
            "macro_rules! call {\n    ($m:ident) => {\n        std::$m::read(\"x\")\n    };\n}\n",
        )];

        // when
        let got = diagnostics(DOMAIN, &files);

        // then
        assert!(
            got.iter().any(|d| d.starts_with("ARCH-domain-purity")
                && d.contains("(written `std::$..` in a macro)")),
            "{got:?}"
        );
    }
}
