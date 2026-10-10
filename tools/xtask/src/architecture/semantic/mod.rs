//! The semantic layer (foundation.md section 3.3, ADR 0015, ADR 0034, ADR 0035): rules about
//! what source code names, checked by parsing every workspace crate with `syn`.
//!
//! For each Cargo target of each workspace package, the module tree is walked from the
//! target's root file through `mod` declarations (`#[path]` and `cfg_attr(.., path)`
//! included), inline modules and `include!` of a literal path. Every `cfg` branch is read:
//! nothing is evaluated, so code for another platform is checked too. Code that cannot be
//! analysed fails instead of passing: a file that does not parse, a module file that does
//! not exist, and an `include!` or `#[path]` whose path is not a literal.
//!
//! The collector records facts (resolved path references, `unsafe` sites) with their file,
//! line and context (production or test code); the rule modules turn facts into
//! diagnostics.

mod collect;
mod paths;
mod rules_purity;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, Package, Target};
use syn::{Attribute, Item, Meta};

use super::policy::Policy;
use super::report::{Diagnostic, Rule};

/// How deep `mod` and `include!` nesting is followed before it is reported as a cycle.
const MAX_NESTING: usize = 64;

/// Where code is compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Context {
    /// Shipped code: library, binary and build-script targets outside test-only items.
    Production,
    /// Test, bench and example targets, `#[cfg(test)]` items and `#[test]` functions.
    Test,
}

/// A place in the source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    /// File, relative to the workspace root.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// Whether the code there is production or test code.
    pub context: Context,
}

impl Site {
    /// `file:line`, the location a diagnostic reports.
    pub fn at(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }
}

/// How a path is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    /// A path in code, a type, an attribute or a `use` tree.
    Path,
    /// A macro invocation's path.
    Macro,
}

/// One path the code names, with every absolute path it may resolve to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    /// Where it is written.
    pub site: Site,
    /// The path as written.
    pub written: String,
    /// Candidate absolute paths; a path into the same target starts with `crate`.
    pub resolved: Vec<Vec<String>>,
    /// Written in a macro body before a metavariable (`std::$m`): it may expand to any path
    /// below each candidate.
    pub open_tail: bool,
    /// Path or macro invocation.
    pub kind: RefKind,
}

/// What the collector found in one workspace package, across all its targets.
#[derive(Debug, Default)]
pub struct Facts {
    /// Package name.
    pub package: String,
    /// The package directory, relative to the workspace root.
    pub dir: String,
    /// Every path reference.
    pub references: Vec<Reference>,
    /// Every `unsafe` block, function, impl, trait, extern block or attribute.
    pub unsafe_sites: Vec<(Site, &'static str)>,
    /// Code that cannot be analysed, with the reason.
    pub problems: Vec<(String, String)>,
}

impl Facts {
    /// The path of `site`'s file relative to the package directory.
    pub fn in_package<'a>(&self, site: &'a Site) -> &'a str {
        site.file
            .strip_prefix(&self.dir)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(&site.file)
    }
}

/// Checks every workspace package of `metadata` against the semantic rules.
pub fn check(metadata: &Metadata, policy: &Policy) -> Vec<Diagnostic> {
    let root = metadata.workspace_root.as_std_path();
    let mut out = Vec::new();
    for package in metadata.workspace_packages() {
        let facts = collect_package(root, package);
        for (at, problem) in &facts.problems {
            out.push(Diagnostic::new(
                Rule::SourceParse,
                problem.clone(),
                at.clone(),
            ));
        }
        out.extend(rules_purity::check(&facts, policy));
    }
    // Line numbers are computed; the parsed sources need not stay in this thread's span map.
    proc_macro2::extra::invalidate_current_thread_spans();
    out
}

fn collect_package(root: &Path, package: &Package) -> Facts {
    let dir = package
        .manifest_path
        .parent()
        .map(|p| relative(root, p.as_std_path()))
        .unwrap_or_default();
    let mut facts = Facts {
        package: package.name.to_string(),
        dir,
        ..Facts::default()
    };
    let renames: BTreeMap<String, String> = package
        .dependencies
        .iter()
        .filter_map(|d| {
            d.rename
                .as_ref()
                .map(|r| (r.replace('-', "_"), d.name.replace('-', "_")))
        })
        .collect();
    for target in &package.targets {
        let units = match load_target(root, target) {
            Ok(units) => units,
            Err(problems) => {
                facts.problems.extend(problems);
                continue;
            }
        };
        collect::collect_target(root, &units, renames.clone(), &mut facts);
    }
    facts
}

/// One module's items from one file. A module whose items come from several files (through
/// `include!`) has one unit per file.
pub struct Unit {
    /// Module path inside the target; the root is empty.
    pub module: Vec<String>,
    /// Absolute path of the file the items come from.
    pub file: PathBuf,
    /// Whether the whole module is test-only.
    pub test: bool,
    /// The items, with inline and out-of-line child modules moved to their own units.
    pub items: Vec<Item>,
}

