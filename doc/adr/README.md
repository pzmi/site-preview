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
| [0027](0027-pr-flow-start.md) | When the PR flow starts | Superseded by 0032 |
| [0028](0028-composition-owns-runtime-and-processes.md) | Composition owns runtime, signals and process spawning | Proposed |
| [0029](0029-performance-targets-measured-not-gating.md) | Performance targets are measured, not gating | Proposed |
| [0030](0030-css-js-asset-hot-reload-in-p0.md) | CSS, JS and asset hot reload in P0 | Accepted |
| [0031](0031-hot-reload-protocol-details.md) | Hot-reload protocol details | Proposed |
| [0032](0032-whole-plan-on-main.md) | The whole plan is integrated on main | Accepted |
| [0033](0033-architecture-policy-external-and-dev-dependencies.md) | Architecture policy: external allowlist shape and dev-dependencies | Proposed |
| [0034](0034-cargo-bylaw-qualification.md) | cargo-bylaw is not qualified; semantic rules use syn | Proposed |
| [0035](0035-semantic-rules-scope-and-resolution.md) | Semantic rules: what is analysed and how names resolve | Proposed |
| [0036](0036-hygiene-and-test-quality-rules.md) | Hygiene and test-quality rules and their exceptions | Proposed |
| [0037](0037-host-origin-parsing-and-core-net.md) | Host and Origin parsing details; `core::net` in the domain | Proposed |
| [0038](0038-application-port-shapes.md) | Application port shapes: synchronous content, async change source, no adapter types | Proposed |
| [0039](0039-request-resolution-outcomes.md) | Request resolution outcomes: no existence leak, snapshot before open | Proposed |
| [0040](0040-change-coordinator-recovery-and-exhaustion.md) | Change coordinator: recovery signal, reconcile requests and revision exhaustion | Proposed |
| [0041](0041-workflow-pin-parsing-and-security-stage-scope.md) | Workflow pin parsing, secret-scan preconditions and the unmaintained-crate scope | Proposed |
| [0042](0042-command-query-and-observability-rules.md) | Command, query and observability rules: selectors and syntax-level flow | Proposed |
| [0043](0043-lol-html-rewriter-and-mpl-exceptions.md) | The `lol_html` rewriter: strict parsing, verified output, scoped MPL-2.0 exceptions | Proposed |
| [0044](0044-sse-admission-and-absolute-form-scheme.md) | SSE admission order, `HEAD` on the event stream, and the absolute-form scheme | Proposed |
| [0045](0045-connection-limits-and-bounded-shutdown.md) | Connection limits and a bounded shutdown drain | Proposed |
| [0046](0046-html-memory-budget-and-warning-sink.md) | HTML memory budget, HEAD parity and the warning sink | Proposed |
| [0047](0047-notify-watcher-registration.md) | The `notify` watcher: per-directory registration, failure reports, scoped CC0-1.0 exception | Proposed |
| [0048](0048-watcher-callback-bridge.md) | The watcher's callback bridge: bounded queue, loss and failure flags, adapter-side filtering | Proposed |
| [0049](0049-watcher-pipeline-task.md) | The watcher pipeline task: deadline-only timer, reconcile hook, cancellation and watcher warnings | Proposed |
| [0050](0050-watch-set-reconciliation.md) | Watch-set reconciliation: source-held hints, voided entries, in-place refresh, bounded retries | Proposed |
| [0051](0051-lifecycle-driven-server-handle.md) | The server handle: start-up and shutdown driven by the domain lifecycle machine | Proposed |
| [0052](0052-quint-toolchain-and-lifecycle-model.md) | Quint toolchain: lockfile-only evaluator; the lifecycle model's two step relations | Proposed |
| [0053](0053-conformance-harness.md) | Conformance harness: an explicit ITF replay path; Quint Connect not adopted yet | Proposed |
| [0054](0054-watcher-model.md) | The watcher model: abstraction, a settled-state encoding of no lost change, a full reload at recovery, root deletion | Proposed |
| [0056](0056-lifecycle-conformance-driver.md) | Lifecycle conformance driver: composition seams, scripted named traces, a sampling relation | Proposed |
| [0057](0057-reload-conformance-driver.md) | Reload conformance driver: in-process SSE bodies, the client's choice read from the model | Proposed |
| [0058](0058-watcher-conformance-driver.md) | Watcher conformance driver: the coordinator in process, the environment from the trace | Proposed |
