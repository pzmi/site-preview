# 0035. Semantic rules: what is analysed and how names resolve

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

ADR 0034 rejected `cargo-bylaw`, so ARCH-06 implements the semantic rules of foundation.md
§3.3 in xtask with `syn` (ADR 0015). `syn` parses one file at a time and expands nothing: it
does not follow `mod` declarations, resolve names, evaluate `cfg`, or run macros, build
scripts and procedural macros. §3.3 forbids reading any of these gaps as a pass. The rules
also need a scope: integration tests build temporary sites and need the filesystem, and a
domain test needs `Instant::now()` to make an `Instant` value at all.

## Decision

- **Reach.** For every Cargo target of every workspace package (library, binaries, tests,
  benches, examples, build script), the checker walks the module tree from the target's
  root through `mod` declarations, including `#[path]` and every `cfg_attr(.., path)`
  branch, and through `include!` of a string literal. A file that does not parse, a
  declared module with no file, and an `include!` or `#[path]` that is not a literal fail
  as `ARCH-source-parse`. Files that no target reaches are not compiled and are not read.
- **Every `cfg` branch** is analysed; nothing is evaluated. This is stricter than a check
  per target.
- **Names.** Per target, `use` trees (renames, groups, `self`, globs), `extern crate`
  aliases, local items, child modules and manifest dependency renames form a scope per
  module; a `use` inside a function counts for its module. A path resolves through
  `crate::`, `self::`, `super::` and `$crate::`, across re-exports in other modules, to
  every candidate it may name, and a rule fails if any candidate is forbidden. A one-segment
  path that nothing binds is a local value.
- **Macros.** Token streams of macro invocations, `macro_rules!` bodies and attribute
  arguments are scanned for paths, `use`/`extern crate` items and `unsafe`. A macro path
  that continues with a metavariable (`std::$m`) counts as every path below its prefix.
- **Context.** Test, bench and example targets, `#[cfg(test)]` items (also
  `cfg(all(test, ..))`) and test functions are test code. Domain purity, filesystem
  authority and layer isolation apply to production code; `unsafe` is rejected everywhere.
- **Rules.** `ARCH-domain-purity` (ADR 0012), `ARCH-fs-authority` (only
  `preview-adapter-fs` and `preview-composition/src/bootstrap.rs`; the path lists include
  `tokio::fs`, `notify` and the `Path`/`PathBuf` methods that touch the filesystem),
  `ARCH-layer-isolation` (adapters never name a sibling adapter or composition, HTTP never
  names the watcher, the application never names adapter technology), `ARCH-unsafe-code`.

## Consequences

- Generated code stays invisible: build-script output reached through a non-literal
  `include!` fails, and procedural-macro output (derives, attribute macros) is not
  analysed. A workspace proc-macro crate would need its own review or a new rule.
- Method calls on values are not type-resolved: `path.exists()` or `instant.elapsed()` is
  not seen, only the paths that produced the value. The rules catch the import or the
  type-qualified call, not data flow; behaviour tests remain the second line (§3.3).
- Over-approximation can report an ambiguous name; the fix is to name the item plainly,
  never to loosen the rule.
- ARCH-07 adds the hygiene and test-quality rules on the same collector.
