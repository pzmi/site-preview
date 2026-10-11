# Architecture rules

The catalog of every rule the architecture checker enforces, with how it is enforced, the
negative fixture that proves it, and the ADR behind it. The source of truth is the `rules!`
table in `tools/xtask/src/architecture/rules.rs`; `cargo xtask list-checks` prints the same
ids as rows of the `architecture` stage. [`architecture.md`](architecture.md) explains the
structure these rules protect.

Run the checker with `cargo xtask architecture`. It checks the repository, then runs every
negative fixture under `tests/architecture-invalid/` (each must fail with the rule its
`expected.txt` names), every positive fixture under `tests/architecture-valid/` (each must
produce no diagnostic), and the catalog's completeness (every rule has fixtures, every
fixture belongs to a rule). A rule without a verified negative case is rejected
(foundation.md section 3.4).

## Enforcement kinds

| Kind | What enforces it | Where it lives |
|:--|:--|:--|
| compiler | rustc and clippy through `[workspace.lints]` (`unsafe_code = "forbid"`, `unused_must_use`, `missing_docs`, `clippy::all`, `dbg_macro`, `todo`, `unimplemented`), run by `cargo xtask quick` | `Cargo.toml` |
| xtask graph | the checker reads `cargo metadata --locked` and the manifests and compares them with `architecture.toml` | `tools/xtask/src/architecture/` (`policy.rs`, `graph.rs`, `external.rs`, `inventory.rs`) |
| xtask semantic | the checker parses sources with `syn` and resolves names (ADR 0035) | `tools/xtask/src/architecture/semantic/` |
| trybuild | compile-fail API fixtures that pin private fields and consuming receivers (ARCH-08); the `.stderr` snapshots are the expected diagnostics | `crates/preview-domain/tests/compile-fail/`, `crates/preview-adapter-fs/tests/compile-fail/` |

Every rule below is enforced by the xtask checker. The compiler lints are a second line for
`unsafe`, `todo!`, `unimplemented!` and `dbg!`: the checker still reports them, so the rule
holds without a compiler run and cannot be relaxed from a manifest (see `ARCH-lint-config`).

## Catalog

Columns: rule id as printed in a diagnostic, the statement, the enforcement kind, the task
that introduced it, its negative fixtures (directories under `tests/architecture-invalid/`)
and the ADRs that explain it.

