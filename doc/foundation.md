# Rust Engineering Foundation — Architecture, Quality, Formal Conformance, and Autonomous Delivery

**Status:** Proposed implementation specification  
**Date:** 2026-10-10  
**Initial application:** Zero-configuration static-site preview / live-reload server (working name: `site-preview`)  
**Applicability:** Reusable foundation for future Rust applications  

## 0. Executive decision

Establish a **multi-crate Cargo workspace with Testable Modular Architecture (TMA), inward-only dependency rules, domain-focused modeling, hexagonal ports/adapters, explicit command/query separation, typed domain events, a composition root, first-class testkit, architecture-policy self-tests, and Quint-based implementation conformance tests** before implementing substantial features.

Make all relevant constraints executable and fail-closed. The compiler should enforce boundaries that the compiler can enforce; a version-controlled dependency policy and semantic checks should enforce the rest; deliberately invalid fixtures must prove the architecture checker works. The test suite must verify behavior at multiple levels, with formal models, model-generated traces, genuine integration tests, mutation analysis, fuzzing, and targeted concurrency/model checking.

**Do not confuse strong architecture with compulsory infrastructure.** Define domain events and deterministic state transitions immediately; add durable event sourcing only when a bounded context actually needs authoritative event history, replay, auditability, or reconstruction. Separate commands and queries immediately; add a generic command bus, durable projections, or messaging only when corresponding use cases exist. These choices must be documented by an ADR, not silently dictated by framework convention.

**Non-negotiable goals:**

1. New features cannot bypass architectural boundaries unnoticed.
2. A deliberately invalid change is demonstrably rejected by each enforcement layer.
3. All tests are reproducible and provide enough evidence for a coding agent to fix a failure without guesswork.
4. The application is entirely Rust at runtime, with no Node/npm/Python requirement for an installed binary. Development-only tools (notably Quint) can use Node.
5. One local command and CI invoke the same quality gates; partial or skipped checks never masquerade as success.
6. Model behavior and implementation behavior are continuously checked for conformance, not merely reviewed separately.
7. The published static files remain unchanged; development-time injection is performed only in served responses.

## 1. Scope and explicit decisions

This plan specifies the **engineering foundation** and its first concrete validation in a local live-reload HTTP server. Product features such as a UI, cloud publishing, authentication, persistence, multi-user collaboration, and asset processing are not assumed to exist. Architectural extension points should accommodate growth without forcing those capabilities into v1.

| Concern | Initial decision | Evolution trigger |
|---|---|---|
| Code structure | Workspace with real crate boundaries | Add bounded-context crates when domain ownership diverges |
| Domain | Pure deterministic policy/state machine | Split domain crate if contexts become independent |
| Application | Explicit commands, queries and ports | Introduce dispatch/bus if routing or middleware becomes substantive |
| I/O | Technology adapters | Add new adapter behind existing port or add a new port |
| Events | Typed in-memory domain events | Persist only if histories are business truth |
| CQRS | Separate write and read contracts | Separate read store/projections only if justified |
| Persistence | None for ephemeral preview state | ADR and storage contracts if durable state appears |
| Testing | Multi-level + conformance + mutation + fuzz | Additional proof methods as concurrency/complexity grows |
| Formal methods | Quint models and ITF trace replay | Kani for bounded local algorithms; Loom/Shuttle for concurrent logic |
| Execution | One binary + development-only tooling | Platforms/releases expanded through explicit matrices |

## 2. Workspace architecture

Use explicit workspace members, not an unreviewed wildcard that silently adopts newly created crates. A new crate must be added to the workspace, architecture inventory, and dependency policy in the same change.

```text
site-preview/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── deny.toml
├── architecture.toml              # Our versioned policy schema
├── .cargo/config.toml
├── .config/nextest.toml
├── .github/workflows/
│   ├── ci.yml
│   ├── formal.yml
│   ├── mutation.yml
│   ├── fuzz.yml
│   ├── security.yml
│   └── release.yml
├── crates/
│   ├── preview-domain/             # Pure policies, transitions, invariants
│   ├── preview-application/        # Commands/queries/use cases/ports
│   ├── preview-adapter-fs/         # Capability-scoped filesystem + notify
│   ├── preview-adapter-http/       # HTTP handlers and live-reload transports
│   ├── preview-composition/        # Only production composition/wiring root
│   ├── preview-cli/                # Binary, CLI lifecycle and exit codes
│   ├── preview-testkit/            # Reusable handwritten fakes (test only)
│   └── preview-conformance/        # Quint-to-real-code driver (test only)
├── tools/xtask/                     # Rust automation and architecture checker
├── tests/
│   ├── architecture-invalid/       # Invalid workspace/source fixtures
│   ├── e2e/                        # CLI / HTTP / browser black-box suite
│   └── fixtures/
├── formal/
│   ├── package.json
│   ├── package-lock.json
│   ├── lifecycle.qnt
│   ├── watcher.qnt
│   ├── reload.qnt
│   ├── scenarios.qnt
│   └── traces/
│       ├── regression/             # Reviewed, committed ITF fixtures
│       └── generated/              # Ignored, reproducibly regenerated
├── fuzz/
│   └── fuzz_targets/
├── docs/
│   ├── architecture.md
│   ├── architecture-rules.md
│   ├── formal-models.md
│   ├── testing.md
│   ├── security.md
│   ├── sdlc.md
│   ├── runbook.md
│   └── adr/
├── CHANGELOG.md
├── CONTRIBUTING.md
└── CLAUDE.md
```