/// Parses a target's module tree. Problems are reported per file; one problem anywhere
/// fails the target.
fn load_target(root: &Path, target: &Target) -> Result<Vec<Unit>, Vec<(String, String)>> {
    let test = target
        .kind
        .iter()
        .any(|k| matches!(k.to_string().as_str(), "test" | "bench" | "example"));
    let file = target.src_path.as_std_path().to_path_buf();
    let mut loader = Loader {
        root,
        units: Vec::new(),
        problems: Vec::new(),
        seen: BTreeSet::new(),
    };
    let mod_dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    loader.load_file(&file, Vec::new(), &mod_dir, test, 0, None);
    if loader.problems.is_empty() {
        Ok(loader.units)
    } else {
        Err(loader.problems)
    }
}

struct Loader<'a> {
    root: &'a Path,
    units: Vec<Unit>,
    problems: Vec<(String, String)>,
    seen: BTreeSet<(PathBuf, Vec<String>)>,
}

impl Loader<'_> {
    fn problem(&mut self, at: String, message: String) {
        self.problems.push((at, message));
    }

    /// Parses `file` as the module `module` whose child modules live in `mod_dir`.
    fn load_file(
        &mut self,
        file: &Path,
        module: Vec<String>,
        mod_dir: &Path,
        test: bool,
        depth: usize,
        from: Option<String>,
    ) {
        let rel = relative(self.root, file);
        let at = from.unwrap_or_else(|| rel.clone());
        if depth > MAX_NESTING || !self.seen.insert((file.to_path_buf(), module.clone())) {
            self.problem(at, format!("`{rel}` is loaded recursively"));
            return;
        }
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(err) => {
                self.problem(at, format!("cannot read `{rel}`: {err}"));
                return;
            }
        };
        match syn::parse_file(&text) {
            Ok(parsed) => {
                let base = file.parent().map(Path::to_path_buf).unwrap_or_default();
                let mut ctx = Place {
                    file: file.to_path_buf(),
                    rel,
                    module,
                    mod_dir: mod_dir.to_path_buf(),
                    path_base: base,
                    test,
                    depth,
                };
                self.load_items(parsed.items, &mut ctx);
            }
            Err(err) => {
                let line = err.span().start().line;
                self.problem(
                    format!("{rel}:{line}"),
                    format!("`{rel}` does not parse: {err}; unparsed code is never a pass"),
                );
            }
        }
    }

    /// Moves `items` into units: child modules get their own, the rest stays in `ctx`.
    fn load_items(&mut self, items: Vec<Item>, ctx: &mut Place) {
        let mut kept = Vec::new();
        for item in items {
            match item {
                Item::Mod(mut m) => {
                    let name = paths::name(&m.ident);
                    let mut child = ctx.module.clone();
                    child.push(name.clone());
                    let test = ctx.test || is_test_only(&m.attrs);
                    let line = m.ident.span().start().line;
                    let paths = path_attributes(&m.attrs);
                    if let Some((_, inner)) = m.content.as_mut() {
                        let inner = std::mem::take(inner);
                        let mod_dir = ctx.mod_dir.join(&name);
                        let mut inner_ctx = Place {
                            module: child,
                            mod_dir: mod_dir.clone(),
                            path_base: match paths.first() {
                                Some(Ok(p)) => ctx.path_base.join(p),
                                _ => mod_dir,
                            },
                            test,
                            ..ctx.clone()
                        };
                        self.load_items(inner, &mut inner_ctx);
                    } else {
                        self.load_module_file(ctx, &name, child, test, line, &paths);
                    }
                    kept.push(Item::Mod(m));
                }
                Item::Macro(mac) if mac.ident.is_none() && is_include(&mac.mac.path) => {
                    let line = mac
                        .mac
                        .path
                        .segments
                        .last()
                        .map_or(0, |s| s.ident.span().start().line);
                    let at = format!("{}:{line}", ctx.rel);
                    match include_path(&mac.mac.tokens) {
                        Some(path) => {
                            let file = ctx.file.parent().unwrap_or(Path::new("")).join(path);
                            let mod_dir = ctx.mod_dir.clone();
                            self.load_file(
                                &file,
                                ctx.module.clone(),
                                &mod_dir,
                                ctx.test,
                                ctx.depth + 1,
                                Some(at),
                            );
                        }
                        None => self.problem(at, non_literal("include!")),
                    }
                }
                other => kept.push(other),
            }
        }
        self.units.push(Unit {
            module: ctx.module.clone(),
            file: ctx.file.clone(),
            test: ctx.test,
            items: kept,
        });
    }

    fn load_module_file(
        &mut self,
        ctx: &Place,
        name: &str,
        module: Vec<String>,
        test: bool,
        line: usize,
        paths: &[Result<String, ()>],
    ) {
        let at = format!("{}:{line}", ctx.rel);
        let mut candidates: Vec<(PathBuf, bool)> = Vec::new();
        for path in paths {
            match path {
                Ok(p) => candidates.push((ctx.path_base.join(p), true)),
                Err(()) => {
                    self.problem(at.clone(), non_literal("#[path]"));
                    return;
                }
            }
        }
        let explicit = !candidates.is_empty();
        let defaults = [
            ctx.mod_dir.join(format!("{name}.rs")),
            ctx.mod_dir.join(name).join("mod.rs"),
        ];
        let existing: Vec<(PathBuf, bool)> = if explicit {
            candidates
                .into_iter()
                .filter(|(p, _)| p.is_file())
                .collect()
        } else {
            defaults
                .iter()
                .find(|p| p.is_file())
                .map(|p| (p.clone(), p.ends_with("mod.rs")))
                .into_iter()
                .collect()
        };
        if existing.is_empty() {
            let tried = if explicit {
                "its #[path]".to_owned()
            } else {
                defaults
                    .iter()
                    .map(|p| format!("`{}`", relative(self.root, p)))
                    .collect::<Vec<_>>()
                    .join(" or ")
            };
            self.problem(
                at,
                format!("module `{name}` has no source file (tried {tried})"),
            );
            return;
        }
        for (file, mod_rs) in existing {
            let mod_dir = if mod_rs {
                file.parent().map(Path::to_path_buf).unwrap_or_default()
            } else {
                file.with_extension("")
            };
            self.load_file(
                &file,
                module.clone(),
                &mod_dir,
                test,
                ctx.depth + 1,
                Some(at.clone()),
            );
        }
    }
}

