# 0065. Fuzz targets beyond the domain, and the audit of the fuzz lockfile

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Amends ADR 0063.

## Context

ADR 0063 let the fuzz crate depend on the domain only, which was enough for `decode_path`.
SEC-05 adds three targets whose code under test is not all in the domain: the HTML rewriter
`inject::rewrite` lives in `preview-adapter-http` (it is `lol_html`-bound by design, ADR 0043),
and the change pipeline's `ChangeCoordinator` lives in `preview-application`. Fuzzing a copy, or
a domain-only slice, would not fuzz the code the server runs.

ADR 0063 also left open how `fuzz/Cargo.lock` is audited. The root `cargo deny` run never sees
that graph, and the graph is not just a subset of the root one: `libfuzzer-sys` is licensed
`(MIT OR Apache-2.0) AND NCSA` (it bundles libFuzzer's C++ sources), and NCSA is not in the
root licence list.

## Decision

- The fuzz crate may depend on `preview-domain`, `preview-application` and
  `preview-adapter-http` (`[dependencies.normal] preview-fuzz` in `architecture.toml`), and on
  `lol_html` with no features, so the rewriter target can re-parse the rewriter's output with
  its own code rather than trust the rewriter's internal check. No visibility is widened: the
  rewriter (`preview_adapter_http::inject::{rewrite, Rewrite, ScriptTag, CLIENT_SCRIPT_PATH}`),
  the coordinator and the Host and Origin policies were already `pub`. The fuzz crate still
  reaches no composition, testkit, conformance or filesystem adapter crate.
- `fuzz/Cargo.lock` is audited on every `cargo xtask security` and `cargo xtask check` by a
  fifth stage, `deny-fuzz`: `cargo deny --manifest-path fuzz/Cargo.toml --config fuzz/deny.toml
  --locked check`. It runs where `deny` runs (pull requests through `security.yml`, the weekly
  schedule, the local gate) rather than only in the nightly fuzz workflow, so a change to the
  fuzz graph is audited before it merges.
- `fuzz/deny.toml` is the root `deny.toml` plus one licence exception, NCSA for
  `libfuzzer-sys` alone. A xtask unit test removes that exception and requires the rest to
  equal the root policy, so the two cannot drift. The root policy is not loosened, and no
  production crate can reach NCSA code.
- `.github/workflows/fuzz.yml` runs every target nightly and on demand, each for a bounded time
  (600 s by default, at most 3600 s), keeps the generated corpus between runs in the Actions
  cache, and uploads a crash input as an artifact. A xtask unit test requires its target matrix
  to equal `fuzz/fuzz_targets/`, and another requires a `[[bin]]` per target.

## Consequences

- A rewriter, application or Host-policy change is fuzzed against the real code; the fuzz build
  compiles the HTTP adapter (`axum`, `tokio`) with the nightly, which costs build time in the
  scheduled lane only.
- The local gate needs the fuzz crate's dependencies in the cargo registry for `deny-fuzz`
  (cargo-deny reads `cargo metadata`); with the lockfile committed, no network access beyond
  the registry fetch is needed.
- A new fuzz target is a file in `fuzz/fuzz_targets/`, a `[[bin]]` entry, a seed directory and
  an entry in the `fuzz.yml` matrix; the unit tests fail on any one missing.