### 2.1 Dependency law

Production dependency direction:

```text
preview-cli ──► preview-composition
                     ├──► preview-adapter-fs ────► preview-application ──► preview-domain
                     ├──► preview-adapter-http ──► preview-application
                     └──► preview-application
```

Adapters may depend directly on `preview-domain` only when needed for domain types; direct application dependencies are preferred for business operations. The CLI does not bypass composition to name concrete adapters. Production crates cannot depend on testkit or conformance crates. `xtask` is a tooling-only crate. `preview-conformance` depends on the application, domain, testkit and whichever concrete adapters are necessary to test real behavior, but production crates never depend on it.

**Dependency graphs represent source-code dependencies, not event/data flow.** Incoming events and outgoing responses can flow in the opposite direction without violating the dependency law.

### 2.2 Ownership within each crate

- **Domain:** value objects, validated identifiers, immutable inputs, policy decisions, typed domain events, transition logic, invariants and refusal reasons. No disk, sockets, clocks, randomness, spawned processes, HTTP, runtime-specific scheduling or environment reads. Use explicit inputs for time and nondeterministic choices. Avoid `serde` as part of the core domain API unless a clear ADR establishes it.
- **Application:** use cases, command/query types, port traits, use-case orchestration, deterministic orchestration policy. Depends only on domain and minimal approved foundational crates. `Result` types are domain/application oriented; no `axum::Response` or `notify::Event` escapes an adapter.
- **FS adapter:** concrete root-directory capability, byte reading, metadata, watcher implementation and OS-specific translations. Only this (plus authorized composition initialization) may obtain ambient filesystem authority.
- **HTTP adapter:** route construction, handlers, representation negotiation, HTTP response injection, SSE or WebSocket transport as selected. Converts HTTP inputs into application commands/queries. Contains no domain policy.
- **Composition:** constructs dependencies, selects adapters/configuration, wires tasks and cancellation, owns runtime resources. Never used as the domain's service locator.
- **CLI:** parses flags/config, returns explicit exit codes, chooses loopback binding defaults and clean shutdown; delegates behavior through composition.
- **Testkit:** fixed clocks, controlled event sources, scripted watcher streams, in-memory content stores, recording notification sinks, fake schedulers and deterministic ID generators. Fakes should implement the same contracts as real adapters; prefer Chicago-style real behavior over generic mocks.
- **Conformance:** interprets Quint stimuli, drives real application/adapter operations, extracts canonical observable state and compares it to model expectations.

### 2.3 Testable, but not artificially abstract

Define traits for nondeterministic or replaceable boundaries: filesystem capability, change-event source, clock/scheduler (if relevant), notification publication, perhaps resource lifecycle. Do not introduce a trait merely to wrap a pure calculation or one stable data structure. Prefer generics where callers know the implementation at compile time; use boxed trait objects only for genuinely runtime-selected dependencies. Keep asynchronous interfaces object-safe where dynamic dispatch is needed, with explicit ownership/cancellation contracts.

### 2.4 CQRS and event responsibility

Maintain `commands`, `queries`, `ports` and `events` as distinguishable application concepts. Commands produce outcomes/events/refusals; queries observe state without changing domain decisions. Avoid requiring a generic command bus before multiple dispatch consumers exist. Domain events are distinct from OS watcher events and distinct from HTTP notifications. The watcher is an input adapter; the server-sent update stream is an output mechanism.

### 2.5 Event sourcing criteria

Do **not** persist every filesystem change into a journal just to claim ES. Use pure state transitions and typed events now; if a later bounded context requires durable history, introduce append-only storage through a dedicated port with versioned events, event IDs, expected revisions, idempotency, atomic append, deterministic replay, snapshot strategy, schema upcasting, read-model rebuilds, and explicit release compatibility invariants. Adopt a per-context ADR and a new Quint model before enabling persisted event sourcing. Never let event-store DTOs infect domain APIs.

## 3. Architecture enforcement — a mandatory three-layer system

### 3.1 Compile-time layer

Cargo workspaces enforce crate ownership, acyclic normal dependencies and Rust visibility; trait bounds enforce interface adherence. Package boundaries are the primary architecture tool. Domain public APIs expose only deliberately selected types/functions. `pub(crate)` is only local to one crate; avoid giant crates that allow unrelated modules to reach internals. Compile-fail fixtures with `trybuild` assert selected invalid API uses remain impossible.

### 3.2 Versioned dependency policy

Implement `cargo xtask architecture` using `cargo_metadata` over **the full resolved dependency graph** and the workspace manifest inventory. It must:

1. Fail on a newly added workspace crate without a declared policy entry, a policy entry for a deleted crate, a missing expected crate, or an empty selector.
2. Enforce direct and transitive workspace dependency allowlists and prohibit cycles and unexpected cross-context edges.
3. Distinguish normal, build and dev dependency kinds; validate build dependencies explicitly and reject testkit/conformance/fuzz-only packages in production dependency closure.
4. Inspect feature-dependent and target-specific edges, including Linux/macOS/Windows where supported. Do not misinterpret Cargo's all-target resolution as proof each target builds successfully.
5. Reject nonreviewed git dependencies, foreign registries or remote-sourced build scripts except documented approved cases; coordinate supply-chain rules with `cargo-deny`.
6. Report complete dependency paths in failures, rather than merely naming the forbidden endpoints.
7. Use source-tree inventory plus Cargo metadata: the policy must not silently skip crates not covered by selectors.