/// Where the loader is.
#[derive(Clone)]
struct Place {
    file: PathBuf,
    rel: String,
    module: Vec<String>,
    /// Directory of child module files.
    mod_dir: PathBuf,
    /// Directory a `#[path]` is relative to.
    path_base: PathBuf,
    test: bool,
    depth: usize,
}

fn non_literal(what: &str) -> String {
    format!(
        "{what} with a path that is not a string literal cannot be analysed (build-script \
         output and macro expansions are invisible to the checker)"
    )
}

/// Whether a macro path is `include!` (also `std::include!`, `core::include!`).
pub fn is_include(path: &syn::Path) -> bool {
    path.segments.last().is_some_and(|s| s.ident == "include")
}

/// The literal path of an `include!` invocation.
pub fn include_path(tokens: &proc_macro2::TokenStream) -> Option<String> {
    syn::parse2::<syn::LitStr>(tokens.clone())
        .ok()
        .map(|lit| lit.value())
}

/// `#[path = ".."]` and `#[cfg_attr(.., path = "..")]` values; `Err` for a non-literal.
fn path_attributes(attrs: &[Attribute]) -> Vec<Result<String, ()>> {
    let mut out = Vec::new();
    for attr in attrs {
        for meta in effective_metas(&attr.meta) {
            if let Meta::NameValue(nv) = &meta
                && nv.path.is_ident("path")
            {
                out.push(match &nv.value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) => Ok(s.value()),
                    _ => Err(()),
                });
            }
        }
    }
    out
}

/// The attribute itself, plus every attribute a `cfg_attr` may expand to (any branch).
pub fn effective_metas(meta: &Meta) -> Vec<Meta> {
    let mut out = vec![meta.clone()];
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
        && let Ok(args) = list
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
    {
        for inner in args.into_iter().skip(1) {
            out.extend(effective_metas(&inner));
        }
    }
    out
}

/// Whether `attrs` make an item test-only: `#[cfg(test)]`, `#[cfg(all(test, ..))]`, or a
/// test attribute (`#[test]`, `#[tokio::test]`).
pub fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        is_test_attribute(attr)
            || matches!(&attr.meta, Meta::List(list) if list.path.is_ident("cfg")
                && list.parse_args::<Meta>().is_ok_and(|m| cfg_requires_test(&m)))
    })
}

/// Whether `attr` marks a test function: its path ends in `test`.
pub fn is_test_attribute(attr: &Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|s| s.ident == "test")
}

/// Whether a `cfg` predicate can only hold when compiling tests.
fn cfg_requires_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(p) => p.is_ident("test"),
        Meta::List(list) => {
            let args = list
                .parse_args_with(
                    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                )
                .map(|a| a.into_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            if list.path.is_ident("all") {
                args.iter().any(cfg_requires_test)
            } else if list.path.is_ident("any") {
                !args.is_empty() && args.iter().all(cfg_requires_test)
            } else {
                false
            }
        }
        Meta::NameValue(_) => false,
    }
}

