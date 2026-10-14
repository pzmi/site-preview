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
| [0011](0011-doc-directory.md) | Use doc/ instead of docs/ | Accepted |
| [0012](0012-security-policies-are-pure-domain-code.md) | Security and change policies are pure domain code | Accepted |
| [0013](0013-per-crate-external-dependency-allowlist.md) | Per-crate external dependency allowlist | Accepted |
| [0014](0014-formal-models-retrofit-then-model-first.md) | Formal models retrofitted in P0, model-first afterwards | Accepted |
| [0015](0015-semantic-checks-via-syn.md) | Semantic architecture checks via a qualified tool or syn | Accepted |
| [0016](0016-test-locations.md) | Where each test level lives | Accepted |
| [0017](0017-reload-conformance-scope.md) | Scope of reload-model conformance | Accepted |
| [0018](0018-coverage-gates.md) | Coverage gates and changed-line computation | Accepted |
| [0019](0019-check-composes-registered-stages.md) | `check` composes registered stages | Accepted |
| [0020](0020-machine-interface-events-and-warnings.md) | JSONL events and warning format | Accepted |
| [0021](0021-host-authority-policy.md) | Allowed Host authorities | Accepted |
| [0022](0022-html-encoding-for-injection.md) | HTML charset labeling and injection eligibility | Accepted |
| [0023](0023-debounce-bounds.md) | Bounds for --debounce | Accepted |
| [0024](0024-p0-supply-chain-and-release-evidence.md) | P0 supply-chain controls and release evidence | Accepted |
| [0025](0025-crates-io-publishing-deferred.md) | crates.io publishing deferred | Accepted |
| [0026](0026-nightly-fuzz-miri-and-concurrency-proof-scope.md) | Nightly scope, fuzz crate placement, concurrency proofs | Accepted |
| [0027](0027-pr-flow-start.md) | When the PR flow starts | Superseded by 0032 |
| [0028](0028-composition-owns-runtime-and-processes.md) | Composition owns runtime, signals and process spawning | Accepted |
| [0029](0029-performance-targets-measured-not-gating.md) | Performance targets are measured, not gating | Accepted |
| [0030](0030-css-js-asset-hot-reload-in-p0.md) | CSS, JS and asset hot reload in P0 | Accepted |
| [0031](0031-hot-reload-protocol-details.md) | Hot-reload protocol details | Accepted |
| [0032](0032-whole-plan-on-main.md) | The whole plan is integrated on main | Accepted |
| [0033](0033-architecture-policy-external-and-dev-dependencies.md) | Architecture policy: external allowlist shape and dev-dependencies | Accepted |
| [0034](0034-cargo-bylaw-qualification.md) | cargo-bylaw is not qualified; semantic rules use syn | Accepted |
| [0035](0035-semantic-rules-scope-and-resolution.md) | Semantic rules: what is analysed and how names resolve | Accepted |
| [0036](0036-hygiene-and-test-quality-rules.md) | Hygiene and test-quality rules and their exceptions | Accepted |
| [0037](0037-host-origin-parsing-and-core-net.md) | Host and Origin parsing details; `core::net` value types in the domain | Accepted |
| [0038](0038-application-port-shapes.md) | Application port shapes: synchronous content, async change source, no adapter types | Accepted |
| [0039](0039-request-resolution-outcomes.md) | Request resolution outcomes: no existence leak, snapshot before open | Accepted |
| [0040](0040-change-coordinator-recovery-and-exhaustion.md) | Change coordinator: recovery signal, reconcile requests and revision exhaustion | Accepted |
| [0041](0041-workflow-pin-parsing-and-security-stage-scope.md) | Workflow pin parsing, secret-scan preconditions and the unmaintained-crate scope | Accepted |
| [0042](0042-command-query-and-observability-rules.md) | Command, query and observability rules: selectors and syntax-level flow | Accepted |
| [0043](0043-lol-html-rewriter-and-mpl-exceptions.md) | The `lol_html` rewriter: strict parsing, verified output, scoped MPL-2.0 exceptions | Accepted |
| [0044](0044-sse-admission-and-absolute-form-scheme.md) | SSE admission order, `HEAD` on the event stream, and the absolute-form scheme | Accepted |
| [0045](0045-connection-limits-and-bounded-shutdown.md) | Connection limits and a bounded shutdown drain | Accepted |
| [0046](0046-html-memory-budget-and-warning-sink.md) | HTML memory budget, HEAD parity and the warning sink | Accepted |
| [0047](0047-notify-watcher-registration.md) | The `notify` watcher: per-directory registration, failure reports, scoped CC0-1.0 exception | Accepted |
| [0048](0048-watcher-callback-bridge.md) | The watcher's callback bridge: bounded queue, loss and failure flags, adapter-side filtering | Accepted |
| [0049](0049-watcher-pipeline-task.md) | The watcher pipeline task: deadline-only timer, reconcile hook, cancellation and watcher warnings | Accepted |
| [0050](0050-watch-set-reconciliation.md) | Watch-set reconciliation: source-held hints, voided entries, in-place refresh, bounded retries | Accepted |
| [0051](0051-lifecycle-driven-server-handle.md) | The server handle: start-up and shutdown driven by the domain lifecycle machine | Accepted |
| [0052](0052-quint-toolchain-and-lifecycle-model.md) | Quint toolchain: lockfile-only evaluator; the lifecycle model's two step relations | Accepted |
| [0053](0053-conformance-harness.md) | Conformance harness: an explicit ITF replay path; Quint Connect not adopted yet | Accepted |
| [0054](0054-watcher-model.md) | The watcher model: abstraction, a settled-state encoding of no lost change, a full reload at recovery, root deletion | Accepted |
| [0056](0056-lifecycle-conformance-driver.md) | Lifecycle conformance driver: composition seams, scripted named traces, a sampling relation | Accepted. Amended by 0070 |
| [0057](0057-reload-conformance-driver.md) | Reload conformance driver: in-process SSE bodies, the client's choice read from the model | Accepted. Amended by 0070 |
| [0058](0058-watcher-conformance-driver.md) | Watcher conformance driver: the coordinator in process, the environment from the trace | Accepted. Amended by 0070 |
| [0059](0059-conformance-negative-controls.md) | Conformance negative controls: faults behind the drivers' seams, a publish seam in the reload driver | Accepted. Amended by 0060, 0070 |
| [0060](0060-conformance-stage-and-trace-corpus.md) | The `conformance` stage in `check`, the trace corpus manifests and `formal.yml` | Accepted. Amended by 0069, 0072 |
| [0061](0061-apalache-formal-verify-scheduled-lane.md) | Apalache in a scheduled `formal-verify` lane | Accepted. Amended (SHA-256 pin) |
| [0062](0062-coverage-stage-scope-and-floors.md) | The `coverage` stage: what is measured, and the floors | Accepted. Amended by 0066 |
| [0063](0063-fuzz-crate-and-architecture-fuzz-only.md) | The fuzz crate: a separate workspace, a `fuzz_only` category and the `xtask fuzz` command | Accepted |
| [0064](0064-changed-line-gate-in-check.md) | The changed-line gate: base, report freshness, and its place in `check` | Accepted. Amended by 0069 |
| [0065](0065-fuzz-targets-beyond-the-domain-and-fuzz-lockfile-audit.md) | Fuzz targets beyond the domain, and the audit of the fuzz lockfile | Accepted |
| [0066](0066-coverage-overall-floor-over-crates-only.md) | The overall coverage floor over `crates/` only; `xtask` report only | Accepted |
| [0067](0067-root-removal-and-watcher-review-fixes.md) | A removed root degrades the watcher for good; start-up failures first; start-up readiness only from `Starting`; composition's hidden seams | Accepted. Amended by 0073 |
| [0068](0068-mutation-scope-and-lane.md) | Mutation testing: scope, test selection and the weekly lane | Accepted |
| [0069](0069-tooling-review-fixes-fuzz-lint-and-gate-bases.md) | Fuzz targets formatted and linted, the changed-line base of a push, the formal lane's concurrency | Accepted. Amended by 0072 |
| [0070](0070-conformance-harness-review-fixes.md) | Conformance harness review fixes: a declared model view, the pipeline's shared decision step, the stop outcome, per-field sensitivity | Accepted |
| [0071](0071-miri-lane-for-domain-and-application.md) | The Miri lane: domain and application tests, forwarded proptest settings, no compile-fail | Accepted |
| [0072](0072-ci-job-topology-and-stage-parity.md) | CI job topology: one job per level, `check --only` parity, restore-only PR caches | Accepted |
| [0073](0073-root-removal-through-the-parent-watch.md) | A root's removal is notified through a watch on its parent | Accepted |
| [0074](0074-native-platform-matrix.md) | Native platform matrix: a `platform` job on four runners, one extra non-gate stage | Accepted |
| [0075](0075-macos-single-recursive-stream.md) | macOS: one recursive FSEvents stream, ignored subtrees dropped in the bridge | Accepted |
| [0076](0076-release-check-archive-and-notices.md) | `release-check`: one archive layout, notices from `cargo about`, a smoke of the extracted binary | Accepted. Amended (fixed notice sections, reproducible archive) |
| [0077](0077-documentation-consistency-stage.md) | Documentation consistency stage: a warning-code registry and the `docs` checks | Accepted |
| [0078](0078-release-workflow.md) | The release workflow: tag-run only, drafts only, write in one job, dry runs on `dry-run-*` tags | Accepted. Amended (release trust boundary) |
| [0080](0080-doctor-and-commit-message-stage.md) | Doctor asserts the gate's toolset, a `commits` stage, and no `cfg!` platform branches | Accepted |