Example **proposed** policy syntax (implemented by `xtask`, not purported to be native Cargo TOML):

```toml
schema = 1

[workspace]
production = [
  "preview-domain", "preview-application", "preview-adapter-fs",
  "preview-adapter-http", "preview-composition", "preview-cli"
]
test_only = ["preview-testkit", "preview-conformance"]
tool_only = ["xtask"]

[dependencies.normal]
preview-domain = []
preview-application = ["preview-domain"]
preview-adapter-fs = ["preview-application", "preview-domain"]
preview-adapter-http = ["preview-application", "preview-domain"]
preview-composition = [
  "preview-domain", "preview-application",
  "preview-adapter-fs", "preview-adapter-http"
]
preview-cli = ["preview-composition"]
preview-testkit = ["preview-domain", "preview-application"]
preview-conformance = [
  "preview-domain", "preview-application", "preview-testkit",
  "preview-adapter-fs", "preview-adapter-http"
]
xtask = []
```

### 3.3 Semantic/module layer

Trial `cargo-bylaw` for domain/adapters, allowed module dependencies and layer ordering, with a pinned version. It is a promising architecture tool but relatively young; qualify it against actual Rust constructs before treating it as authoritative. In particular check macro expansion, module reexports, `cfg` branches, generated code and trait implementations. Never accept a tool's absence, no matches, parse failures or unsupported language construct as a passing architecture result. If the trial fails, implement narrowly scoped semantic checks using maintained Rust tooling or compile-time crate separation, not brittle text grep.

Core semantic rules:

- Domain may not reference `std::fs`, `std::net`, `std::process`, environment, uncontrolled clock, random generator, Tokio, HTTP, or concrete adapter symbols.
- All ambient filesystem authority originates at the authorized bootstrap boundary and is delegated through a capability handle.
- HTTP route/handler modules may use application interfaces, not construct filesystem/watch adapters.
- Application command modules cannot import query handlers/read-side implementations or HTTP DTOs.
- Query implementations cannot mutate domain state; cannot call command handlers to implement a read.
- Adapters must not refer to sibling adapters or composition.
- Observability/instrumentation cannot change domain decisions.
- `unsafe` is forbidden in owned production code except by a separately approved, isolated crate and ADR (default: none).
- No unapproved lint suppressions, ignored tests, coverage exclusions, process sleeps in deterministic tests or production `todo!()`/`unimplemented!()`.

Do not assume a dependency graph check detects stdlib calls, reexports, or data-flow violations: supplement with targeted analysis and behavior tests.

### 3.4 Self-tests for the policy itself

Create controlled invalid workspaces and code snippets exercising EACH rule: reversed dependency, testkit in normal dependency closure, unknown crate, forbidden filesystem call in domain, direct HTTP-to-FS coupling, incorrectly categorized dependency, and an empty or mismatched selector. Assert the checker exits nonzero, identifies the specific violation and points at the offending path. Include positive fixtures with allowed dependencies so the checker is not merely over-restrictive. Reject a rule without at least one verified negative case. Run this on every PR.

## 4. Rust baseline configuration and reproducibility

- Pin the exact stable Rust toolchain release in `rust-toolchain.toml`, with `rustfmt`, `clippy`, and `llvm-tools-preview` components. Keep nightly tools separately pinned for fuzzing, Miri and any nightly-only coverage/proof features; do not let nightly leak into the stable production build.
- Use Rust edition 2024 and workspace resolver 3 if compatible with the pinned compiler. Commit `Cargo.lock` for the application, use `--locked` in CI/release, and control update PRs separately.
- Set `[workspace.lints]` for basic strict lint policy; inherit through member manifests. Zero Clippy warnings on `--all-targets`, and evaluate relevant feature combinations. Deny `unsafe_code` in owned application crates.
- Pin Cargo subcommands and Node/Quint tools through documented versions/hashes or installer lockfiles. CI must not silently fetch a different binary on reruns. Prefer locked/pinned GitHub Action SHAs.
- Include `.cargo/config.toml` alias for `cargo xtask` and code the `xtask` binary in Rust; one source of truth for check composition, stages, evidence, timeouts and exit codes.
- Maintain architecture and test docs as implementation contracts, not unverified prose. ADRs record why major policies exist; policy changes require corresponding tests.

Illustrative root manifest (tool/dependency versions deliberately not invented):

```toml
[workspace]
resolver = "3"
members = [
  "crates/preview-domain",
  "crates/preview-application",
  "crates/preview-adapter-fs",
  "crates/preview-adapter-http",
  "crates/preview-composition",
  "crates/preview-cli",
  "crates/preview-testkit",
  "crates/preview-conformance",
  "tools/xtask",
]

[workspace.package]
edition = "2024"

[workspace.lints.rust]
unsafe_code = "forbid"
unused_must_use = "deny"

[workspace.lints.clippy]
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
```

Each member crate opts into the workspace lints. The `xtask` crate may need a separately documented tooling exception; avoid weakening production crates to accommodate tooling needs.

## 5. Tests: distinct responsibilities, shared real interfaces