| Rule id | Statement | Enforcement | Task | Negative fixtures | ADR |
|:--|:--|:--|:--|:--|:--|
| `ARCH-policy-schema` | architecture.toml is missing, malformed, has an unknown field or another schema version. | xtask graph | ARCH-01 | [`policy-schema-version`](../tests/architecture-invalid/policy-schema-version), [`policy-unknown-field`](../tests/architecture-invalid/policy-unknown-field) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-metadata` | cargo metadata or a crate manifest cannot be read, including a cyclic workspace. | xtask graph | ARCH-01 | [`normal-cycle`](../tests/architecture-invalid/normal-cycle), [`unreadable-member-manifest`](../tests/architecture-invalid/unreadable-member-manifest) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-missing-member` | A [workspace] members entry has no Cargo.toml on disk. | xtask graph | ARCH-01 | [`missing-member`](../tests/architecture-invalid/missing-member) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-not-a-member` | A crate under crates/ or tools/ is not a workspace member. | xtask graph | ARCH-01 | [`not-a-member`](../tests/architecture-invalid/not-a-member) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-unknown-crate` | A workspace crate is in no policy category. | xtask graph | ARCH-01 | [`unknown-crate`](../tests/architecture-invalid/unknown-crate) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-missing-crate` | The policy names a crate that is not in the workspace. | xtask graph | ARCH-01 | [`missing-crate`](../tests/architecture-invalid/missing-crate) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-empty-selector` | A policy category is present but empty. | xtask graph | ARCH-01 | [`empty-selector`](../tests/architecture-invalid/empty-selector) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-duplicate-category` | A crate is listed in more than one category, or twice in one. | xtask graph | ARCH-01 | [`duplicate-category`](../tests/architecture-invalid/duplicate-category) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-forbidden-edge` | A normal workspace edge is not in [dependencies.normal]. | xtask graph | ARCH-02 | [`cli-bypasses-composition`](../tests/architecture-invalid/cli-bypasses-composition), [`reversed-edge`](../tests/architecture-invalid/reversed-edge) | [0002](adr/0002-multi-crate-workspace.md) |
| `ARCH-undeclared-build-dep` | A build edge to a workspace crate is not in [dependencies.build]. | xtask graph | ARCH-02 | [`undeclared-build-dep`](../tests/architecture-invalid/undeclared-build-dep) | [0002](adr/0002-multi-crate-workspace.md) |
| `ARCH-forbidden-dev-edge` | A dev edge targets a crate that is neither test_only nor in the normal allowlist. | xtask graph | ARCH-02 | [`forbidden-dev-edge`](../tests/architecture-invalid/forbidden-dev-edge) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-production-closure` | A production crate reaches a test_only or tool_only crate over normal or build edges. | xtask graph | ARCH-02 | [`testkit-in-production`](../tests/architecture-invalid/testkit-in-production), [`transitive-test-only`](../tests/architecture-invalid/transitive-test-only) | [0002](adr/0002-multi-crate-workspace.md) |
| `ARCH-policy-cycle` | The [dependencies] allowlist contains a cycle. | xtask graph | ARCH-02 | [`policy-cycle`](../tests/architecture-invalid/policy-cycle) | [0002](adr/0002-multi-crate-workspace.md) |
| `ARCH-unlisted-external` | A direct third-party dependency is not in [external.<kind>] for its crate. | xtask graph | ARCH-03 | [`domain-uses-tokio`](../tests/architecture-invalid/domain-uses-tokio), [`target-specific-external`](../tests/architecture-invalid/target-specific-external), [`unlisted-build-external`](../tests/architecture-invalid/unlisted-build-external), [`unlisted-external`](../tests/architecture-invalid/unlisted-external) | [0013](adr/0013-per-crate-external-dependency-allowlist.md), [ 0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-external-feature` | A third-party edge enables a feature outside its [external.<kind>] list. | xtask graph | ARCH-03 | [`application-tokio-rt-feature`](../tests/architecture-invalid/application-tokio-rt-feature) | [0013](adr/0013-per-crate-external-dependency-allowlist.md), [ 0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-external-source` | A package is not from crates.io and has no [exceptions.sources] entry citing an existing ADR. | xtask graph | ARCH-03 | [`git-dependency`](../tests/architecture-invalid/git-dependency), [`path-outside-workspace`](../tests/architecture-invalid/path-outside-workspace), [`source-exception-missing-adr`](../tests/architecture-invalid/source-exception-missing-adr) | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) |
| `ARCH-source-parse` | A reachable source file cannot be analysed: it does not parse, a module file is missing, or an include!/#[path] is not a literal. | xtask semantic | ARCH-06 | [`include-non-literal`](../tests/architecture-invalid/include-non-literal), [`unparseable-source`](../tests/architecture-invalid/unparseable-source) | [0035](adr/0035-semantic-rules-scope-and-resolution.md) |
| `ARCH-domain-purity` | preview-domain production code names I/O, process, env, thread, clock, randomness, a runtime, HTTP or an outer layer. | xtask semantic | ARCH-06 | [`cfg-attr-path-default`](../tests/architecture-invalid/cfg-attr-path-default), [`domain-aliased-fs`](../tests/architecture-invalid/domain-aliased-fs), [`domain-fs-via-macro-head`](../tests/architecture-invalid/domain-fs-via-macro-head), [`domain-std-fs`](../tests/architecture-invalid/domain-std-fs), [`domain-std-net`](../tests/architecture-invalid/domain-std-net), [`domain-tokio-via-macro-path`](../tests/architecture-invalid/domain-tokio-via-macro-path) | [0012](adr/0012-security-policies-are-pure-domain-code.md), [ 0035](adr/0035-semantic-rules-scope-and-resolution.md) |
| `ARCH-fs-authority` | Production code outside preview-adapter-fs and preview-composition/src/bootstrap.rs names filesystem authority. | xtask semantic | ARCH-06 | [`ambient-fs-in-application`](../tests/architecture-invalid/ambient-fs-in-application) | [0004](adr/0004-rustix-openat-capability.md), [ 0035](adr/0035-semantic-rules-scope-and-resolution.md) |
| `ARCH-layer-isolation` | An adapter names a sibling adapter, composition or watcher, or the application names adapter technology. | xtask semantic | ARCH-06 | [`composition-names-fs-adapter`](../tests/architecture-invalid/composition-names-fs-adapter), [`http-names-fs-adapter`](../tests/architecture-invalid/http-names-fs-adapter) | [0028](adr/0028-composition-owns-runtime-and-processes.md), [ 0035](adr/0035-semantic-rules-scope-and-resolution.md) |
| `ARCH-unsafe-code` | Owned code contains an unsafe block, fn, impl, trait, extern block or unsafe attribute. | xtask semantic | ARCH-06 | [`unsafe-block`](../tests/architecture-invalid/unsafe-block) | [0035](adr/0035-semantic-rules-scope-and-resolution.md) |
| `ARCH-missing-selector` | A crate or file the semantic rules name is missing and not listed in [semantic] absent, absent lists a member, production composition lacks src/bootstrap.rs, or production application lacks its commands or queries module or impl ReloadState. | xtask semantic | ARCH-07 | [`missing-bootstrap`](../tests/architecture-invalid/missing-bootstrap), [`missing-query-selector`](../tests/architecture-invalid/missing-query-selector), [`missing-selector`](../tests/architecture-invalid/missing-selector) | [0035](adr/0035-semantic-rules-scope-and-resolution.md), [0042](adr/0042-command-query-and-observability-rules.md) |
| `ARCH-lint-config` | A lint is relaxed outside source: a member's [lints] is not workspace = true, a workspace lint is allowed, rustflags allow or cap lints, or clippy.toml allows. | xtask graph | ARCH-07 | [`clippy-toml-allow`](../tests/architecture-invalid/clippy-toml-allow), [`member-lints-override`](../tests/architecture-invalid/member-lints-override), [`rustflags-allow`](../tests/architecture-invalid/rustflags-allow) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-unfinished-code` | Production code calls todo!, unimplemented! or dbg! without an [exceptions.lints] entry. | xtask semantic | ARCH-07 | [`todo-in-production`](../tests/architecture-invalid/todo-in-production) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-ignored-test` | Code carries an #[ignore] attribute. | xtask semantic | ARCH-07 | [`ignored-test`](../tests/architecture-invalid/ignored-test) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-lint-suppression` | An allow, expect or coverage(off) attribute lacks an [exceptions.lints] entry citing an existing ADR, or an entry matches nothing. | xtask semantic | ARCH-07 | [`allow-with-missing-adr`](../tests/architecture-invalid/allow-with-missing-adr), [`stale-lint-exception`](../tests/architecture-invalid/stale-lint-exception), [`unjustified-allow`](../tests/architecture-invalid/unjustified-allow) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-test-sleep` | Test code sleeps outside the bounded poll helper preview-testkit/src/poll.rs. | xtask semantic | ARCH-07 | [`sleep-in-test`](../tests/architecture-invalid/sleep-in-test) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-test-name` | A test function's name does not match should_<outcome>_when_<condition>, or a macro hides a test. | xtask semantic | ARCH-07 | [`badly-named-proptest`](../tests/architecture-invalid/badly-named-proptest), [`badly-named-test`](../tests/architecture-invalid/badly-named-test) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-test-structure` | A test function's body lacks the // given, // when and // then comments in that order. | xtask semantic | ARCH-07 | [`missing-given-when-then`](../tests/architecture-invalid/missing-given-when-then) | [0036](adr/0036-hygiene-and-test-quality-rules.md) |
| `ARCH-command-isolation` | preview-application command code names crate::queries or an HTTP type (axum, http, hyper, tower, preview_adapter_http), also through re-exports. | xtask semantic | ARCH-09 | [`command-imports-query`](../tests/architecture-invalid/command-imports-query), [`command-names-http-dto`](../tests/architecture-invalid/command-names-http-dto) | [0042](adr/0042-command-query-and-observability-rules.md) |
| `ARCH-query-purity` | preview-application query code names crate::commands, takes &mut self or &mut of domain/application state, or calls a ReloadState method outside its read allowlist. | xtask semantic | ARCH-09 | [`query-calls-command`](../tests/architecture-invalid/query-calls-command), [`query-mutates-state`](../tests/architecture-invalid/query-mutates-state) | [0042](adr/0042-command-query-and-observability-rules.md) |
| `ARCH-observability-isolation` | The domain names an instrumentation crate, or a tracing/log/observability/metrics value reaches a branch condition or returned value in domain or application code. | xtask semantic | ARCH-09 | [`instrumentation-drives-decision`](../tests/architecture-invalid/instrumentation-drives-decision) | [0042](adr/0042-command-query-and-observability-rules.md) |

## What the checker does not see

These blind spots are recorded decisions, not oversights. None of them is read as a pass
where a rule could be evaded by it; each is covered by another mechanism or is a known limit.

- **Generated code.** Output of procedural macros (derives, attribute macros) is not
  analysed. Build-script output reached through a non-literal `include!` fails as
  `ARCH-source-parse`. A workspace proc-macro crate needs its own review or a new rule
  (ADR 0035).
- **Method calls on values.** Calls such as `path.exists()` or `instant.elapsed()` are not
  type-resolved. The rules catch the import or the type-qualified call, not data flow;
  behaviour tests are the second line (ADR 0035, foundation.md section 3.3).
- **Re-exports across crates.** Composition could re-export a filesystem API that the CLI
  then names. The isolation rules close that route structurally: composition names the
  filesystem adapter only in `src/bootstrap.rs`, and the CLI names only composition
  (ADR 0035).
- **Ambiguous names.** Over-approximation can report a name that is not really forbidden.
  The fix is to name the item plainly, never to loosen the rule (ADR 0035).
- **Unreached files.** A source file no Cargo target reaches through `mod`, `#[path]` or a
  literal `include!` is not compiled and is not read (ADR 0035).
- **Unlisted test macros.** Only `#[test]`, `#[tokio::test]` and `proptest!` are known
  test forms. A test written inside any other macro fails `ARCH-test-name` because it cannot
  be checked; another attribute ending in `test` does not make test code (ADR 0036).
- **Data flow for commands, queries and instrumentation.** `ARCH-query-purity` matches
  `ReloadState` method calls by name, not by receiver type, and does not follow a state
  handle a query passes to a helper outside `queries`. `ARCH-observability-isolation` follows
  instrumentation values only through `let` bindings and assignments inside one function,
  into branch conditions, `match` scrutinees and guards, `let .. else` and returned values;
  a value passed to another function, stored in a field or used inside a `macro_rules!`
  body is not followed. Behaviour tests and review are the second line (ADR 0042).
- **Platforms.** Dependency edges are evaluated for `x86_64-unknown-linux-gnu`,
  `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`
  only (ADR 0033, ADR 0003).
- **`cargo-bylaw`.** It was evaluated and rejected because a selector that matches nothing
  and an unparseable file both passed (ADR 0034).

Every `cfg` branch is analysed and none is evaluated, which is stricter than a per-target
check (ADR 0035).

## The policy file

`architecture.toml` at the repository root is parsed strictly: an unknown field or another
`schema` fails `ARCH-policy-schema`.

| Table | Meaning |
|:--|:--|
| `schema` | The schema version, currently `1`. |
| `[workspace]` | The categories `production`, `test_only` and `tool_only`, each a list of crate names. Every workspace crate is in exactly one; a present list is not empty. |
| `[dependencies.normal]` | Per crate, the workspace crates it may depend on over normal edges. Must be acyclic. A dev edge may also target a `test_only` crate. |
| `[dependencies.build]` | The same for build edges; none are allowed unless listed. |
| `[external.normal]`, `[external.dev]`, `[external.build]` | Per workspace crate, a table of third-party crates and the features each may enable: a list of feature names (`"default"` stands for default features) or `"*"`. A crate allowed under `normal` is also allowed as a dev-dependency (ADR 0033). |
| `[exceptions.sources]` | Maps a non-crates.io source, without query or revision, to `{ adr = "NNNN" }`. |
| `[[exceptions.lints]]` | `{ path, lint, adr }`: approves one lint suppression in one workspace-relative file (ADR 0036). |
| `[semantic]` | Optional. `absent` lists workspace crates that rules name but a fixture workspace does not contain. The real policy has no `[semantic]` table. |

## Adding a rule

1. Add a row to the `rules!` table in `tools/xtask/src/architecture/rules.rs`: a `Rule`
   variant, the id (`ARCH-<kebab-name>`), the owning task, a one-sentence description that
   ends with a period, and the names of at least one negative fixture.
2. Implement the check in the graph code or the `semantic` collector and report it with the
   new rule. Reuse the collector of ADR 0035; do not add a text search.
3. Add each fixture as a directory under `tests/architecture-invalid/<name>/`: a mini
   workspace with its own `Cargo.toml`, `architecture.toml`, `crates/`, a committed
   `Cargo.lock` (a third-party crate is stubbed as a local vendored source so it resolves
   offline), and an `expected.txt` of `key: value` lines:
   - `rule:` the rule id (required)
   - `at:` a substring of the diagnostic's path (required)
   - `contains:` a substring of the message (any number)
   - `only: true` (optional) requires every diagnostic to be of that rule; without it other
     diagnostics may also appear.
4. If the rule could be over-restrictive, add or extend a positive fixture under
   `tests/architecture-valid/`.
5. Add the catalog row to this file with the fixture links and the ADR. A new or reversed
   decision gets an ADR in the same commit.
6. Run `cargo xtask architecture`. The completeness check fails if a rule has no fixture,
   a fixture is missing or names another rule, or a fixture is in no rule. Run
   `cargo xtask list-checks` to see the new id.

## Requesting an exception

An exception is a departure approved by an ADR, never a loosened rule. Open or find the
ADR first (next free number, listed in `doc/adr/README.md`), then add the entry in the same
commit:

- a lint suppression (`#[allow]`, `#[expect]`, `#[coverage(off)]`, a relaxed workspace
  lint, or an allowed `todo!`/`unimplemented!`/`dbg!`): an `[[exceptions.lints]]` entry with
  `path`, `lint` as written in the source and `adr`. The entry must match the code; an
  entry that matches nothing fails, so removing the suppression removes the entry.
- a non-crates.io package source: an `[exceptions.sources]` entry citing the ADR.
- `#[ignore]` and a path dependency outside the workspace have no exception.

A failure of the architecture stage is fixed by fixing the structure, not by editing
`architecture.toml` (unless the task says so, with an ADR).
