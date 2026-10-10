//! Walks the units of one Cargo target twice: first to build the module scopes (`use`,
//! `extern crate`, items, child modules, also inside macro token streams), then to record
//! every path reference with its resolution, and every `unsafe` site.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{Attribute, Item, Meta};

use super::paths::{self, Scopes, TokenScan, Written, scan_tokens, use_leaves};
use super::{
    Context, Facts, RefKind, Reference, Site, Unit, include_path, is_include, is_test_only,
    non_literal, relative,
};

/// Collects the facts of one target into `facts`.
pub fn collect_target(
    root: &Path,
    units: &[Unit],
    renames: BTreeMap<String, String>,
    facts: &mut Facts,
) {
    let mut scopes = Scopes::new(renames);
    for unit in units {
        let mut binder = Binder {
            scopes: &mut scopes,
            module: unit.module.clone(),
        };
        for item in &unit.items {
            binder.visit_item(item);
        }
    }
    for unit in units {
        let mut collector = Collector {
            root,
            scopes: &scopes,
            module: unit.module.clone(),
            file: unit.file.clone(),
            rel: relative(root, &unit.file),
            test: unit.test,
            facts: &mut *facts,
            depth: 0,
        };
        for item in &unit.items {
            collector.visit_item(item);
        }
    }
}

/// Pass one: the scopes.
struct Binder<'a> {
    scopes: &'a mut Scopes,
    module: Vec<String>,
}

impl Binder<'_> {
    fn bind_tokens(&mut self, scan: &TokenScan) {
        for item in &scan.uses {
            self.scopes.add_use(&self.module, item);
        }
        for item in &scan.extern_crates {
            self.scopes.add_extern_crate(&self.module, item);
        }
    }
}

impl<'ast> Visit<'ast> for Binder<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let name = match item {
            Item::Const(i) => Some(&i.ident),
            Item::Enum(i) => Some(&i.ident),
            Item::Fn(i) => Some(&i.sig.ident),
            Item::Static(i) => Some(&i.ident),
            Item::Struct(i) => Some(&i.ident),
            Item::Trait(i) => Some(&i.ident),
            Item::TraitAlias(i) => Some(&i.ident),
            Item::Type(i) => Some(&i.ident),
            Item::Union(i) => Some(&i.ident),
            Item::Macro(i) => i.ident.as_ref(),
            _ => None,
        };
        if let Some(name) = name {
            self.scopes.add_item(&self.module, paths::name(name));
        }
        match item {
            Item::Use(u) => self.scopes.add_use(&self.module, u),
            Item::ExternCrate(e) => self.scopes.add_extern_crate(&self.module, e),
            Item::Mod(m) => {
                self.scopes.add_child(&self.module, &paths::name(&m.ident));
                // Out-of-line and inline modules are their own units; an inline module left
                // in place (inside a function body) counts for this module.
                visit::visit_item_mod(self, m);
            }
            other => visit::visit_item(self, other),
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let scan = scan_tokens(&mac.tokens);
        self.bind_tokens(&scan);
        visit::visit_macro(self, mac);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if let Meta::List(list) = &attr.meta {
            let scan = scan_tokens(&list.tokens);
            self.bind_tokens(&scan);
        }
        visit::visit_attribute(self, attr);
    }
}

/// Pass two: references and `unsafe` sites.
struct Collector<'a> {
    root: &'a Path,
    scopes: &'a Scopes,
    module: Vec<String>,
    file: PathBuf,
    rel: String,
    test: bool,
    facts: &'a mut Facts,
    depth: usize,
}

impl Collector<'_> {
    fn site(&self, line: usize) -> Site {
        Site {
            file: self.rel.clone(),
            line,
            context: if self.test {
                Context::Test
            } else {
                Context::Production
            },
        }
    }

    fn reference(&mut self, path: &Written, line: usize, kind: RefKind, open_tail: bool) {
        let resolved = self.scopes.resolve(&self.module, path);
        self.push(path, resolved, line, kind, open_tail);
    }

    fn push(
        &mut self,
        path: &Written,
        resolved: Vec<Vec<String>>,
        line: usize,
        kind: RefKind,
        open_tail: bool,
    ) {
        if resolved.is_empty() {
            return;
        }
        let site = self.site(line);
        self.facts.references.push(Reference {
            site,
            written: path.render(),
            resolved,
            open_tail,
            kind,
        });
    }

    fn use_item(&mut self, item: &syn::ItemUse) {
        for leaf in use_leaves(item) {
            let resolved = self.scopes.resolve_import(&self.module, &leaf.path);
            self.push(&leaf.path, resolved, leaf.line, RefKind::Path, false);
        }
    }

    fn extern_crate(&mut self, item: &syn::ItemExternCrate) {
        let path = Written {
            absolute: true,
            segments: vec![paths::name(&item.ident)],
        };
        let resolved = self.scopes.resolve_import(&self.module, &path);
        self.push(
            &path,
            resolved,
            item.ident.span().start().line,
            RefKind::Path,
            false,
        );
    }

    fn unsafe_site(&mut self, line: usize, what: &'static str) {
        let site = self.site(line);
        self.facts.unsafe_sites.push((site, what));
    }

    /// Records what a token stream names: paths, macro calls, `use` leaves, `unsafe`.
    fn tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        let scan = scan_tokens(tokens);
        for found in &scan.paths {
            let kind = if found.is_macro {
                RefKind::Macro
            } else {
                RefKind::Path
            };
            self.reference(&found.path, found.line, kind, found.open_tail);
        }
        for item in &scan.uses {
            self.use_item(item);
        }
        for item in &scan.extern_crates {
            self.extern_crate(item);
        }
        for line in scan.unsafe_lines {
            self.unsafe_site(line, "`unsafe` in a macro");
        }
    }

    /// Runs `f` with the test context switched on when `attrs` make the item test-only.
    fn scoped(&mut self, attrs: &[Attribute], f: impl FnOnce(&mut Self)) {
        let outer = self.test;
        self.test = outer || is_test_only(attrs);
        f(self);
        self.test = outer;
    }

    /// Visits the file an `include!` in expression or statement position names.
    fn include(&mut self, mac: &syn::Macro, line: usize) {
        let at = format!("{}:{line}", self.rel);
        let Some(path) = include_path(&mac.tokens) else {
            self.facts.problems.push((at, non_literal("include!")));
            return;
        };
        let file = self.file.parent().unwrap_or(Path::new("")).join(path);
        let rel = relative(self.root, &file);
        if self.depth > super::MAX_NESTING {
            self.facts
                .problems
                .push((at, format!("`{rel}` is included recursively")));
            return;
        }
        let parsed = fs::read_to_string(&file)
            .map_err(|e| format!("cannot read `{rel}`: {e}"))
            .and_then(|text| {
                syn::parse_file(&text)
                    .map(Included::Items)
                    .or_else(|_| syn::parse_str::<syn::Expr>(&text).map(Included::Expr))
                    .map_err(|e| format!("`{rel}` does not parse: {e}"))
            });
        match parsed {
            Ok(included) => {
                let (outer_file, outer_rel) = (
                    std::mem::replace(&mut self.file, file),
                    std::mem::replace(&mut self.rel, rel),
                );
                self.depth += 1;
                match &included {
                    Included::Items(file) => {
                        for item in &file.items {
                            self.visit_item(item);
                        }
                    }
                    Included::Expr(expr) => self.visit_expr(expr),
                }
                self.depth -= 1;
                self.file = outer_file;
                self.rel = outer_rel;
            }
            Err(problem) => self.facts.problems.push((at, problem)),
        }
    }
}