| Level | Execution | Contract |
|---|---|---|
| Unit | `cargo test`, nextest | Domain units and boundary examples |
| Property | `proptest` | Generated values, shrunk counterexamples, persistent seeds |
| Component | nextest | Real application + in-memory adapters; no external services |
| Contract | Shared generic harness | Every fake and production adapter fulfills the port contract |
| Architecture | xtask + Rust tests | All crate/module/policy rules and their negative controls |
| Integration | nextest | Actual tempdir filesystem, watcher, sockets, subprocess as applicable |
| E2E | `assert_cmd` + browser runner | Installed binary and externally observable behavior |
| Compile-fail | `trybuild` | Invalid type/API uses do not compile |
| Conformance | Quint Connect / ITF replay | Model stimuli -> real code -> observable state equivalence |
| Mutation | `cargo-mutants` | Test quality rather than simple execution |
| Fuzz | `cargo-fuzz` | Adversarial parsing and state sequences |
| Concurrency | Loom/Shuttle | Synchronization schedules and ordering/cancellation |
| Formal | Quint/Apalache, targeted Kani | Model invariants, bounded Rust local properties |
| Benchmarks | Criterion/isolated system tests | Performance over controlled, recorded environments |

Test-location conventions should follow Rust idioms: unit tests next to modules, separate workspace crates for shared black-box tests and model conformance. Preserve *semantic* test levels, without artificially copying pytest file layouts. Make level names discoverable to agents via `cargo xtask list-checks`.

### 5.1 Contract tests

For each port that has multiple implementations, write a reusable contract test harness parameterized by a factory. Run it with each in-memory fake **and** each real adapter. Verify refusals, ordering, idempotency, cancellation, error propagation, and resource cleanup. The fake must not have looser semantics than production. Use `tempfile` and dynamically allocated ports instead of shared global paths and fixed ports.

### 5.2 Test quality rules

- No arbitrary sleeps for synchronizing tests. Use explicit events, injected clocks, barriers or readiness conditions. OS watcher integration tests may require bounded condition polling; record a reason and never assume timing itself is a correctness oracle.
- No global monkeypatch-style replacement of production functions. Prefer dependency injection and functional fakes; tightly scoped external-process interception may be allowed when justified.
- Every application service/use case must have a corresponding test entry point. Check this semantically, not only by string matching class names.
- Every regression gets a deterministic fixture/seed. New nondeterministic failures cannot be quietly retried to green: nextest should report flaky reruns as failures when retries are enabled.
- Test helpers must not use unchecked panic/assert behavior in ways that prevent meaningful failure output. Include expected/actual diff and the full operation sequence on state-machine failures.
- Test quality checks must be self-tested using intentionally bad tests; changing conventions requires a policy change and negative fixture.

Property tests keep failing seeds. A crate's inline property tests use proptest's default persistence, which writes `<crate>/proptest-regressions/<module>.txt`; black-box properties in `<crate>/tests/` set `FileFailurePersistence::Direct("proptest-regressions/<file>.txt")` (the default silently persists nothing outside `src/`). The directory is committed and never ignored, so a counterexample replays first on every later run. Case counts come from `PROPTEST_CASES`, set for the `ci` nextest profile in `.config/nextest.toml`, never from code. Domain properties on the public API live in `crates/preview-domain/tests/properties.rs`; those needing private items stay in their module.

### 5.3 Coverage and mutation

Initial proposed **acceptance targets**, to be calibrated from a real baseline:

- 0 architecture violations, 0 Clippy warnings, 0 unexpected skipped gates.
- >=90% overall instrumented line coverage to start; then ratchet upward, with a target of >=95% for core domain/application. A coverage decrease requires explicit human review with justification.
- >=95% changed-line coverage, measured using LLVM coverage Cobertura export and `diff-cover` or a tested equivalent. Full coverage must incorporate all in-process unit/component/integration suites, not just fast tests. Note: LLVM line/region coverage on stable does not provide the exact stable branch-coverage parity of Python's pytest-cov branch mode.
- Target >=90% killed mutation score for critical domain algorithms after equivalent mutations are triaged. Report weekly, eventually gate *new* survivors, not an unexplained historical count.
- Mutation runner selects tests that actually exercise mutated code. Exclude costly browser and E2E suites by design unless a mutation target needs them; don't claim survivors are eliminated if the selected harness cannot execute the path.

## 6. Formal methods and conformance — REQUIRED, not optional

The required mechanism is: Quint models, committed named interleavings, reproducible seeded random traces, replay against the real implementation and real services, comparison after every stimulus, deliberate broken-implementation negative tests and scheduled `quint verify`/Apalache. The Rust foundation must retain **that entire mechanism**, with a Rust implementation of the replay driver.

### 6.1 Quint models for the initial preview server

1. **`lifecycle.qnt`** — States `Stopped`, `Starting`, `Serving`, `Stopping`, `Failed`; actions for start, bind failure, watcher failure, cancellation, stop and restart. Invariants: no responses in stopped state; exactly one active owner of a bound listener; all owned tasks released after stop; shutdown is idempotent; a failed startup does not leave resources allocated.
2. **`watcher.qnt`** — Actions for file creation, modify, rename, delete, atomic-save, ignored temp-file changes, duplicated/reordered notifications, root deletion/recreation and shutdown. Invariants: only eligible changes can trigger reload; no lost eligible changes after coalescing; duplicate events cannot produce unbounded reload storms; a watcher failure transitions through explicit error/recovery handling.
3. **`reload.qnt`** — Browser connects/disconnects/reconnects; file changes occur; server emits versioned notifications. Invariants: monotonic generation/version; a reconnecting browser eventually observes a newer generation if the server and channel remain available; no notification claims a generation that has not been committed; no assumptions of reliable delivery during a disconnected interval. Separate safety properties from fairness/liveness assumptions.

