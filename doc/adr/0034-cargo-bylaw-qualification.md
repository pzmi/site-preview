# 0034. cargo-bylaw is not qualified; semantic rules use syn

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

ADR 0015 (D14) asks ARCH-05 to qualify `cargo-bylaw` for the semantic rules of foundation.md §3.3 before ARCH-06 builds them. §3.3 rejects a tool whose absence, no-match result, parse failure or unsupported construct reads as a pass. The spike ran in a throwaway workspace outside the repository: `cargo install --locked cargo-bylaw@0.1.0`, the only release on crates.io (published 2026-08-14, 213 crates including `ra_ap_*` 0.0.347, about 4 minutes to build), with toolchain 1.97.0 and `rust-src` present. One library crate held one module per case, a sibling crate, a `build.rs` writing `OUT_DIR/gen.rs` and a function-like proc-macro. The rules were `forbid-dependencies` with `scope = "actual"`.

| Case | Observed |
|:--|:--|
| Macro-generated `use std::fs as hidden` in a `macro_rules!` | Found (Use and Call at the invocation site) |
| `pub use std::fs as f` and `f::metadata` in another module | Found at both the re-export and the use site |
| `#[cfg(target_os = "linux")]` and `"macos"` branches | Only the active target's branch is analysed; the macOS branch was found only with `--target aarch64-apple-darwin` |
| Trait in one module implemented in another | The `impl` is recorded as a Type dependency on the trait's module |
| Generated code (`include!` of `build.rs` output; proc-macro output) | Found in both, after running the build script and the proc-macro server |
| Selector `modules = ["std::fs", "std::fs::**"]` | Matched nothing, with no diagnostic: the rule passed |
| Selector `external_crates = ["std"]` | Hits, but also `std::collections::HashMap`: std is one node, so `std::fs` cannot be told apart from allowed std modules |
| Selector naming a misspelled module (`spike_domian::**`) | "architecture checks passed", exit 0 |
| A file with a syntax error | No diagnostic, exit 0 |

## Decision

`cargo-bylaw` 0.1.0 is **not qualified**. Its rust-analyzer analysis handles macros, re-exports, generated code and trait impls well. It still fails §3.3 on three counts: it cannot express module-level std rules (`std::fs`, `std::net`, `std::time::Instant::now`), a selector that matches nothing passes, and an unparseable file passes. Its rule kinds (`forbid-dependencies`, `only-dependencies`, `layers`, `no-cycles`) constrain dependency edges only, so `unsafe` and the hygiene rules of ARCH-07 need another checker anyway. Per ADR 0015, ARCH-06 and ARCH-07 implement the semantic rules in xtask with `syn` (`full`, `visit`). The tool is not added to `tools/versions.toml` or to any gate.

## Consequences

- ARCH-06 must cover what `syn` does not see: it fails on any file that does not parse, on `include!` of a non-literal path, and on a forbidden path inside a macro body or invocation token stream (scanned conservatively). It analyses every `cfg` branch, which is stricter than per-target analysis.
- Re-evaluate on a later `cargo-bylaw` release that has module-level selectors for toolchain crates and fails on parse errors and empty selectors; adopting it then needs a new ADR that supersedes this one.