/// `path` relative to `root`, with `/` separators.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(source: &str) -> Vec<Attribute> {
        let item: syn::ItemFn = syn::parse_str(&format!("{source} fn f() {{}}")).unwrap();
        item.attrs
    }

    #[test]
    fn should_mark_item_test_only_when_cfg_requires_test() {
        // given
        let cases = [
            ("#[cfg(test)]", true),
            ("#[cfg(all(test, unix))]", true),
            ("#[cfg(any(test, unix))]", false),
            ("#[cfg(not(test))]", false),
            ("#[tokio::test]", true),
            ("#[cfg(unix)]", false),
        ];

        // when
        let got: Vec<(&str, bool)> = cases
            .iter()
            .map(|(src, _)| (*src, is_test_only(&attrs(src))))
            .collect();

        // then
        assert_eq!(got, cases);
    }

    #[test]
    fn should_read_every_branch_when_path_is_given_through_cfg_attr() {
        // given
        let attrs = attrs(
            r#"#[cfg_attr(unix, path = "u.rs")] #[cfg_attr(windows, path = concat!("w", ".rs"))]"#,
        );

        // when
        let paths = path_attributes(&attrs);

        // then
        assert_eq!(paths, [Ok("u.rs".to_owned()), Err(())]);
    }

    #[test]
    fn should_report_line_when_file_does_not_parse() {
        // given
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("lib.rs");
        fs::write(&file, "pub fn ok() {}\npub fn broken( {\n").unwrap();
        let mut loader = Loader {
            root: dir.path(),
            units: Vec::new(),
            problems: Vec::new(),
            seen: BTreeSet::new(),
        };

        // when
        loader.load_file(&file, Vec::new(), dir.path(), false, 0, None);

        // then
        assert_eq!(loader.problems.len(), 1, "{:?}", loader.problems);
        assert!(
            loader.problems[0].0.starts_with("lib.rs:"),
            "{:?}",
            loader.problems
        );
        assert!(loader.problems[0].1.contains("does not parse"));
        assert!(loader.units.is_empty());
    }

    #[test]
    fn should_follow_modules_and_includes_when_tree_is_loaded() {
        // given
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(
            root.join("lib.rs"),
            "mod a;\n#[cfg(test)]\nmod t { mod inner; }\n#[path = \"other.rs\"]\nmod o;\ninclude!(\"gen.rs\");\n",
        )
        .unwrap();
        fs::write(root.join("a.rs"), "mod b;\n").unwrap();
        fs::write(root.join("a/b.rs"), "").unwrap();
        fs::create_dir_all(root.join("t")).unwrap();
        fs::write(root.join("t/inner.rs"), "").unwrap();
        fs::write(root.join("other.rs"), "").unwrap();
        fs::write(root.join("gen.rs"), "fn generated() {}\n").unwrap();
        let mut loader = Loader {
            root,
            units: Vec::new(),
            problems: Vec::new(),
            seen: BTreeSet::new(),
        };

        // when
        loader.load_file(&root.join("lib.rs"), Vec::new(), root, false, 0, None);

        // then
        assert!(loader.problems.is_empty(), "{:?}", loader.problems);
        let mut loaded: Vec<(String, String, bool)> = loader
            .units
            .iter()
            .map(|u| (u.module.join("::"), relative(root, &u.file), u.test))
            .collect();
        loaded.sort();
        assert_eq!(
            loaded,
            [
                (String::new(), "gen.rs".to_owned(), false),
                (String::new(), "lib.rs".to_owned(), false),
                ("a".to_owned(), "a.rs".to_owned(), false),
                ("a::b".to_owned(), "a/b.rs".to_owned(), false),
                ("o".to_owned(), "other.rs".to_owned(), false),
                ("t".to_owned(), "lib.rs".to_owned(), true),
                ("t::inner".to_owned(), "t/inner.rs".to_owned(), true),
            ]
        );
    }

    #[test]
    fn should_fail_when_module_file_or_literal_path_is_missing() {
        // given
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        fs::write(
            root.join("lib.rs"),
            "mod gone;\ninclude!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));\n",
        )
        .unwrap();
        let mut loader = Loader {
            root,
            units: Vec::new(),
            problems: Vec::new(),
            seen: BTreeSet::new(),
        };

        // when
        loader.load_file(&root.join("lib.rs"), Vec::new(), root, false, 0, None);

        // then
        let got: Vec<String> = loader
            .problems
            .iter()
            .map(|(at, m)| format!("{at}: {m}"))
            .collect();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            got[0].starts_with("lib.rs:1: module `gone` has no source file"),
            "{got:?}"
        );
        assert!(
            got[1].starts_with("lib.rs:2: include! with a path"),
            "{got:?}"
        );
    }
}