Future models: path-serving authorization and multi-client behavior if the protocol becomes complex; persisted-event compatibility/release model if ES or stored projections are introduced. Do not model every trivial pure function in Quint; use property tests or Kani for local algorithms.

### 6.2 Model-first workflow

For a new concurrency/protocol feature: (a) specify external stimuli and observable state; (b) write the Quint model and safety/liveness invariants; (c) create named interleaving scenarios; (d) run simulation/model checks; (e) implement Rust driver with a **canonical observation adapter**; (f) implement the real feature; (g) keep conformance mandatory in CI. A model change without matching driver/behavior changes must fail.

### 6.3 Conformance driver design

The harness must use model-based testing to drive real Rust code, not reimplement the domain transition algorithm inside tests. For each ITF transition:

1. Decode the externally visible action and selected nondeterministic parameters.
2. Apply that stimulus to a real application use case or concrete adapter under controlled dependencies.
3. Capture outcome/refusal, externally emitted events, relevant state and resources.
4. Project **real observed state** into a canonical model-facing representation. Normalize inherently irrelevant fields (timestamps, OS-specific error strings) explicitly; never normalize away correctness differences.
5. Compare model and implementation outcome/state **after every transition**, not only at the final step.
6. Report model, seed, trace ID, step index, action, expected state, actual state and minimal useful diff.
7. Always run a negative-control test replacing one component with a deliberately faulty implementation and assert the checker detects the precise bad transition.

