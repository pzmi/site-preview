# Architecture decision records

This directory holds the decisions behind site-preview, one file per decision, in Michael Nygard's lightweight ADR format: `# NNNN. Title`, a date, then **Status**, **Context**, **Decision** and **Consequences**, each record about 30 lines or fewer. To add a record, copy [`template.md`](template.md), use the next free number, and add a row here in the same commit. A decision is never edited away: a reversal is a new ADR, and the old record's status becomes `Superseded by NNNN`. **Accepted** records are owner decisions. **Proposed** records are planner defaults that the owner may override. `doc/tasks.md` cites decision D*n* as ADR *n*+1.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-multi-crate-workspace.md) | Multi-crate workspace supersedes the single-crate layout | Accepted |
| [0003](0003-linux-and-macos-only.md) | P0 platforms are Linux and macOS only | Accepted |
| [0004](0004-rustix-openat-capability.md) | Confined file access via rustix openat capability handle | Accepted |
| [0005](0005-proportionate-foundation-and-early-vertical-slice.md) | Proportionate foundation with an early vertical slice | Accepted |
| [0006](0006-bootstrap-integration-and-dco.md) | Bootstrap integration: one DCO-signed commit per task | Accepted |
| [0007](0007-cargo-metadata-and-publishability.md) | Cargo metadata and publishability | Accepted |
| [0008](0008-pinned-stable-toolchain.md) | Pinned stable toolchain, edition 2024, committed lockfile | Accepted |
| [0009](0009-agent-sandbox-and-pinned-actions.md) | Agent sandbox assumptions and SHA-pinned actions | Accepted |
| [0010](0010-pinned-dev-tools-fail-closed.md) | Pinned dev tools, fail closed when missing | Accepted |
| [0011](0011-doc-directory.md) | Use doc/ instead of docs/ | Proposed |
| [0012](0012-security-policies-are-pure-domain-code.md) | Security and change policies are pure domain code | Proposed |
| [0013](0013-per-crate-external-dependency-allowlist.md) | Per-crate external dependency allowlist | Proposed |
| [0014](0014-formal-models-retrofit-then-model-first.md) | Formal models retrofitted in P0, model-first afterwards | Proposed |
| [0015](0015-semantic-checks-via-syn.md) | Semantic architecture checks via a qualified tool or syn | Proposed |
| [0016](0016-test-locations.md) | Where each test level lives | Proposed |
| [0017](0017-reload-conformance-scope.md) | Scope of reload-model conformance | Proposed |
| [0018](0018-coverage-gates.md) | Coverage gates and changed-line computation | Proposed |
| [0019](0019-check-composes-registered-stages.md) | `check` composes registered stages | Proposed |
| [0020](0020-machine-interface-events-and-warnings.md) | JSONL events and warning format | Proposed |
| [0021](0021-host-authority-policy.md) | Allowed Host authorities | Proposed |
| [0022](0022-html-encoding-for-injection.md) | HTML charset labeling and injection eligibility | Proposed |
| [0023](0023-debounce-bounds.md) | Bounds for --debounce | Proposed |
| [0024](0024-p0-supply-chain-and-release-evidence.md) | P0 supply-chain controls and release evidence | Proposed |
| [0025](0025-crates-io-publishing-deferred.md) | crates.io publishing deferred | Proposed |
| [0026](0026-nightly-fuzz-miri-and-concurrency-proof-scope.md) | Nightly scope, fuzz crate placement, concurrency proofs | Proposed |
| [0027](0027-pr-flow-start.md) | When the PR flow starts | Proposed |
| [0028](0028-composition-owns-runtime-and-processes.md) | Composition owns runtime, signals and process spawning | Proposed |
| [0029](0029-performance-targets-measured-not-gating.md) | Performance targets are measured, not gating | Proposed |
| [0030](0030-css-js-asset-hot-reload-in-p0.md) | CSS, JS and asset hot reload in P0 | Accepted |
| [0031](0031-hot-reload-protocol-details.md) | Hot-reload protocol details | Proposed |