enum Included {
    Items(syn::File),
    Expr(syn::Expr),
}

fn first_line(path: &syn::Path) -> usize {
    path.segments
        .first()
        .map_or(0, |s| s.ident.span().start().line)
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs: &[Attribute] = match item {
            Item::Const(i) => &i.attrs,
            Item::Enum(i) => &i.attrs,
            Item::ExternCrate(i) => &i.attrs,
            Item::Fn(i) => &i.attrs,
            Item::ForeignMod(i) => &i.attrs,
            Item::Impl(i) => &i.attrs,
            Item::Macro(i) => &i.attrs,
            Item::Mod(i) => &i.attrs,
            Item::Static(i) => &i.attrs,
            Item::Struct(i) => &i.attrs,
            Item::Trait(i) => &i.attrs,
            Item::TraitAlias(i) => &i.attrs,
            Item::Type(i) => &i.attrs,
            Item::Union(i) => &i.attrs,
            Item::Use(i) => &i.attrs,
            _ => &[],
        };
        self.scoped(attrs, |this| visit::visit_item(this, item));
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs: &[Attribute] = match item {
            syn::ImplItem::Const(i) => &i.attrs,
            syn::ImplItem::Fn(i) => &i.attrs,
            syn::ImplItem::Type(i) => &i.attrs,
            syn::ImplItem::Macro(i) => &i.attrs,
            _ => &[],
        };
        self.scoped(attrs, |this| visit::visit_impl_item(this, item));
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attrs: &[Attribute] = match item {
            syn::TraitItem::Const(i) => &i.attrs,
            syn::TraitItem::Fn(i) => &i.attrs,
            syn::TraitItem::Type(i) => &i.attrs,
            syn::TraitItem::Macro(i) => &i.attrs,
            _ => &[],
        };
        self.scoped(attrs, |this| visit::visit_trait_item(this, item));
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        visit::visit_item_use(self, item);
        self.use_item(item);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        visit::visit_item_extern_crate(self, item);
        self.extern_crate(item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.reference(
            &Written::from_syn(path),
            first_line(path),
            RefKind::Path,
            false,
        );
        visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let line = first_line(&mac.path);
        self.reference(&Written::from_syn(&mac.path), line, RefKind::Macro, false);
        for segment in &mac.path.segments {
            visit::visit_path_arguments(self, &segment.arguments);
        }
        if is_include(&mac.path) {
            self.include(mac, line);
        }
        self.tokens(&mac.tokens);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if let Meta::List(list) = &attr.meta {
            if list.path.is_ident("unsafe") {
                self.unsafe_site(first_line(&list.path), "an `#[unsafe(..)]` attribute");
            }
            self.tokens(&list.tokens);
        }
        visit::visit_attribute(self, attr);
    }

    fn visit_expr_unsafe(&mut self, expr: &'ast syn::ExprUnsafe) {
        self.unsafe_site(expr.unsafe_token.span.start().line, "an `unsafe` block");
        visit::visit_expr_unsafe(self, expr);
    }

    fn visit_signature(&mut self, sig: &'ast syn::Signature) {
        if let syn::Safety::Unsafe(token) = &sig.safety {
            self.unsafe_site(token.span.start().line, "an `unsafe fn`");
        }
        visit::visit_signature(self, sig);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Some(token) = &item.unsafety {
            self.unsafe_site(token.span.start().line, "an `unsafe impl`");
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if let Some(token) = &item.unsafety {
            self.unsafe_site(token.span.start().line, "an `unsafe trait`");
        }
        visit::visit_item_trait(self, item);
    }

    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) {
        self.unsafe_site(item.abi.extern_token.span.start().line, "an `extern` block");
        visit::visit_item_foreign_mod(self, item);
    }
}