Use [Quint Connect](https://github.com/quint-co/quint-connect) as the first-choice generator/replay integration (`quint-connect` 0.1.2 observed in October 2026). **Also preserve a separately tested, explicit ITF replay path for committed regression traces.** Do not assume Quint Connect's convenience macros cover all existing hand-written ITF regression fixtures until validated. If necessary use `serde` plus an ITF decoding library or narrow local decoder within the test-only conformance crate. Keep model hooks independent of the chosen harness library.

### 6.4 Trace corpus and gates

- `formal/traces/regression/`: reviewed named sequences including all incidents/counterexamples. Committed. Regenerated scenarios must be byte-for-byte checked (with deterministic seeds/normalization) so stale fixtures fail.
- `formal/traces/generated/`: generated fresh per CI run, ignored by Git; record seed, Quint version, scenario family, trace count and maximum steps.
- PR: Quint typecheck, named scenario tests, bounded random simulation/invariants, regression freshness, replay **both committed and new traces** against real Rust code, negative controls. Start with approximately 30 traces × 25 steps per model as an initial **budget to measure**, not a proven speed guarantee.
- Scheduled: larger trace space, multiple seeds, `quint verify` through pinned Apalache for bounded verification; retain failed ITFs as artifacts and promote useful counterexamples to regression corpus.
- A future production-trace checker can validate captured real traces against the model, but it is a separate capability, not a replacement for model-to-code replay.

Quint's CLI and Apalache run only in the test/build environment. The runtime Rust binary has no Node/JVM requirement.

## 7. Project-specific application guarantees

The preview server has meaningful security and correctness constraints despite being local:

| Invariant | Design/test strategy |
|---|---|
| Serve only inside configured root | Use a directory-capability API such as `cap-std`, not merely string canonicalization; test traversal, encoded paths, symlinks and TOCTOU races on supported OSes |
| Default loopback-only binding | CLI + socket tests; explicit opt-in for wider interfaces, reject unsafe configuration where needed |
| Do not allow unintended DNS-rebinding/cross-origin read access | Validate Host/Origin policies appropriate to the chosen transport; test hostile origins and requests, not only `127.0.0.1` binding |
| No modifications to original static files | Byte-for-byte golden tests before/after serving HTML, CSS, images and JS |
| HTML reload script injection is response-only | HTTP representation tests for HTML vs non-HTML, missing body, encoding, content length, streaming and CSP considerations |
| Reliable local feedback loop | Real watcher integration across create/edit/delete/rename/atomic save, plus browser reload E2E |
| No browser hangs on server restart | Reconnection backoff and generation/resync model, verified by Quint and browser E2E |
| Duplicate change events are harmless | Pure state machine property tests and model conformance |
| No orphaned server/watcher tasks | Cancellation and shutdown integration tests; closed listener and subprocess exit verified |
| Works on supported OSes | Native CI matrix with OS-specific watcher/path behavior |
| No JS runtime needed by the installed binary | Release artifact smoke test with only binary + static fixture |
| Playwright can inspect it | E2E browser driver, debug artifacts, screenshots on failure |

**Filesystem security:** The initially selected root can be opened using explicit ambient authority at composition time. All untrusted subsequent requests should be resolved through a capability-scoped directory handle, even when symlinks race with changes. This is stronger than `canonicalize()+starts_with(root)` and prevents accidentally reopening ambient paths during request handling.

**Protocol design:** Prefer one-way SSE for browser reload notification unless actual bidirectional messaging is necessary. Use explicit monotonic generation IDs and reconnection semantics. The HTTP handler must avoid leaking internal absolute paths, watching outside the root, or changing static assets in-place. Treat development-time HTML injection as an observable, testable response transformation.

## 8. Security and supply chain

Adopt `cargo-deny` with reviewed advisory, license, ban, duplicate-version and registry/source rules; Gitleaks; `actionlint`; CodeQL for Rust where available; and `cargo-vet` after establishing a defensible audit policy. `cargo-machete` can assist with unused dependencies but needs reviewed false-positive exceptions. Build SBOMs, embed auditable dependency metadata if useful, and produce verifiable build provenance/signatures for release assets. Do not call an advisory-free dependency graph a full security guarantee.

Pin GitHub Actions by commit SHA, minimize token permissions, protect secrets, separate PR validation from privileged releases, reject untrusted artifact promotion, and verify that release artifacts correspond to the tested commit and lockfile. For network-connected autonomous agents, runtime permissions/egress policies are a separate boundary and must not be inferred from Rust language safety.

`unsafe` stays forbidden in owned crates. If a third-party crate uses unsafe internally, address that through audit/vetting, not by pretending `unsafe_code = "forbid"` in application crates audits dependencies.

## 9. Command surface and local/CI parity

Provide the commands below as an implemented `xtask` surface. Exact subcommands are part of this **project specification**, not commands assumed to exist in Cargo by default.

```text
cargo xtask fmt            # Apply formatting (developer convenience)
cargo xtask quick          # Formatter, Clippy, fast unit/component
cargo xtask architecture   # Crate/module policy + negative fixtures
cargo xtask test           # All ordinary levels including docs + E2E
cargo xtask conformance    # Quint model/scenarios/traces + Rust replay
cargo xtask coverage       # LLVM coverage + threshold + changed-code gate
cargo xtask security       # deny/secrets/workflows and required audits
cargo xtask check          # The full blocking PR-quality gate
cargo xtask mutation       # Longer source mutation analysis
cargo xtask fuzz           # Bounded fuzz run or reproduction
cargo xtask formal-verify  # Apalache/Kani deep lane (when configured)
cargo xtask release-check  # Clean packaged artifact tests
cargo xtask list-checks    # Machine-readable check catalog, policies, owners
```

`check` must run formatting, lint, architecture + negative controls, unit/component/contract/integration/E2E, Rust doctests (nextest alone does not run doctests), conformance, coverage and security. It must not quietly downgrade a failed test to a warning. `quick` is explicitly not equivalent to `check`. If a local tool is absent, report actionable dependency instructions and a non-success exit code rather than claiming a green gate.

Use nextest groups for genuinely shared resources, prefer dynamically assigned ports over serializing an entire suite, and use process isolation for black-box tests. For CI snapshots, cache compiler/build artifacts keyed by lockfile/toolchain and ensure cache corruption only causes slow builds, never incorrect results.

## 10. CI workflow topology

**PR blocking (Linux):**

- `Architecture and policy`: metadata policy, semantic layer constraints, architecture negative fixtures, manifest/tool pinning, docs-rule consistency.
- `Static quality`: rustfmt, Clippy, `cargo check` targets/features, compile-fail tests.
- `Fast tests`: unit/property/component/contract/composition via nextest.
- `Integration`: real FS and HTTP tests, subprocess lifecycle, no fixed ports, result artifacts.
- `Conformance (Quint)`: typecheck, scenarios/freshness, generated+committed ITF replay against real implementation, negative controls.
- `Coverage`: all relevant in-process test suites, line and changed-line thresholds, machine-readable report.
- `Security`: cargo-deny, secrets, actionlint, CodeQL where available, dependency policy.
- `Artifact smoke`: build release binary, execute outside repo against fixture, verify no Node/npm requirement and no mutated source files.

**Cross-platform required matrix:** macOS + Windows platform-specific integration and binary E2E, alongside Linux. Run all platform-specific code on its native OS; the Linux conformance lane can remain canonical initially unless cross-platform behavior affects model obligations.

**Scheduled:** mutation and score drift; extended Quint/Apalache proof matrix; deeper fuzzing; Miri for applicable units, Kani for selected proof harnesses; Loom/Shuttle for concurrency code; security recertification; longer performance baselines. Time budgets should be measured and ratcheted, not invented upfront.

**Release:** clean checkout with exact toolchain + lockfile; repeat required artifact/acceptance tests; archive test evidence; produce platform binaries, hashes, SBOM/provenance; sign/attest if release distribution needs it; no privileged publishing from untrusted PR contexts.

If a path classifier implements documentation-only optimization, it must be fail-closed, tested against renames/deletions/generated input, and still run documentation integrity, security and architecture-document consistency. Avoid a premature classifier unless CI expense warrants its failure surface.

## 11. Agent execution contract and SDLC

Treat all quality gates as deterministic transitions in the software-development workflow. Copy the *principle* of a stage/gate workflow model, not its full product-specific orchestration into the Rust project.

**Definition of Ready:** issue states user-visible behavior, invariants, affected bounded context/crates, threat model (when inputs/resources change), test levels, model impact, acceptance criteria and anticipated backward-compatibility effects. Short tasks can record these concisely; large tasks get an ADR/design note.

**Agent implementation procedure:**

1. Read `docs/architecture.md`, `docs/testing.md`, relevant ADRs and `CLAUDE.md`.
2. Identify boundaries and update architecture policy/ADR first if a new dependency or capability is needed.
3. For protocol/state/concurrency changes, update Quint model, named scenarios and conformance observation contract first.
4. Write failing unit/property/contract/conformance tests, then implement the smallest valid behavior.
5. Run `cargo xtask quick` continuously, followed by `cargo xtask check` before completion.
6. Save failure evidence; never suppress a failing test, widen lint exemptions, remove a modeled invariant or rewrite a golden fixture merely to obtain green CI.
7. Report exactly which gates ran, versions/seeds, and which (if any) remain unexecuted. No unverified assertion of quality.

**Definition of Done:** code, tests and models agree; architecture rules and negative fixtures pass; build/packaged binary passes; affected documentation and ADRs updated; PR CI required gates green; agent supplies reproducible verification evidence. Manual exception requires explicit owner approval and follow-up issue, with no false green signal.

Keep architecture policy, ADRs and tool versions owned as code. Do not add nonfunctional orchestration machinery or a project board solely to satisfy process conventions.

## 12. Execution plan with concrete acceptance criteria

The phases below are **ordered dependency milestones**, not calendar estimates. Each phase can be independently reviewed and must have functioning tests and demonstrable acceptance evidence. Do not ship a milestone consisting only of scaffolding and promises.

### Phase 0 — Decisions and baseline repository

Deliver: README, intended user contract, root workspace, exact stable Rust pin, locked dependencies, documented runtime/development distinction, architecture ADRs, CI skeleton, xtask command surface.

Accept when: a clean checkout reproducibly builds the empty CLI and invokes `xtask` without an external application runtime; bad or missing toolchain gives a clear error. All declared crates compile. Tool versions are audited and committed.

### Phase 1 — Strong architectural enforcement

Deliver: domain/application/adapter/composition/CLI/testkit/conformance crates; versioned dependency allowlist using `cargo_metadata`; selected semantic module checker; negative architecture fixtures; compile-fail tests for critical API boundaries; architecture rule catalog and explanations.

Accept when: injecting each forbidden edge, stdlib filesystem call into domain, unregistered crate, unexpected build dependency, or testkit production dependency makes the gate fail with the correct diagnostic; restoring code makes it pass. All selectors find their expected targets. Both positive and negative fixtures run in CI.

### Phase 2 — Testing and quality platform

Deliver: nextest configuration, testkit fakes, generic contract harnesses, property-test conventions, coverage reports/ratchet, cargo-deny, Gitleaks/actionlint, protected CI jobs, artifact capture, mutation configuration and baseline report, `quick`/`check` consistency.

Accept when: one deliberately broken fake fails the same adapter contract as a broken real adapter; an untested changed line affects the changed-code gate; a known mutation survives and is visible; a failing test returns nonzero both locally and in CI. The quality gate runs on a clean checkout.

### Phase 3 — Quint models and conformance engine

Deliver: three models (`lifecycle`, `watcher`, `reload`) with explicit external stimuli, invariants and named scenarios; pinned Quint toolchain; Quint Connect evaluation; explicit ITF replay API; canonical state observation; committed regression fixtures; random seeded traces; conformance negative controls; CI lane and failure artifacts.

Accept when: deliberately remove generation monotonicity or duplicate-event suppression from a real Rust test implementation and a replayed Quint scenario fails at a named step with expected/actual state; replay with same seed reproduces the error; committed corpus freshness and generated trace suite both run on every PR. Apalache scheduled lane has a successful baseline bounded verification run or reports a reviewed tooling blocker rather than a fabricated pass.

### Phase 4 — First real vertical slice

Deliver: safe root capability; validated URL-to-relative-path mapping; static HTTP file serving; HTML-response-only reload injection; one actual filesystem watcher; application/domain reload generation state; browser SSE subscription; graceful cancellation; CLI startup and terminal feedback.

Accept when: a fresh directory with `index.html` can be served using one installed binary and a browser automatically refreshes on changes, including atomic save; path escapes and hostile origins are rejected; source files never change; reconnection and shutdown tests pass; the application runs without Node/npm; state transitions remain model-conformant.

### Phase 5 — Adversarial and cross-platform hardening

Deliver: malicious input corpus, fuzz targets, OS-specific symlink/watcher tests, lifecycle and reconnect E2E tests, captured browser artifacts, macOS/Windows integration matrix, test flake isolation, reproducible release builds; targeted Loom/Shuttle/Kani verification for any complexity introduced.

Accept when: supported-platform artifacts pass real filesystem/HTTP tests; clean restart leaves no resource leaks; traversal and symlink race tests pass; fresh fuzz runs have no untriaged crash; verified properties and their scope are explicitly documented. Performance benchmarks have stable measurement procedure and an approved budget before becoming blocking.

### Phase 6 — Release and autonomous development readiness

Deliver: signed/attested reproducible artifacts as needed, SBOM, security reporting, release smoke tests, enforced PR branch rules, agent instructions, documentation consistency checks and version/upgrade policy.

Accept when: a separate fresh machine/CI job can install and run each promised platform artifact; all required checks are green; a coding agent can implement a small feature inside approved dependencies and discover exact remediation instructions for every intentionally broken rule.

## 13. Deliverable inventory / issue breakdown

Create separate tasks with dependencies and clear owners:

- `ARCH-01` Workspace crate topology and root toolchain pin
- `ARCH-02` Architecture policy schema and Cargo graph checker
- `ARCH-03` Semantic module/library restrictions
- `ARCH-04` Architecture negative-fixture test harness
- `ARCH-05` Public API compile-fail fixtures
- `TEST-01` Nextest, deterministic fake/testkit contracts
- `TEST-02` Property tests and regression seed persistence
- `TEST-03` LLVM coverage, changed-code gate and ratchet
- `TEST-04` Mutation testing, baseline and survivor triage
- `FORMAL-01` Lifecycle Quint model and named scenarios
- `FORMAL-02` Watcher Quint model and named scenarios
- `FORMAL-03` Reload/reconnect Quint model and named scenarios
- `FORMAL-04` Quint Connect evaluation and ITF replay driver
- `FORMAL-05` Observation equivalence and negative controls
- `FORMAL-06` Generated/committed trace CI and Apalache job
- `SEC-01` Dependency advisories/licenses/registries policy
- `SEC-02` Secrets, workflow analysis, artifact provenance
- `SITE-01` Directory capability and path security
- `SITE-02` HTTP serving / injection response contract
- `SITE-03` FS watcher and deterministic event normalization
- `SITE-04` Reload delivery, reconnect and cancellation
- `SITE-05` End-to-end/browser/cross-platform regression suite
- `OPS-01` xtask consolidated quality gate
- `OPS-02` CI matrix, caches and failure evidence
- `OPS-03` Release artifact smoke/attestation
- `DOC-01` ADRs, architecture contracts and agent operating guide

Each task should be independently verifiable and should specify whether it changes the model, port contracts, public APIs, dependency graph, or security trust boundary.

## 14. Risks and countermeasures

| Risk | Countermeasure |
|---|---|
| Lots of crates but porous boundaries | Every crate has one reason to change; dependency inventory and compile-fail tests verify isolation |
| Very new architecture tools misparse constructs | Cargo compiler boundaries are primary; independently tested metadata checker; qualify cargo-bylaw before relying on it |
| Model is consistent with itself but not with real implementation | Real-code conformance driver, observation after every step and deliberately broken implementation controls |
| Model overabstracts failure/reconnection behavior | Explicit external stimuli, visible outcomes, stable abstraction contract, named adversarial interleavings |
| State-space explosion | Small focused models, bounded domains, diverse seeds, scheduled deep verification; document proof scope |
| Fake diverges from real adapter | Shared contract suite, real integration E2E and no fake-only conformance claim |
| CI gate becomes too slow or flaky | Layered/parallel jobs, short PR traces plus scheduled deep lanes, no sleeps, isolated resources, budgets measured |
| New dependency exposes supply-chain risk | `cargo-deny`, pinned dependencies, cargo-vet policy, restricted sources, reproducible builds |
| Filesystem race escapes root | Capability-based directory handles and adversarial native tests, not `canonicalize` as the only defense |
| Agent weakens checks to make CI pass | Enforced negative controls, approved exception changes, branch rules and explicit DoD |
| Rust static checks are mistaken for correctness proof | Property, mutation, fuzz, integration and model conformance retained as independent evidence |
| ES/CQRS abstractions become framework-first | Separate command/query contracts and events immediately; persist events or introduce bus only after ADR defines semantics |

## 15. Reference implementations and sources

**Rust and Quint ecosystem:**

- [Cargo workspace reference](https://doc.rust-lang.org/cargo/reference/workspaces.html), [Cargo metadata contract](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html), [Cargo resolver/dependency kinds](https://doc.rust-lang.org/cargo/reference/resolver.html).
- [cargo-bylaw](https://github.com/danielgerlag/cargo-bylaw) (young; evaluate), [Quint Connect](https://github.com/quint-co/quint-connect), [Quint Model-Based Testing](https://quint.sh/docs/model-based-testing), [Quint CLI](https://quint.sh/docs/quint).
- [cargo-nextest](https://nexte.st/), [nextest test groups](https://nexte.st/docs/configuration/test-groups/), [cargo-mutants](https://mutants.rs/), [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov), [proptest](https://github.com/proptest-rs/proptest), [trybuild](https://github.com/dtolnay/trybuild).
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny), [cargo-vet](https://mozilla.github.io/cargo-vet/), [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), [Kani](https://github.com/model-checking/kani), [Loom](https://github.com/tokio-rs/loom), [Shuttle](https://github.com/awslabs/shuttle).
- [cap-std](https://github.com/sunfishcode/cap-std) for capability-scoped filesystem access.

## 16. Final acceptance definition

The foundation is complete when it demonstrates, on a clean checkout, that: the binary builds with a pinned toolchain and locked dependency graph; architectural violations fail and policy negative controls prove it; all test levels and port contracts execute; an actual Rust implementation conforms to reviewed and freshly generated Quint traces step-by-step; known erroneous implementations are rejected; mutations, fuzzing and formal verification have functioning scheduled lanes; unsafe path/network failures are ruled out by adversarial tests; supported release binaries behave as promised with no JS runtime; CI/local full quality gates agree; and autonomous agents are not able to claim completion without verifiable green evidence.

**Guiding principle:** maximize deterministic constraints and proof of compliance, not the number of abstractions. Make architectural decisions difficult to violate, domain decisions easy to replay, and failures cheap to reproduce.
