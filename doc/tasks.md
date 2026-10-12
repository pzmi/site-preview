# site-preview: task breakdown

This file breaks the build of [`plan.md`](plan.md) (product behavior: HTTP contract, security rules, SSE protocol, watcher semantics, CLI) and [`foundation.md`](foundation.md) (engineering foundation: workspace crates, architecture enforcement, xtask, test levels, Quint conformance, CI) into 104 small tasks in dependency order. `plan.md` and `foundation.md` own *what*: a citation such as "plan §5.2" or "fnd §3.2" points to a section of one of them. This file owns *order* and *size*. The ADRs in [`adr/`](adr/README.md) own *why* where the two documents conflict or leave a choice open. Each task is one focused commit, sized so that a fresh agent with a small context can finish it after reading only the sections and files the task names.

## Decisions

Each decision is an ADR. Tasks cite the D-id, and D*n* is ADR *n*+1. ADR 0001 is [Record architecture decisions](adr/0001-record-architecture-decisions.md). **Accepted** decisions come from the owner. **Proposed** decisions are planner defaults that the owner may override; until the owner does, tasks follow them as written.

| D | ADR | Decision | Status |
|---|---|---|---|
| D1 | [0002](adr/0002-multi-crate-workspace.md) | The multi-crate workspace in fnd §2 is the code structure and supersedes plan §4. plan.md stays authoritative for product behavior. | Accepted |
| D2 | [0003](adr/0003-linux-and-macos-only.md) | P0 platforms are Linux and macOS only. Windows CI and Windows target checks are deferred. | Accepted |
| D3 | [0004](adr/0004-rustix-openat-capability.md) | Confined file access uses `rustix` `openat` with `O_NOFOLLOW\|O_CLOEXEC` on every component, wrapped as a capability handle in `preview-adapter-fs`. `cap-std` is rejected because it allows in-root symlinks. `unsafe_code = "forbid"` still holds. | Accepted |
| D4 | [0005](adr/0005-proportionate-foundation-and-early-vertical-slice.md) | Phasing: scaffold, architecture checker, domain/application, serving with an early thin slice, injection+SSE, watcher, CLI/lifecycle/JSON, formal+conformance, hardening, release. No milestone is scaffolding only. | Accepted |
| D5 | [0006](adr/0006-bootstrap-integration-and-dco.md) | One DCO-signed commit per task (`git commit -s`) with subject `<ID>: <summary>`, integrated onto `main` by the orchestrator for the whole plan. Implementing agents never push. | Accepted |
| D6 | [0007](adr/0007-cargo-metadata-and-publishability.md) | `[workspace.package]` carries license `MIT OR Apache-2.0`, the repository URL, `rust-version`, keywords and categories, and `readme`. Only `preview-cli` (binary `site-preview`) is publishable; every other crate has `publish = false`. | Accepted |
| D7 | [0008](adr/0008-pinned-stable-toolchain.md) | Toolchain pinned to stable `1.97.0` in `rust-toolchain.toml`, edition 2024, resolver 3. `Cargo.lock` is committed and CI uses `--locked`. | Accepted |
| D8 | [0009](adr/0009-agent-sandbox-and-pinned-actions.md) | Agent sandbox: no Docker. Chromium is preinstalled at `/opt/pw-browsers`; never run `playwright install`. Node is used only for dev tooling (Quint, Playwright). GitHub Actions are pinned by SHA. | Accepted |
| D9 | [0010](adr/0010-pinned-dev-tools-fail-closed.md) | Dev tools are pinned and documented. When one is missing, xtask exits non-zero with an install command; it never passes silently. | Accepted |
| D10 | [0011](adr/0011-doc-directory.md) | Every document fnd §2 places under `docs/` lives under `doc/` (the existing directory): `doc/architecture.md`, `doc/adr/`, and so on. | Proposed |
| D11 | [0012](adr/0012-security-policies-are-pure-domain-code.md) | URL decoding, component eligibility, Host/Origin policy, injection eligibility, debounce, change classification and watch-set reconciliation are pure functions in `preview-domain`. Adapters translate their inputs and outputs. Serving and watching share one eligibility predicate. | Proposed |
| D12 | [0013](adr/0013-per-crate-external-dependency-allowlist.md) | `architecture.toml` also lists the third-party crates (and features) each crate may use directly. `preview-domain` may use only `percent-encoding`. `preview-application` may use `tokio` with feature `sync` only, for the latest-state channel. `preview-cli` uses no `tokio`; composition owns the runtime. | Proposed |
| D13 | [0014](adr/0014-formal-models-retrofit-then-model-first.md) | The Quint models in P0 are written after the vertical slice exists (D4) and are retrofitted onto it. From FORMAL-09 on, the model-first workflow of fnd §6.2 is mandatory for any protocol, state or concurrency change. | Proposed |
| D14 | [0015](adr/0015-semantic-checks-via-syn.md) | Semantic module rules (fnd §3.3): ARCH-05 qualifies `cargo-bylaw`. If it fails qualification, the rules are implemented in xtask by parsing sources with `syn`, never by text grep. | Proposed |
| D15 | [0016](adr/0016-test-locations.md) | Rust black-box tests (`assert_cmd`) live in `crates/preview-cli/tests/`. Browser tests live in `tests/e2e/browser/` (Node `@playwright/test`, pinned to the release that matches preinstalled Chromium 1194, i.e. 1.56.x, with a lockfile). `client.js` is unit-tested with `node --test` and no dependencies. The shared fixture site is `tests/fixtures/site/`. | Proposed |
| D16 | [0017](adr/0017-reload-conformance-scope.md) | Conformance against `reload.qnt` drives real server-side code: the latest-state channel and the SSE handler. Browser-side decisions are covered by `client.js` unit tests and Playwright. Tests never reimplement the client in Rust. | Proposed |
| D17 | [0018](adr/0018-coverage-gates.md) | The overall line-coverage gate starts at 90% (fnd §5.3) and is ratcheted in `coverage-floor.toml`. The ≥95% changed-line gate is computed in Rust by xtask from LCOV plus `git diff`, with no Python `diff-cover`. | Proposed |
| D18 | [0019](adr/0019-check-composes-registered-stages.md) | `cargo xtask check` runs every stage registered in the check catalog. A task that introduces a gate registers its stage, and a stage that has not been introduced is absent rather than "skipped". `quick` also runs `architecture` because it is fast. | Proposed |
| D19 | [0020](adr/0020-machine-interface-events-and-warnings.md) | `--json` stdout carries JSONL events `ready` (once) and `watcher` (state changes) only. Warnings go to stderr as `site-preview: warning[<code>]: <message>` with stable codes. Both schemas are documented in the README. | Proposed |
| D20 | [0021](adr/0021-host-authority-policy.md) | Allowed Host authorities: the bound numeric IP:port, plus `localhost:<port>` only when the listener is loopback. In `--allow-remote` mode only numeric IPs are allowed (the bound IP; for a wildcard listener, any numeric IP). The port must always match. | Proposed |
| D21 | [0022](adr/0022-html-encoding-for-injection.md) | HTML is labeled `text/html; charset=utf-8` and is eligible for injection only when its bytes are valid UTF-8 and any `<meta charset>` agrees. Otherwise it is served unchanged as `text/html` with a once-per-path warning. | Proposed |
| D22 | [0023](adr/0023-debounce-bounds.md) | `--debounce` accepts `ms` and `s` units in the range 10 ms to 10 s inclusive. Anything else is a usage error (exit 2). | Proposed |
| D23 | [0024](adr/0024-p0-supply-chain-and-release-evidence.md) | P0 supply chain: `cargo-deny`, Gitleaks, actionlint, CodeQL (Rust), Dependabot (actions and cargo). Releases ship `SHA256SUMS`, GitHub build-provenance attestations and a CycloneDX SBOM. `cargo-vet`, `cargo-machete` and detached signatures are deferred (plan §14 item 10). | Proposed |
| D24 | [0025](adr/0025-crates-io-publishing-deferred.md) | Publishing to crates.io is out of scope for v0.1.0, because `preview-cli` cannot be published while its path dependencies have `publish = false`. Distribution is GitHub release archives. | Proposed |
| D25 | [0026](adr/0026-nightly-fuzz-miri-and-concurrency-proof-scope.md) | One dated nightly is pinned for `cargo-fuzz` and Miri only. `fuzz/` stays outside the workspace and is listed as `fuzz_only` in `architecture.toml`. Loom, Shuttle and Kani are not adopted in P0 unless code adds a custom synchronization primitive (P0 uses only tokio `watch` and `Semaphore`). | Proposed |
| D26 | [0027](adr/0027-pr-flow-start.md) | Superseded by D31: the planned switch to branch protection and a PR flow at OPS-04 is dropped. | Superseded |
| D27 | [0028](adr/0028-composition-owns-runtime-and-processes.md) | `preview-composition` builds the Tokio runtime, installs signal handlers and launches the browser opener (`open`/`xdg-open`, no shell). `preview-cli` parses flags, renders output and maps errors to exit codes. | Proposed |
| D28 | [0029](adr/0029-performance-targets-measured-not-gating.md) | The targets in plan §13 are measured by a reproducible non-blocking xtask procedure and recorded in `doc/performance.md`. They become blocking only after the owner approves a budget (fnd §12 phase 5). | Proposed |
| D29 | [0030](adr/0030-css-js-asset-hot-reload-in-p0.md) | CSS, JS and asset hot reload are **P0**, overriding plan §7.3 "always full-page reload" and plan §14 item 1. A debounced revision carries the changed eligible paths; the SSE `reload` event is `{instance_id, revision, changes:[{path, kind}], full}`. CSS swaps in place, assets are cache-busted, JS reloads with scroll restore or uses an opt-in hot-accept API, and HTML reloads fully. Anything unknown or unmatched falls back to a full reload. The render-time baseline/instance logic is unchanged. | Accepted |
| D30 | [0031](adr/0031-hot-reload-protocol-details.md) | Hot-reload details. Kinds by extension: `css` (`.css`), `js` (`.js`, `.mjs`), `html` (`.html`, `.htm`), `asset` (images, fonts, media), `other`. At most 64 changes per revision; above that `full = true`. `full = true` also for overflow/rescan, directory changes, any `html`/`other` change, and whenever the SSE connection's previous sent revision ≠ `revision − 1` (no hot apply across a coalesced gap). The scroll position is carried across a JS reload in `history.state` (key `__sitePreview`, merged only into object or null state, removed after restore). No cookies, `localStorage` or `sessionStorage`. Form state is not restored in P0. Same-origin iframes are handled by their own injected client. Amended 2026-10-11: a `Create`, `Remove` or `Rename` is checked as a file and as a directory; a name denied only as a directory (`vendor`) makes the set full without its path, and one eligible both ways also requests a watch-set reconcile. | Proposed |
| D31 | [0032](adr/0032-whole-plan-on-main.md) | The whole plan is integrated directly on `main`: one DCO-signed commit per task, pushed by the orchestrator. No task switches to a PR flow or enables branch protection; a PR flow may follow only after the plan is complete, via a new ADR. Review checkpoints and CI on `main` are the quality net, and red CI on `main` is fixed immediately on `main`. | Accepted |
| D32 | [0033](adr/0033-architecture-policy-external-and-dev-dependencies.md) | `[external]` in `architecture.toml` has `normal`, `dev` and `build` sub-tables mapping a crate to `{ dep = [features] }` (`"default"` = default features, `"*"` = any). A crate allowed as normal is also allowed as dev. Fixture workspaces commit a `Cargo.lock` and stub third-party crates as vendored sources. | Proposed |
| D33 | [0034](adr/0034-cargo-bylaw-qualification.md) | `cargo-bylaw` 0.1.0 is not qualified (ARCH-05): it cannot select `std::fs` apart from the rest of `std`, and a selector that matches nothing or an unparseable file passes. The semantic rules are implemented in xtask with `syn` per D14; the tool is in no gate. | Proposed |
| D34 | [0035](adr/0035-semantic-rules-scope-and-resolution.md) | The `syn` semantic rules (ARCH-06) read every file a Cargo target reaches through `mod`, `#[path]` and literal `include!`, in every `cfg` branch, and fail on what they cannot read. Names resolve through `use`, `extern crate`, globs, re-exports and manifest renames; macro token streams are scanned conservatively. Domain purity, filesystem authority and layer isolation apply to production code; `unsafe` is rejected everywhere. | Proposed |
| D35 | [0036](adr/0036-hygiene-and-test-quality-rules.md) | `[[exceptions.lints]]` (`path`, `lint`, `adr`) approves an `allow`/`expect`/`coverage(off)` or a production `todo!`/`unimplemented!`/`dbg!` (as `clippy::todo` etc.) only when the ADR file exists; an entry that matches nothing fails. `#[ignore]` has no exception. Test code may sleep only inside production code of `preview-testkit/src/poll.rs`. Tests are named `should_<outcome>_when_<condition>` with `// given`, `// when`, `// then` in order. | Proposed |
| D36 | [0037](adr/0037-host-origin-parsing-and-core-net.md) | The domain uses `core::net` address value types; `std::net` stays forbidden. Host parsing is strict (printable ASCII, no userinfo, bracketed IPv6 without zone id, canonical dotted-quad IPv4, canonical port, 80 when absent). `localhost` (any ASCII case) is allowed whenever the listener is loopback; `0.0.0.0`/`::` as Host is always rejected. SSE: `Sec-Fetch-Site: cross-site` refused; an Origin must be `http://` plus the accepted Host authority, port included; `null` refused; neither header allowed. | Proposed |
| D37 | [0038](adr/0038-application-port-shapes.md) | `SiteContent` and `Clock` are synchronous and object safe; `ChangeSource` is async (`impl Future + Send`), cancellation safe and used through generics, with the batch generic over the event type until DOM-05 lands. Refusals carry categories, never paths. No port names an adapter or runtime type. Amended 2026-10-11: `ChangeBatch::Failed` carries `SourceFailure::{RegistrationFailed, RuntimeFailure}`, never `RevisionExhausted`. | Proposed |
| D38 | [0039](adr/0039-request-resolution-outcomes.md) | `ResolveRequest` decodes, checks eligibility, snapshots, then opens; malformed is 400, traversal 404, ineligible 403 without an open. A final name denied only as a directory, requested without a slash, gives `NotFound` for every outcome but a plain file, so a redirect or error never reveals it. `Io` becomes `Unavailable(kind)`; `RedirectToSlash` carries the decoded path; the snapshot is an `FnOnce() -> SiteState`. | Proposed |
| D39 | [0040](adr/0040-change-coordinator-recovery-and-exhaustion.md) | Watcher recovery is reported by composition through `ChangeCoordinator::watcher_ready(dirs)` after a successful (re-)registration; the `ChangeSource` port has no recovered batch. `RecordOutcome::needs_reconcile` is set by a directory change, an unattributable (`Rescan`) event and an overflow. `RevisionExhausted` degrades once and makes the coordinator refuse record, overflow and settle without retrying; a later `watcher_ready` is refused. `receive(batch, now)` is the single batch-to-command mapping. Amended 2026-10-11: exhaustion comes from `settle` only; once exhausted `deadline()` and `receive` return no deadline; `watcher_ready(0)` is refused; a create, remove or rename of an entry eligible as a directory also requests a reconcile. Amended 2026-10-12 (ADR 0054): after a reconcile composition reports readiness through `watcher_reconciled(dirs, now)`; a recovery there marks the next revision full (`FullReason::Recovered`, no rescan), so a change made while a directory was unwatched is caught up. | Proposed |
| D40 | [0041](adr/0041-workflow-pin-parsing-and-security-stage-scope.md) | The action-pin check parses YAML (`yaml-rust2`) and walks `jobs.*.uses`, `jobs.*.steps[*].uses` and `runs.steps[*].uses`, failing closed on any other `uses` token, merge key or parse error; a pin is 40 lowercase hex after the first `@`, a local `./` path or a `docker://…@sha256:` digest. xtask resolves the repository root from its manifest, never the current directory. The secret scan refuses a non-root, shallow or empty history and zero scanned commits. `unmaintained = "workspace"` fails direct dependencies only. The cargo-deny stage is `deny`. | Proposed |
| D41 | [0042](adr/0042-command-query-and-observability-rules.md) | ARCH-09 selects `preview-application`'s `commands` and `queries` modules and `impl ReloadState`, failing `ARCH-missing-selector` when production lacks any; name resolution records the modules a path passes through, so re-exports count. Commands name no `crate::queries` or HTTP type; queries name no `crate::commands`, take no `&mut self` or `&mut` of domain/application/generic state, and call no `ReloadState` method beyond `new`, `current`, `subscribe` (matched by name). The domain names no instrumentation crate, and no `tracing`/`log`/`observability`/`metrics` value, also through locals in one function, reaches a branch, scrutinee, guard, let-else or returned value. | Proposed |
| D42 | [0043](adr/0043-lol-html-rewriter-and-mpl-exceptions.md) | The HTML rewriter is `lol_html` 3 in strict mode (UTF-8, no ESI, no charset adjustment), inserting before the first body end tag outside a closed `<template>`, else at document end; every result must be the input plus the tag at one offset and parse as exactly one client `<script>`, else `Failed`. "Already present" is a `script` whose `src` is exactly `/__site_preview/client.js`. `deny.toml` accepts MPL-2.0 only for `cssparser`, `cssparser-macros`, `dtoa-short` and `selectors`. | Proposed |
| D43 | [0044](adr/0044-sse-admission-and-absolute-form-scheme.md) | The Host gate stores the accepted `Authority` in request extensions and refuses an absolute-form target whose scheme is not `http` with 421. The event stream checks Origin first (raw values; a repeated or non-text `Origin`/`Sec-Fetch-Site` refused; 403 fixed body), then `GET` takes one of `DEFAULT_MAX_STREAMS = 64` permits (`503`, `Retry-After: 2`), owned by the stream so a dropped body or an ended stream returns it. `HEAD` answers headers only, without subscribing or taking a permit. | Proposed |
| D44 | [0045](adr/0045-connection-limits-and-bounded-shutdown.md) | Composition serves through its own accept loop over hyper's HTTP/1 builder (`hyper`, `hyper-util` direct), with `ServerLimits` (not on the CLI): a 10 s header-read timeout that is also the idle keep-alive timeout, at most 128 connections (an excess one is closed at once), and a 3 s shutdown drain after which open connections are dropped and the run exits 0. A second SIGINT/SIGTERM during the drain exits 128 + signal (130, 143). `--json` (CLI-06) is accepted and switches stdout to JSONL. | Proposed |
| D45 | [0046](adr/0046-html-memory-budget-and-warning-sink.md) | HTML held in memory (at most 8 MiB per page) reserves twice its size plus 64 KiB from a shared 128 MiB budget until its body is sent or dropped, sent in 64 KiB copies; a request that does not fit waits. HEAD of HTML does the GET's work and sends its headers (the rewritten `Content-Length`) without a body or a warning. The adapter's `WarningSink` port, re-exported by composition and implemented by the CLI on stderr, gets each `(code, path)` once (at most 4096 pairs). | Proposed |

## Rules for every task

1. **Read first:** this file's Decisions table, then the sections the task cites, then any existing file it changes. Do not read either spec document whole. If a cited section contradicts a Decision, the Decision wins; say so in the commit message.
2. **Green after every task:** `cargo xtask quick` passes (fmt check, Clippy `-D warnings` on all targets, `architecture` once ARCH-01 lands, fast nextest). **From OPS-01 on**, `cargo xtask check` must pass instead. A gate that cannot run in the sandbox because a pinned tool is missing fails with the D9 message. Report it as a tooling blocker; never fake a pass and never remove the stage.
3. **Commit:** exactly one DCO-signed commit per task, `git commit -s`, with subject `<ID>: <summary>` (for example `FS-01: rustix openat root capability`). The orchestrator integrates onto `main` for the whole plan (D5, D31) and pushes after each merged task. **Implementing agents never push**, never create tags and never change remote settings. Only the orchestrator runs `gh`.
4. **Tests:** name them `should_<outcome>_when_<condition>`, with `// given`, `// when`, `// then` comments. Deterministic tests never sleep: use injected clocks, `tokio::time::pause`, channels or readiness signals. Real-OS watcher and black-box tests may poll a condition, but only through the shared bounded helper (with a reason) introduced in TEST-01. Use `tempfile` and port `0`; never a fixed path or a fixed port.
5. **No silencing:** no `#[ignore]`, no `#[allow]`/`#[expect]`, no coverage exclusions, no production `todo!()`/`unimplemented!()`/`dbg!()` without an entry in `architecture.toml`'s `[exceptions]` that cites an ADR. Never delete a failing test, weaken a modeled invariant or rewrite a golden fixture just to get green.
6. **New crate:** a new crate is added to `members` in `Cargo.toml` and to `architecture.toml` in the same commit. A new third-party dependency is added to the crate's external allowlist (D12) in the same commit, using the latest stable compatible version, locked in `Cargo.lock`. Never invent a version number.
7. **Decisions:** a change that adds or reverses an architectural decision adds a new ADR, or supersedes an existing one, in the same commit: copy `doc/adr/template.md`, take the next number, update `doc/adr/README.md`, and on a superseded ADR set its status to `Superseded by NNNN`. A task that implements an existing decision cites its ADR in the commit body. Agents never flip an ADR from Proposed to Accepted; only the owner does.
8. **Browser:** never run `playwright install`. Set `PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`. Node is a dev-only dependency: the shipped binary must never need it.
9. **Parallelism:** sequential by default. Only tasks whose "Parallel with" column names each other may run concurrently, in separate worktrees with disjoint file sets. The orchestrator rebases. If two parallel tasks both need to touch `Cargo.toml`, `Cargo.lock` or `architecture.toml`, the second one rebases and re-locks.
10. **Impact:** state the task's **Impact** line in the commit body. A change to the model, a port contract, the public API, the dependency graph or a security boundary also updates the doc that owns it (`doc/architecture-rules.md`, `doc/security.md`, `doc/formal-models.md`) in the same commit, once that doc exists.
11. **Scope:** do not create or edit `CLAUDE.md` or `.claude/`; they are maintained outside this task list. If a task renames a command that `CLAUDE.md` mentions, say so in the report.

**Model key:** `opus` covers design, security boundaries, concurrency, formal models and the architecture checker. `sonnet` covers well-specified implementation. `haiku` covers mechanical docs and config.

**Review checkpoints (⛳):** after a ⛳ task, the orchestrator runs `/code-review` over the diff since the previous checkpoint and tags `checkpoint-<ID>` before work builds on top. The checkpoints are **FND-05** (scaffold+CI), **ARCH-07** (architecture checker), **APP-04** (domain+application core, including the change set), **E2E-01** (M1 secure serving), **E2E-02** (M2 injection+SSE), **E2E-03** (M3 watcher), **E2E-07** (M4 agent workflow), **FORMAL-10** (formal+conformance), **SEC-05** (hardening), **OPS-04** (CI matrix green twice, required checks documented) and **OPS-06** (release pipeline, before the first tag).

## Overview

| ID | Title | Depends on | Model | ⛳ | Parallel with |
|---|---|---|---|---|---|
| FND-01 | Workspace, toolchain pin, crate stubs, lockfile, licenses | — | sonnet | | |
| FND-02 | `site-preview` binary: `--version`, `--help` | FND-01 | sonnet | | FND-03 |
| FND-03 | xtask skeleton: command surface, `quick`, `list-checks` | FND-01 | sonnet | | FND-02 |
| FND-04 | Pinned dev tools: manifest, `doctor`, `install-tools` | FND-03 | sonnet | | |
| TEST-01 | nextest config, doctests, bounded-poll helper | FND-04 | sonnet | | |
| FND-05 | CI skeleton: Linux `quick` job | TEST-01, FND-02 | sonnet | ⛳ | |
| ARCH-01 | Policy schema, inventory check, fixture harness | FND-05 | opus | | |
| ARCH-02 | Workspace edge allowlist, kinds, closure, paths | ARCH-01 | opus | | |
| ARCH-03 | External deps, features, sources, targets | ARCH-02 | opus | | |
| ARCH-04 | Positive fixture and rule-catalog completeness | ARCH-03 | sonnet | | ARCH-05 |
| ARCH-05 | Qualify cargo-bylaw (spike + ADR) | ARCH-01 | opus | | ARCH-04 |
| ARCH-06 | Semantic rules: domain purity, authority, isolation | ARCH-04, ARCH-05 | opus | | |
| ARCH-07 | Semantic rules: hygiene and test quality | ARCH-06 | opus | ⛳ | |
| OPS-01 | `cargo xtask test` and `check`; CI runs `check` | ARCH-07 | sonnet | | DOC-01 |
| DOC-01 | `doc/architecture.md`, `doc/architecture-rules.md` | ARCH-07 | haiku | | OPS-01 |
| SEC-01 | cargo-deny policy and `security` stage | OPS-01 | sonnet | | DOM-01 |
| SEC-02 | Gitleaks and actionlint | SEC-01 | sonnet | | DOM-02 |
| SEC-03 | CodeQL and Dependabot | SEC-02 | haiku | | DOM-03 |
| DOM-01 | `SitePath`: URL decoding and component validation | OPS-01 | opus | | SEC-01 |
| DOM-02 | Component eligibility predicate | DOM-01 | opus | | SEC-02 |
| DOM-03 | `SiteState`, `InstanceId`, `Revision`, domain events | OPS-01 | sonnet | | DOM-01, DOM-02, SEC-03 |
| DOM-04 | Global trailing-edge debounce machine | DOM-03 | opus | | DOM-05, DOM-06 |
| DOM-05 | Change classification | DOM-02 | sonnet | | DOM-04, DOM-06 |
| DOM-06 | Watch-set reconciliation | DOM-02 | sonnet | | DOM-04, DOM-05 |
| DOM-07 | Host authority and SSE Origin policy | OPS-01 | opus | | DOM-04..06 |
| DOM-08 | Injection eligibility policy | DOM-03 | sonnet | | DOM-07 |
| DOM-09 | Lifecycle and watcher-health state machines | DOM-03 | opus | | DOM-08 |
| DOM-10 | Change set: kinds, URL paths, cap, full fallback | DOM-04, DOM-05 | opus | | DOM-07..09 |
| TEST-02 | Property-test conventions and seed persistence | DOM-02, DOM-04 | sonnet | | APP-01 |
| APP-01 | Application layout and ports | DOM-02, DOM-03 | opus | | TEST-02 |
| TEST-03 | Testkit fakes | APP-01 | sonnet | | |
| APP-02 | `ReloadState` latest-state channel | TEST-03, DOM-10 | sonnet | | APP-03 |
| APP-03 | `ResolveRequest` query with render snapshot | TEST-03, DOM-02 | opus | | APP-02 |
| APP-04 | Change commands: record, settle, overflow, health | APP-02, DOM-04, DOM-05, DOM-09, DOM-10 | opus | ⛳ | |
| ARCH-09 | Semantic rules: commands, queries, observability | APP-04 | opus | | FS-01 |
| FS-01 | Root capability: `openat` per component | APP-01 | opus | | APP-02..04 |
| FS-02 | `SiteContent` adapter: index resolution, errors | FS-01, APP-03 | opus | | |
| FS-03 | Root opening and reserved-path check | FS-01 | sonnet | | FS-02 |
| TEST-04 | `SiteContent` contract harness | FS-02, TEST-03 | opus | | FS-04 |
| FS-04 | Concurrent symlink-swap race test | FS-02 | opus | | TEST-04 |
| ARCH-08 | Compile-fail (`trybuild`) API fixtures | FS-02, DOM-01 | sonnet | | FS-04 |
| HTTP-01 | Router, method gate, errors, internal namespace | APP-03 | sonnet | | FS-02 |
| HTTP-02 | GET/HEAD file streaming | HTTP-01, FS-02 | sonnet | | |
| CLI-01 | Thin vertical slice: `serve` a directory | HTTP-02, FS-03 | opus | | |
| HTTP-03 | MIME mapping and overrides | CLI-01 | sonnet | | HTTP-04 |
| HTTP-04 | Directory slash redirect and index | CLI-01 | sonnet | | HTTP-03 |
| HTTP-05 | Host authority middleware | CLI-01, DOM-07 | opus | | |
| CLI-02 | Serve flags, bind rules, multiple instances | HTTP-05 | sonnet | | |
| E2E-01 | Black-box harness and HTTP/security suite | CLI-02, HTTP-03, HTTP-04, FS-04 | opus | ⛳ | DOC-02 |
| DOC-02 | `doc/testing.md` | TEST-04, TEST-02 | haiku | | E2E-01 |
| HTTP-06 | `lol_html` rewriter | E2E-01 | opus | | HTTP-09 |
| HTTP-07 | Injection in GET responses | HTTP-06, DOM-08, APP-02 | opus | | |
| HTTP-08 | CSP restriction warning | HTTP-07 | sonnet | | HTTP-10 |
| HTTP-09 | Embedded `client.js` and its route | E2E-01 | sonnet | | HTTP-06 |
| HTTP-10 | SSE endpoint | HTTP-09, APP-02 | opus | | HTTP-08 |
| HTTP-11 | SSE Origin guard and 64-connection limit | HTTP-10, DOM-07 | opus | | |
| HTTP-12 | SSE `reload` payload: changes, `full`, gap rule | HTTP-11 | opus | | |
| HTTP-13 | Client: CSS hot swap | HTTP-12 | sonnet | | HTTP-14 |
| HTTP-14 | Client: JS reload with scroll restore | HTTP-12 | sonnet | | HTTP-13 |
| HTTP-15 | Client: opt-in JS hot-accept API | HTTP-14 | opus | | HTTP-16 |
| HTTP-16 | Client: asset cache-bust | HTTP-13 | sonnet | | HTTP-15 |
| E2E-02 | Injection and SSE black-box suite | HTTP-07, HTTP-08, HTTP-12 | sonnet | ⛳ | DOC-03, HTTP-13..16 |
| DOC-03 | `doc/security.md` | HTTP-11 | sonnet | | E2E-02 |
| FS-05 | Eligible-directory traversal | E2E-02, DOM-06 | sonnet | | |
| FS-06 | notify registration and degraded reporting | FS-05, APP-04 | opus | | FS-07 |
| FS-07 | notify event translation | FS-05, DOM-05 | sonnet | | FS-06 |
| FS-08 | Bounded callback bridge and overflow flag | FS-06, FS-07 | opus | | |
| TEST-05 | `ChangeSource` contract harness | FS-08, TEST-03 | opus | | CLI-03 |
| CLI-03 | Composition: watcher pipeline and debounce task | FS-08, APP-04 | opus | | TEST-05 |
| FS-09 | Directory create/remove reconciliation | CLI-03 | opus | | |
| E2E-03 | Watcher black-box suite (Linux) | FS-09, TEST-05 | sonnet | ⛳ | |
| CLI-04 | Lifecycle, startup ordering, startup refusals | E2E-03, DOM-09 | opus | | |
| CLI-05 | Signals and graceful shutdown | CLI-04 | opus | | |
| CLI-06 | `--json` ready and watcher events; final human output | CLI-05 | sonnet | | CLI-07 |
| CLI-07 | `--open` browser launcher | CLI-04 | sonnet | | CLI-06 |
| E2E-04 | Playwright harness | CLI-06 | sonnet | | |
| E2E-05 | Browser smoke: load, edit, reload, assets, widths | E2E-04 | sonnet | | |
| E2E-06 | Browser reconnect and restart recovery | E2E-05 | opus | | DOC-04, E2E-09 |
| E2E-09 | Browser hot reload: CSS, assets, JS scroll, accept API | E2E-05, HTTP-15, HTTP-16 | sonnet | | E2E-06 |
| DOC-04 | README completion and `doc/runbook.md` | CLI-06, CLI-07, HTTP-15 | sonnet | | E2E-06 |
| E2E-07 | `AGENTS.md`, example skill, agent workflow test | E2E-06, E2E-09, DOC-04 | sonnet | ⛳ | |
| FORMAL-01 | Quint toolchain pin and `lifecycle.qnt` | E2E-07 | opus | | |
| FORMAL-02 | `watcher.qnt` | FORMAL-01 | opus | | FORMAL-03 |
| FORMAL-03 | `reload.qnt` | FORMAL-01 | opus | | FORMAL-02 |
| FORMAL-04 | Quint Connect evaluation and ITF replay core | FORMAL-01 | opus | | FORMAL-02, FORMAL-03 |
| FORMAL-05 | Lifecycle conformance driver | FORMAL-04 | opus | | |
| FORMAL-06 | Watcher conformance driver | FORMAL-02, FORMAL-05 | opus | | FORMAL-07 |
| FORMAL-07 | Reload conformance driver | FORMAL-03, FORMAL-05 | opus | | FORMAL-06 |
| FORMAL-08 | Conformance negative controls | FORMAL-06, FORMAL-07 | opus | | |
| FORMAL-09 | Trace corpus, `conformance` stage, `formal.yml` | FORMAL-08 | opus | | |
| FORMAL-10 | Apalache `formal-verify` scheduled lane | FORMAL-09 | sonnet | ⛳ | TEST-06 |
| TEST-06 | LLVM coverage and overall floor | FORMAL-09 | sonnet | | FORMAL-10 |
| TEST-07 | Changed-line coverage gate | TEST-06 | opus | | SEC-04 |
| TEST-08 | Mutation testing, baseline, `mutation.yml` | TEST-07 | sonnet | | SEC-04 |
| SEC-04 | Fuzz crate, pinned nightly, `SitePath` target | FORMAL-09 | sonnet | | TEST-07, TEST-08 |
| SEC-05 | Fuzz targets: rewriter, Host, change sequences; `fuzz.yml` | SEC-04 | opus | ⛳ | TEST-09 |
| TEST-09 | Miri lane and concurrency-proof scope | SEC-04 | sonnet | | SEC-05 |
| E2E-08 | Soak and performance measurement | SEC-05 | sonnet | | OPS-02 |
| OPS-02 | CI job topology and failure evidence | SEC-05, TEST-08 | sonnet | | E2E-08 |
| OPS-03 | Linux+macOS native matrix | OPS-02 | sonnet | | |
| OPS-04 | CI matrix green twice on `main`, `doc/sdlc.md`, required checks | OPS-03 | sonnet | ⛳ | |
| OPS-05 | `release-check`: packaged-binary smoke | OPS-04 | sonnet | | DOC-05 |
| DOC-05 | Documentation consistency check | OPS-04 | sonnet | | OPS-05 |
| OPS-06 | `release.yml`: archives, SHA256SUMS, provenance, SBOM | OPS-05, DOC-05 | opus | ⛳ | |
| OPS-07 | v0.1.0: changelog, install docs, tag handoff | OPS-06, E2E-08 | haiku | | |

**Execution order:** FND → ARCH → OPS-01, DOC-01, SEC-01..03 interleaved with DOM → TEST-02, APP, TEST-03, APP-04 ⛳ → ARCH-09 with FS-01..04, TEST-04 and ARCH-08 → HTTP-01, HTTP-02 → **CLI-01 (first working binary)** → HTTP-03..05, CLI-02 → E2E-01 ⛳ (M1) → HTTP-06..12 → E2E-02 ⛳ (M2) ∥ HTTP-13..16 (hot-reload client) → FS-05..08, CLI-03, FS-09, TEST-05 → E2E-03 ⛳ (M3) → CLI-04..07 → E2E-04..06, E2E-09 → E2E-07 ⛳ (M4) → FORMAL-01..10 ⛳ → TEST-06..09, SEC-04..05 ⛳ → E2E-08, OPS-02..04 ⛳ → OPS-05, DOC-05, OPS-06 ⛳ → OPS-07.

---

## Lane FND: scaffold, toolchain, xtask skeleton, CI skeleton

### FND-01: Workspace, toolchain pin, crate stubs, lockfile, licenses
- **Depends on:** none. **Model:** sonnet. **Size:** M.
- **Read:** fnd §2 (tree and §2.1 only), fnd §4 (whole); D1, D6, D7, D10, D12.
- **Create/Change:**
  - Root `Cargo.toml`: virtual manifest with explicit `members` (the 8 `crates/preview-*` plus `tools/xtask`), `resolver = "3"`, and `[workspace.package]` per D6/D7 (`version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.97"`, license, repository, keywords `["live-reload", "static-site", "preview", "dev-server", "cli"]`, the 3 categories, `readme = "README.md"`). `[workspace.lints]` per fnd §4 adds `clippy::all = deny` and `missing_docs = warn`. `[workspace.dependencies]` is empty for now.
  - `rust-toolchain.toml`: `channel = "1.97.0"`, components `rustfmt`, `clippy`, `llvm-tools-preview`.
  - `.cargo/config.toml`: alias `xtask = "run --locked --package xtask --"`.
  - `crates/preview-{domain,application,adapter-fs,adapter-http,composition,testkit,conformance}/{Cargo.toml,src/lib.rs}`: crate-level doc comment stating the crate's single responsibility (fnd §2.2); `lints.workspace = true`; `publish = false`. Add only the path dependencies allowed by fnd §3.2.
  - `crates/preview-cli/{Cargo.toml,src/main.rs}`: `[[bin]] name = "site-preview"`; publishable (D6, see D24); `fn main()` only.
  - `tools/xtask/{Cargo.toml,src/main.rs}`: `publish = false`; prints usage.
  - `architecture.toml`: `schema = 1`, plus the `[workspace]` and `[dependencies.normal]` tables from fnd §3.2, an empty `[external]` table and an empty `[exceptions]` table. The checker arrives in ARCH-01.
  - `Cargo.lock`; `.gitignore` (`/target`, `formal/traces/generated/`, `fuzz/target`, `fuzz/corpus`, `fuzz/artifacts`, `node_modules/`, `tests/e2e/browser/test-results/`); `LICENSE-MIT`, `LICENSE-APACHE`; `README.md` (title, one-line tagline, "work in progress", links to `doc/plan.md` and `doc/tasks.md`; DOC-04 completes it).
  - Commit the existing `doc/` unchanged.
- **Accept:** `cargo build --locked --workspace` succeeds. `cargo metadata --format-version 1` lists exactly 9 workspace members. `rustc --version` inside the repo reports 1.97.0. `cargo fmt --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings` are clean. `grep -L 'publish = false' crates/*/Cargo.toml` prints only `crates/preview-cli/Cargo.toml`.
- **Impact:** dependency graph (initial).

### FND-02: `site-preview` binary: `--version`, `--help`
- **Depends on:** FND-01. **Model:** sonnet. **Size:** S. Parallel with FND-03.
- **Read:** plan §3 "Command" block only; D12, D27.
- **Create/Change:** `crates/preview-cli/src/{main.rs,args.rs,exit.rs}`: a `clap` derive parser with a `serve` subcommand placeholder that has no flags yet, and an `ExitCode` mapping (0 for success, 2 for usage). `clap` goes in the cli's `[external]` allowlist. Tests: `crates/preview-cli/tests/cli_meta.rs` (`assert_cmd`, `predicates` as dev-dependencies) with `should_print_version_when_called_with_version_flag` (`site-preview 0.1.0`), `should_list_serve_when_called_with_help`, and `should_exit_2_when_given_unknown_flag`.
- **Accept:** the three tests pass. `site-preview serve` without the later flags exits 2 with a clap usage error; the subcommand is not wired until CLI-01, and nothing calls `todo!()`.
- **Impact:** public API (CLI surface).

### FND-03: xtask skeleton: command surface, `quick`, `list-checks`
- **Depends on:** FND-01. **Model:** sonnet. **Size:** M. Parallel with FND-02.
- **Read:** fnd §9 (whole); D9, D18.
- **Create/Change:** `tools/xtask/src/{main.rs,catalog.rs,quick.rs,process.rs}`. Use `clap` for the subcommands and add every command named in fnd §9, plus `doctor` and `install-tools`. `catalog.rs` holds the check catalog: id, level, command, owner task, and whether `check` includes it. `quick` runs `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, and `cargo test --locked --workspace` (nextest replaces it in TEST-01). It streams output and stops at the first failure with that stage's exit code. `list-checks` prints the catalog as JSON. Each command that has not been implemented yet exits 3 with `not available yet: introduced by <task id>`. Unit tests: `catalog.rs` ids are unique; every subcommand appears in the catalog.
- **Accept:** `cargo xtask quick` exits 0 on the tree. `cargo xtask list-checks | jq -e 'length > 0'` succeeds. `cargo xtask coverage` exits 3 and names TEST-06. Negative: a temporary unformatted file makes `quick` exit non-zero (covered by a unit test of the stage runner using a failing fake command).
- **Impact:** none (tooling).

### FND-04: Pinned dev tools: manifest, `doctor`, `install-tools`
- **Depends on:** FND-03. **Model:** sonnet. **Size:** S.
- **Read:** fnd §4 (4th bullet), fnd §9 (last paragraph); D9, D25.
- **Create/Change:** `tools/versions.toml` lists each tool with its exact version and install method: `cargo-nextest`, `cargo-deny`, `cargo-llvm-cov`, `cargo-mutants`, `cargo-fuzz`, `gitleaks`, `actionlint`, `cargo-cyclonedx`; Node tools `quint` and `@playwright/test` (versions owned by `formal/package-lock.json` and `tests/e2e/browser/package-lock.json`, only noted here); the pinned nightly date; and Apalache. Use the latest stable release at task time, verified with `cargo search` or the project's release page; never invent a version. Add `tools/xtask/src/tools.rs`: `doctor` checks presence and exact version of each tool and prints `missing <tool> <version>: run 'cargo xtask install-tools <tool>'` or a manual URL. `install-tools [tool…]` runs `cargo install --locked <tool>@<version>`, or prints a manual step for binary tools (gitleaks, actionlint, Apalache). Add `pub fn require(tool)`, which every later stage calls before invoking a tool.
- **Accept:** `cargo xtask doctor` exits non-zero when any tool is missing and lists every missing tool. A unit test `should_fail_with_install_hint_when_tool_missing` (PATH set to an empty temp dir) passes. Running `cargo xtask install-tools cargo-nextest` in the sandbox, then `doctor`, reports nextest as OK. If the network blocks an install, the report says so.
- **Impact:** none (tooling).

### TEST-01: nextest config, doctests, bounded-poll helper
- **Depends on:** FND-04. **Model:** sonnet. **Size:** S.
- **Read:** fnd §5 (table), fnd §5.2, fnd §9 (nextest paragraph).
- **Create/Change:** `.config/nextest.toml` with profiles `default` and `ci`; `retries = 0`; `fail-fast = false` in ci; JUnit output in ci; a `slow-timeout`; and test group `os-watcher` with `max-threads = 4` for real-watcher tests. `quick` switches to `cargo nextest run --locked --workspace` (calling `tools::require`) plus `cargo test --locked --workspace --doc`. Add `crates/preview-testkit/src/poll.rs`: `wait_until(deadline, reason, condition)`, the only sanctioned polling primitive (rule 4), with unit tests including `should_report_reason_when_deadline_expires`. Production crates must not use it (the testkit is never a production dependency).
- **Accept:** `cargo xtask quick` runs nextest and doctests. Without nextest installed, `quick` exits non-zero with the install hint. `cargo nextest list --profile ci` succeeds.
- **Impact:** none.

### FND-05: CI skeleton: Linux `quick` job ⛳
- **Depends on:** TEST-01, FND-02. **Model:** sonnet. **Size:** S.
- **Read:** fnd §8 (2nd paragraph), fnd §10 (first block, headings only); D8, D9.
- **Create/Change:** `.github/workflows/ci.yml` runs on `push` to `main` and on `pull_request`, with `permissions: contents: read`. One `quick` job on `ubuntu-24.04` checks out, installs the toolchain from `rust-toolchain.toml`, installs nextest at the version pinned in `tools/versions.toml` (through `cargo xtask install-tools cargo-nextest` or a SHA-pinned installer action), caches by `Cargo.lock` and toolchain, and runs `cargo xtask quick`. Every `uses:` is pinned by a full commit SHA with a `# vX.Y.Z` comment. There is no Windows job (D2).
- **Accept:** `grep -E 'uses: [^@]+@[0-9a-f]{40}' .github/workflows/ci.yml` matches every `uses:` line. The orchestrator confirms the first CI run is green. The workflow is valid YAML (`python3 -c 'import yaml…'` or `actionlint` if present).
- **Impact:** none.

---

## Lane ARCH: architecture enforcement

### ARCH-01: Policy schema, inventory check, fixture harness
- **Depends on:** FND-05. **Model:** opus. **Size:** M.
- **Read:** fnd §3.2 (rules 1 and 7, example policy), fnd §3.4; `architecture.toml`.
- **Create/Change:** `tools/xtask/src/architecture/{mod.rs,policy.rs,inventory.rs,report.rs}`. `policy.rs` uses serde to parse `architecture.toml` with `deny_unknown_fields` and a schema version check. `inventory.rs` uses `cargo_metadata` and the source tree (`crates/*/Cargo.toml`, `tools/*/Cargo.toml`) to find crates. It fails on a crate that is not in policy, a policy entry with no crate, a crate listed as a member but missing on disk, an empty selector list, and a crate that is in more than one category. `report.rs` emits diagnostics as `ARCH-<rule>: <message> (at <path>)`. `cargo xtask architecture` takes `--manifest-path`, so the harness can point it at fixtures. Fixture harness: `tools/xtask/tests/architecture_fixtures.rs` iterates `tests/architecture-invalid/<case>/`, each a mini workspace plus `expected.txt` (expected rule id and path substring), and asserts a non-zero exit with matching output. Fixtures: `unknown-crate`, `missing-crate`, `empty-selector`, `duplicate-category`.
- **Accept:** `cargo xtask architecture` exits 0 on the repo. Each of the 4 negative fixtures exits non-zero with its expected rule id and path. `quick` now runs `architecture` (D18).
- **Impact:** dependency graph (policy enforcement).

### ARCH-02: Workspace edge allowlist, kinds, closure, paths
- **Depends on:** ARCH-01. **Model:** opus. **Size:** M.
- **Read:** fnd §2.1, fnd §3.2 (rules 2, 3, 6).
- **Create/Change:** `tools/xtask/src/architecture/graph.rs`. Build the resolved graph from `cargo metadata` (`resolve.nodes[].deps[].dep_kinds`). Enforce the direct allowlist for normal edges, the transitive closure (no production crate reaches `test_only` or `tool_only` in normal or build closure), acyclicity, and explicit build-dependency validation (none allowed unless listed). Dev-dependency edges are allowed only toward `test_only` crates and crates in the dependent's normal allowlist. Failures print the full path (`preview-cli -> preview-composition -> preview-testkit`). Fixtures: `reversed-edge` (domain→application), `testkit-in-production` (adapter-http normal-depends on testkit), `transitive-test-only` (via an intermediate crate), `undeclared-build-dep`, `cli-bypasses-composition` (cli→adapter-http).
- **Accept:** all 5 fixtures fail with their rule id and full path. The repo passes. A unit test checks the path rendering format.
- **Impact:** dependency graph.

### ARCH-03: External deps, features, sources, targets
- **Depends on:** ARCH-02. **Model:** opus. **Size:** M.
- **Read:** fnd §3.2 (rules 4 and 5); D2, D12, D25.
- **Create/Change:** `tools/xtask/src/architecture/external.rs` covers the `[external]` table: per crate, the allowed direct third-party crates and, optionally, the allowed features (for example `preview-application = { tokio = ["sync"] }`). It rejects direct dependencies that are not listed, rejects features outside the list (resolved features from metadata), and rejects `source` values other than crates.io unless they are in `[exceptions.sources]` with an ADR id. Target-specific edges are evaluated for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and both musl targets via `--filter-platform`. Windows is not evaluated (D2), and the report states which targets were checked. Fixtures: `domain-uses-tokio`, `application-tokio-rt-feature`, `git-dependency`, `unlisted-external`.
- **Accept:** the 4 fixtures fail with the correct rule. The repo passes. The `architecture` output lists the evaluated targets.
- **Impact:** dependency graph, security boundary (supply chain).

### ARCH-04: Positive fixture and rule-catalog completeness
- **Depends on:** ARCH-03. **Model:** sonnet. **Size:** S. Parallel with ARCH-05.
- **Read:** fnd §3.4.
- **Create/Change:** `tests/architecture-valid/full-topology/` is a mini workspace that uses every allowed edge kind and must pass. `tools/xtask/src/architecture/rules.rs` holds a static catalog of rule ids with a one-line description. A test, `should_have_negative_fixture_when_rule_is_declared`, fails if any rule id lacks a fixture in `tests/architecture-invalid/` whose `expected.txt` names it.
- **Accept:** the positive fixture passes. Deleting any one negative fixture makes the completeness test fail (verified in the test by running against a copied fixture directory with one case removed).
- **Impact:** none.

### ARCH-05: Qualify cargo-bylaw (spike + ADR)
- **Depends on:** ARCH-01. **Model:** opus. **Size:** M. Parallel with ARCH-04.
- **Read:** fnd §3.3 (first paragraph); D14.
- **Create/Change:** a throwaway spike in a scratch directory (not committed) that runs a pinned `cargo-bylaw` against cases covering: macro-generated `use std::fs`, a re-export alias (`pub use std::fs as f`), a `#[cfg(target_os = "linux")]` branch, a trait impl across modules, and generated code. Commit only `doc/adr/NNNN-cargo-bylaw-qualification.md` (the next free number, 0034; Status: Proposed), recording the version tried, each case's result, and the verdict: adopt, or fall back to `syn` per D14. If adopted, also add it to `tools/versions.toml`.
- **Accept:** the ADR exists, is indexed in `doc/adr/README.md`, and lists all 5 cases with observed results. If the tool cannot be installed in the sandbox, the ADR records that as the verdict ("not qualified: unavailable") and the fallback applies.
- **Impact:** none (decision only).

### ARCH-06: Semantic rules: domain purity, authority, isolation
- **Depends on:** ARCH-04, ARCH-05. **Model:** opus. **Size:** L.
- **Read:** fnd §3.3 (rule bullets 1, 2, 3, 6, 8); the cargo-bylaw ADR from ARCH-05; D11, D12.
- **Create/Change:** `tools/xtask/src/architecture/semantic/{mod.rs,paths.rs,rules_purity.rs}`, using either the qualified tool or `syn` (with `full` and `visit` features). Resolve `use` trees and aliases within a crate, and treat any path that resolves to a forbidden root as a hit. Rules:
  - `preview-domain` must not reference `std::{fs,net,process,env,thread}`, `std::time::{SystemTime,Instant}::now`, `tokio`, `axum`, `notify`, `rustix` or `rand`.
  - `std::fs` and `rustix::fs` are allowed only in `preview-adapter-fs` and in `preview-composition/src/bootstrap.rs`.
  - `preview-adapter-http` must not name `preview_adapter_fs` or `notify`.
  - The source has no `unsafe` blocks (defense in depth beside the lint).

  An unparseable file is a failure, never a pass. Fixtures under `tests/architecture-invalid/` (as source snippets in mini crates): `domain-std-fs`, `domain-aliased-fs` (`use std::fs as f; f::read`), `domain-tokio-via-macro-path`, `http-names-fs-adapter`, `ambient-fs-in-application`.
- **Accept:** the 5 fixtures fail with rule id, file and line. The repo passes. A file with a syntax error yields a failure diagnostic, not a pass.
- **Impact:** security boundary (filesystem authority), dependency graph.

### ARCH-07: Semantic rules: hygiene and test quality ⛳
- **Depends on:** ARCH-06. **Model:** opus. **Size:** M.
- **Read:** fnd §3.3 (last bullet), fnd §5.2; Rules for every task, rules 4 and 5.
- **Create/Change:** `tools/xtask/src/architecture/semantic/rules_hygiene.rs`:
  - No `todo!`/`unimplemented!`/`dbg!` outside tests.
  - No `#[ignore]`.
  - No `#[allow(...)]`/`#[expect(...)]`/`#[coverage(off)]` unless the `[exceptions]` entry (path plus lint) cites an existing ADR file.
  - No `std::thread::sleep` or `tokio::time::sleep` in any `tests/` directory or `#[cfg(test)]` module, except inside `preview-testkit/src/poll.rs`.
  - Every `#[test]`/`#[tokio::test]` fn name matches `^should_[a-z0-9_]+_when_[a-z0-9_]+$`, and its body has `// given`, `// when` and `// then` comments (checked from the source span).

  Fixtures: `ignored-test`, `unjustified-allow`, `allow-with-missing-adr`, `sleep-in-test`, `badly-named-test`, `missing-given-when-then`, `todo-in-production`. Rename any existing tests that violate these rules.
- **Accept:** all 7 fixtures fail. The repo passes. `cargo xtask list-checks` lists every rule id from ARCH-01..07.
- **Impact:** none.

### ARCH-08: Compile-fail (`trybuild`) API fixtures
- **Depends on:** FS-02, DOM-01. **Model:** sonnet. **Size:** S. Parallel with FS-04.
- **Read:** fnd §3.1.
- **Create/Change:** `crates/preview-domain/tests/compile_fail.rs` plus `tests/compile-fail/*.rs` with `.stderr`: constructing `SitePath` without `decode` (private fields), mutating `Revision` downward, and building a `SiteState` with a revision that does not come from `next()`. `crates/preview-adapter-fs/tests/compile_fail.rs`: constructing `RootDir` from a raw fd or path outside `RootDir::open_root`, and obtaining an `OwnedFd` from `OpenedFile` mutably.
- **Accept:** `cargo nextest run -p preview-domain -p preview-adapter-fs` passes with each fixture failing to compile for the stated reason (pinned `.stderr`). Making a field `pub` breaks the corresponding fixture (state this check in the commit).
- **Impact:** public API.

### ARCH-09: Semantic rules: commands, queries, observability
- **Depends on:** APP-04. **Model:** opus. **Size:** M. Parallel with FS-01.
- **Read:** fnd §3.3 (rule bullets 4, 5 and 7), fnd §2.4; ADR 0035, ADR 0036; the module layout APP-01..APP-04 created in `crates/preview-application/src/{commands,queries,ports}/`.
- **Create/Change:** `tools/xtask/src/architecture/semantic/rules_cqrs.rs` on the ARCH-06 collector, with a rule row and negative fixtures for each rule:
  - `ARCH-command-isolation`: a module under `preview-application/src/commands/` names nothing under `crate::queries` and no HTTP DTO (`axum`, `http`, `preview_adapter_http`), also through re-exports. Fixtures `command-imports-query`, `command-names-http-dto`.
  - `ARCH-query-purity`: a module under `src/queries/` names nothing under `crate::commands`, takes no `&mut` receiver or parameter of a domain state type, and calls no `ReloadState` publish method. Fixtures `query-calls-command`, `query-mutates-state`.
  - `ARCH-observability-isolation`: domain code names no `tracing`/`log` item, and an application value returned by an instrumentation call (a `tracing` or `log` macro, or a module named `observability`/`metrics`) never flows into a branch condition or a returned decision; checked conservatively from the syntax (a call inside an `if`/`match` scrutinee or a `return`). Fixture `instrumentation-drives-decision`.

  The module selectors (`commands/`, `queries/`) are checked to exist like the ARCH-07 selectors (`ARCH-missing-selector`). The positive fixture gains a command, a query and a traced command that pass.
- **Accept:** each fixture fails with its rule, file and line; the repo and the positive fixture pass; `list-checks` lists the new rule ids with owner ARCH-09; renaming `src/queries/` in a copy of the positive fixture fails with `ARCH-missing-selector`.
- **Impact:** port contract, dependency graph.

---

## Lane DOM: pure domain types and state machines

### DOM-01: `SitePath`: URL decoding and component validation
- **Depends on:** OPS-01. **Model:** opus. **Size:** M. Parallel with SEC-01.
- **Read:** plan §5.2 (rules 1, 2, 6, last paragraph), plan §12 "Pure unit tests" bullet 1; D11.
- **Create/Change:** `crates/preview-domain/src/{lib.rs,path.rs}`. `SitePath` holds validated `Vec<Component>` and has private fields. `decode_request_path(&str) -> Result<SitePath, PathRefusal>` decodes percent-encoding exactly once (`percent-encoding` crate, D12). It rejects malformed `%`, NUL, `\`, an encoded `/` or `\` (`%2f`, `%5c`), `.` and `..` (literal or encoded), empty interior components and invalid UTF-8. It preserves a trailing-slash flag. `PathRefusal` variants are `Malformed` and `Traversal`. Unit tests cover each item of plan §12 bullet 1, including `%252e%252e` (decoded once, so it becomes the literal name `%2e%2e`), a valid space (`%20`) and Unicode (`caf%C3%A9`).
- **Accept:** `cargo nextest run -p preview-domain path::` passes. Negative cases include `should_refuse_when_path_contains_encoded_dot_dot` and `should_refuse_when_path_contains_encoded_slash`.
- **Impact:** security boundary, public API (domain).

### DOM-02: Component eligibility predicate
- **Depends on:** DOM-01. **Model:** opus. **Size:** S. Parallel with SEC-02.
- **Read:** plan §5.2 (rules 3–5), plan §8.1 step 2, plan §15 bullets 3–4; D11.
- **Create/Change:** `crates/preview-domain/src/eligibility.rs`:
  - `fn component(name, kind: Dir|File, level: Root|Nested) -> Eligibility { Eligible, Denied(DenyReason) }` (the level is needed for the root-only `Reserved` rule) with reasons `Hidden`, `ExcludedDirectory` (exact `node_modules`, `vendor`, `.venv`, `__pycache__`), `EditorArtifact` (`*~`, `.*.swp`, `.*.swo`, `#*#`, `*.tmp`, `*.bak`, `*.orig`, `*.rej`, `4913`) and `Reserved` (`__site_preview` at root).
  - `fn path(&SitePath) -> Eligibility`, which applies directory rules to all but the last component.

  Serving (APP-03) and watching (FS-05, DOM-05) use this one predicate.
- **Accept:** tests include `should_allow_component_when_name_only_contains_vendor_substring` (`my-vendor-artwork`), `should_deny_when_any_component_is_hidden` (`.well-known/x`), `should_deny_when_file_ends_with_tilde`, and `should_deny_when_root_component_is_reserved`.
- **Impact:** security boundary.

### DOM-03: `SiteState`, `InstanceId`, `Revision`, domain events
- **Depends on:** OPS-01. **Model:** sonnet. **Size:** S. Parallel with DOM-01, DOM-02, SEC-03.
- **Read:** plan §7.1; fnd §2.2 (Domain bullet), fnd §2.4.
- **Create/Change:** `crates/preview-domain/src/{site_state.rs,events.rs}`:
  - `InstanceId` is validated: `[A-Za-z0-9-]{1,64}`, constructed from caller-supplied parts (pid, startup nanos) because the domain has no clock (D11).
  - `Revision(u64)` has `ZERO` and `next()`; `next()` saturates and returns a `RevisionExhausted` refusal at `u64::MAX`.
  - `SiteState { instance_id, revision }` provides `initial(id)`, `advanced()`, and `differs(&other)` (instance or revision).
  - `DomainEvent::{RevisionPublished(SiteState), WatcherDegraded{reason}, WatcherRecovered}`.
- **Accept:** tests cover plan §12 unit bullets 5–6 at the domain level: `should_report_difference_when_instance_changes_with_same_revision` and `should_report_difference_when_revision_advances`.
- **Impact:** model, public API (domain).

### DOM-04: Global trailing-edge debounce machine
- **Depends on:** DOM-03. **Model:** opus. **Size:** M. Parallel with DOM-05, DOM-06.
- **Read:** plan §8.3; D22.
- **Create/Change:** `crates/preview-domain/src/{time.rs,debounce.rs}`. `Tick` is an opaque monotonic millisecond instant supplied by the caller. `DebounceWindow` is validated against D22 (10 ms–10 s). `Debouncer` has `record(now) -> Deadline`, `poll(now) -> Poll::{Idle, Wait(Deadline), Fire}`, and `mark_overflow(now)`, which forces dirty state so the next fire also requests a full rescan. `Fire` returns `Settled { rescan: bool }` and clears dirty. There is no internal clock.
- **Accept:** `should_fire_once_when_burst_settles`, `should_extend_deadline_when_event_arrives_before_deadline`, `should_not_fire_when_no_event_recorded`, `should_request_rescan_when_overflow_marked`, and `should_refuse_window_when_zero_or_above_ten_seconds`. A proptest (any event schedule) checks that the number of fires is at most the number of quiet gaps ≥ window and is at least 1 if any event occurred.
- **Impact:** model.

### DOM-05: Change classification
- **Depends on:** DOM-02. **Model:** sonnet. **Size:** S. Parallel with DOM-04, DOM-06.
- **Read:** plan §8.2.
- **Create/Change:** `crates/preview-domain/src/change.rs` defines `ChangeKind::{Create, Modify, Remove, Rename, DirectoryChanged, AccessOnly, MetadataOnly, Ambiguous}` and `ChangeEvent { kind, path: RelPath }` (`RelPath` is root-relative components from the adapter). `classify(&ChangeEvent) -> Classification::{Relevant{dir_changed: bool}, Ignored(IgnoreReason), Rescan}`. Ineligible components (via DOM-02) are `Ignored`. Editor temp names are `Ignored`. `AccessOnly` and `MetadataOnly` are `Ignored`. `Ambiguous` is `Rescan`.
- **Accept:** `should_ignore_when_change_is_inside_dot_git`, `should_ignore_when_file_is_editor_swap`, `should_be_relevant_when_html_is_modified`, `should_rescan_when_event_is_ambiguous`, and `should_flag_dir_change_when_directory_created`.
- **Impact:** model.

### DOM-06: Watch-set reconciliation
- **Depends on:** DOM-02. **Model:** sonnet. **Size:** S. Parallel with DOM-04, DOM-05.
- **Read:** plan §8.1 (steps 5–6, "New directories").
- **Create/Change:** `crates/preview-domain/src/watch_set.rs`. `WatchSet` (a `BTreeSet<RelPath>`) has `reconcile(desired) -> WatchDiff { add, remove }`, `apply(diff)`, and `nearest_existing_ancestor(path)`. It is pure, so tests can enumerate cases.
- **Accept:** `should_add_only_new_directories_when_reconciling`, `should_remove_descendants_when_directory_removed`, and a proptest showing that `apply(reconcile(d))` yields exactly `d`.
- **Impact:** model.

### DOM-07: Host authority and SSE Origin policy
- **Depends on:** OPS-01. **Model:** opus. **Size:** M. Parallel with DOM-04..06.
- **Read:** plan §5.4 (whole); D20.
- **Create/Change:** `crates/preview-domain/src/request_policy.rs` holds both policies:
  - `HostPolicy::new(bound: BoundAddr { ip, port, loopback, wildcard }, allow_remote)` with `check(host_header: Option<&str>) -> Allow | Reject(HostReject)`. It parses `host[:port]` and bracketed IPv6, rejects a missing Host, DNS names other than `localhost` (and `localhost` only when loopback), mismatched ports and userinfo.
  - `OriginPolicy::check(origin: Option<&str>, sec_fetch_site: Option<&str>, allowed_authorities) -> Allow | Reject`. It rejects an Origin whose authority is not allowed, and rejects `Sec-Fetch-Site: cross-site`. A request with neither header is allowed (agents and curl).
- **Accept:** `should_reject_when_host_is_dns_name` (`evil.example:PORT`), `should_reject_when_port_mismatches`, `should_allow_localhost_alias_when_bound_to_loopback`, `should_reject_localhost_when_allow_remote_on_wildcard`, `should_allow_numeric_ip_when_wildcard_and_allow_remote`, `should_reject_when_origin_is_cross_site`, and `should_allow_sse_when_no_browser_headers`.
- **Impact:** security boundary.

### DOM-08: Injection eligibility policy
- **Depends on:** DOM-03. **Model:** sonnet. **Size:** S. Parallel with DOM-07.
- **Read:** plan §6 steps 1–3; D21.
- **Create/Change:** `crates/preview-domain/src/injection.rs`. The input is `InjectionInput { method: Get|Head, status_ok, media: Html|Other, fetch_dest: Option<FetchDest>, size, utf8_ok }`; the output is `decide(...) -> Inject | Skip(SkipReason)`. It injects only for GET + OK + Html + (dest is `document` or `iframe`, or dest is absent) + size ≤ 8 MiB + `utf8_ok`. `SkipReason::TooLarge` and `NotUtf8` carry a "warn" flag. The no-header fallback is documented in a doc comment.
- **Accept:** `should_skip_when_fetch_dest_is_empty`, `should_skip_when_method_is_head`, `should_skip_with_warning_when_html_exceeds_eight_mebibytes`, `should_inject_when_dest_missing_and_html_get`, and `should_skip_when_media_is_svg`.
- **Impact:** model.

### DOM-09: Lifecycle and watcher-health state machines
- **Depends on:** DOM-03. **Model:** opus. **Size:** M. Parallel with DOM-08.
- **Read:** fnd §6.1 item 1 (states and invariants); plan §8.4; plan §9 (diagram).
- **Create/Change:** `crates/preview-domain/src/{lifecycle.rs,watcher_health.rs}`:
  - `Lifecycle` has states `Stopped`, `Starting`, `Serving`, `Stopping` and `Failed`, and inputs `Start`, `Bound`, `BindFailed`, `WatcherReady`, `WatcherFailed`, `Stop` and `Stopped`. Transitions return `Transition { next, effects }` or a `Refusal`. `Stop` is idempotent. `WatcherFailed` during `Starting` still reaches `Serving`, with the effect `Degraded`.
  - `WatcherHealth::{Starting, Watching{dirs}, Degraded{reason}}` has transitions and emits `DomainEvent`s.
- **Accept:** `should_reach_serving_degraded_when_watcher_fails_during_start`, `should_be_noop_when_stop_called_twice`, `should_refuse_serving_when_bind_failed`, and a proptest over random input sequences checking that no state is reachable except through declared transitions and that `Failed` is terminal.
- **Impact:** model.

### DOM-10: Change set: kinds, URL paths, cap, full fallback
- **Depends on:** DOM-04, DOM-05. **Model:** opus. **Size:** M. Parallel with DOM-07..09.
- **Read:** D29, D30 (ADRs 0030, 0031); plan §8.3; `crates/preview-domain/src/{debounce.rs,change.rs}`.
- **Create/Change:** `crates/preview-domain/src/change_set.rs`:
  - `AssetKind::{Css, Js, Html, Asset, Other}` is derived from the final component's extension per D30.
  - `ChangedPath { url_path: String, kind }` builds a percent-encoded root-relative URL path (`/css/site.css`) from the validated components.
  - `ChangeSet { changes: BTreeSet<ChangedPath>, full: bool, reason: Option<FullReason> }` has `accumulate(&ChangeEvent, Classification)`, `mark_full(reason)` and `take()`. `full` is forced by: more than 64 entries (`CapExceeded`), `Rescan`/overflow (`Rescan`), `dir_changed` (`DirectoryChanged`), and an `Html` or `Other` kind (`DocumentChanged`). A `Remove` of a css, js or asset path also forces full (`Removed`), because no in-place swap can apply. Duplicate events for one path yield one entry.

  Extend `Debouncer` so the fire carries the accumulated `ChangeSet`: `Poll::Fire(Settled { rescan, changes })`, with `record(now, &ChangeEvent, Classification)`. The fields of `ChangeSet` are private (`changes()`, `is_full()`, `reason()`), so a hot set can never exceed the cap.
- **Accept:** `should_mark_full_when_html_changes`, `should_keep_hot_when_only_css_changes`, `should_mark_full_when_more_than_64_paths_change`, `should_mark_full_when_overflow_rescan`, `should_deduplicate_when_same_path_changes_twice`, `should_percent_encode_url_path_when_name_has_space`, and a proptest: `full == false` implies every change kind is in {Css, Js, Asset} and the count is ≤ 64.
- **Impact:** model, public API (domain).

---

## Lane APP: application commands, queries and ports

### APP-01: Application layout and ports
- **Depends on:** DOM-02, DOM-03. **Model:** opus. **Size:** M. Parallel with TEST-02.
- **Read:** fnd §2.2 (Application bullet), fnd §2.3, fnd §2.4; plan §5.3 (first paragraph); D11, D12.
- **Create/Change:** `crates/preview-application/src/{lib.rs,ports/mod.rs,ports/site_content.rs,ports/change_source.rs,ports/clock.rs,commands/mod.rs,queries/mod.rs,events.rs}`:
  - `SiteContent` (sync, `Send + Sync`): `open(&SitePath) -> Result<Opened, ContentRefusal>`. `Opened` is `File(Box<dyn ContentFile>)` or `Directory { index: Option<Box<dyn ContentFile>> }`. `ContentFile` is `Read + Send` with `len()`. `ContentRefusal` is `NotFound`, `Forbidden` or `Io(kind)`, with no absolute paths.
  - `ChangeSource`: `next() -> ChangeBatch::{Events(Vec<ChangeEvent>), Overflow, Failed(reason), Closed}`, documented for ordering and cancellation.
  - `Clock`: `now() -> Tick`.

  Doc comments on each port state its contract, which TEST-04 and TEST-05 test.
- **Accept:** `cargo doc -p preview-application --no-deps` succeeds without warnings. `architecture` passes. No adapter types (`axum`, `notify`) appear in signatures; ARCH-06 enforces this for imports.
- **Impact:** port contract, public API.

### TEST-02: Property-test conventions and seed persistence
- **Depends on:** DOM-02, DOM-04. **Model:** sonnet. **Size:** S. Parallel with APP-01.
- **Read:** fnd §5 (Property row), fnd §5.2 (4th bullet).
- **Create/Change:** a `proptest` dev-dependency in the workspace. `crates/preview-domain/proptest-regressions/` is committed. The `proptest` config uses `failure_persistence` pointing at source-adjacent files, and the `PROPTEST_CASES` default is set by the xtask unit-tests stage for `ci`. Add properties in `crates/preview-domain/tests/properties.rs`: decoding never yields a `.`/`..` component or a `/` within a component; any eligible `SitePath` has no hidden component; the debounce property moves here from DOM-04 if it was inline.
- **Accept:** the properties pass. A deliberately inserted failing property (in a test of the convention, then reverted, as described in the commit body) produces a regression file under `proptest-regressions/`. `.gitignore` does not ignore that directory.
- **Impact:** none.

### TEST-03: Testkit fakes
- **Depends on:** APP-01. **Model:** sonnet. **Size:** M.
- **Read:** fnd §2.2 (Testkit bullet), fnd §5.1; `crates/preview-application/src/ports/`.
- **Create/Change:** `crates/preview-testkit/src/{lib.rs,site.rs,clock.rs,change_source.rs,ids.rs}`:
  - `InMemorySite` implements `SiteContent`. It holds a tree of dirs, files and symlink markers, and `open` refuses any path crossing a symlink marker exactly as D3 requires.
  - `FakeClock` has `advance(ms)`.
  - `ScriptedChangeSource` is fed through a handle: `push(events)`, `overflow()`, `fail(reason)`, `close()`.
  - `FixedInstanceIds` provides deterministic instance ids.

  `architecture.toml` gets the testkit's `[external]` entries.
- **Accept:** unit tests for each fake, including `should_refuse_when_path_crosses_symlink_marker`. The architecture check confirms the testkit is absent from production closure.
- **Impact:** none.

### APP-02: `ReloadState` latest-state channel
- **Depends on:** TEST-03, DOM-10. **Model:** sonnet. **Size:** S. Parallel with APP-03.
- **Read:** plan §7.1 (last paragraph), plan §7.2 (first sentence); D12, D29.
- **Create/Change:** `crates/preview-application/src/reload_state.rs`. `ReloadState` wraps `tokio::sync::watch<Published>` (`Published { state: SiteState, changes: ChangeSet }`; the initial value has empty changes and `full = true`) with `new(instance)`, `current()` (snapshot), `subscribe() -> StateSubscription` (subscribe before reading: `subscription.initial()` returns the value observed at subscribe time), and `publish_next(changes) -> Result<SiteState, RevisionExhausted>`. Publishing is monotonic only. `current()` returns the `SiteState` alone, so the HTML snapshot and baseline are unchanged (D29).
- **Accept:** `should_deliver_latest_only_when_reader_is_slow` (publish 3 times, reader sees the latest, not a queue), `should_observe_publish_after_subscribe_when_published_between_subscribe_and_read`, and `should_never_decrease_revision_when_publishing` (proptest).
- **Impact:** model, port contract.

### APP-03: `ResolveRequest` query with render snapshot
- **Depends on:** TEST-03, DOM-02. **Model:** opus. **Size:** M. Parallel with APP-02.
- **Read:** plan §5.1 (table rows 1–3, 6), plan §5.2 rule 5, plan §7.3 ("Snapshot ordering"); D11.
- **Create/Change:** `crates/preview-application/src/queries/resolve_request.rs`. `ResolveRequest { raw_path }` is executed against `&dyn SiteContent` and `&ReloadState`, or a `StateSnapshot` provider until APP-02 merges (a trait-free closure is acceptable). Order: decode (DOM-01), then eligibility (DOM-02), then **snapshot `SiteState`**, then `open`. Results: `Resolved::{File{file, snapshot, name}, RedirectToSlash, NotFound, Forbidden(reason), BadRequest}`, plus `Unavailable(kind)` for an I/O failure (D38). A directory with an index is served when the path has a trailing slash. A directory without a trailing slash yields `RedirectToSlash`. A directory without an index yields `NotFound` (never a listing). A refused path yields `Forbidden` or `NotFound`, without content.
- **Accept:** over `InMemorySite`: `should_redirect_when_directory_requested_without_slash`, `should_return_not_found_when_directory_lacks_index`, `should_forbid_when_component_hidden`, `should_snapshot_state_before_open_when_resolving_file` (the fake publishes during open and the returned snapshot is the pre-open value), and `should_return_bad_request_when_encoding_malformed`.
- **Impact:** port contract, security boundary.

### APP-04: Change commands: record, settle, overflow, health ⛳
- **Depends on:** APP-02, DOM-04, DOM-05, DOM-09, DOM-10. **Model:** opus. **Size:** M.
- **Read:** plan §8.2, §8.3, §8.4; fnd §2.4; D29.
- **Create/Change:** `crates/preview-application/src/commands/{record_change.rs,settle.rs,watcher_health.rs}`. `ChangeCoordinator` owns `Debouncer` and `WatcherHealth` and has:
  - `record(batch, now) -> RecordOutcome { next_deadline, needs_reconcile }`, which classifies with DOM-05 and ignores irrelevant events without touching the debouncer.
  - `overflow(now)`.
  - `settle(now) -> Option<Settled { state: SiteState, rescan, changes: ChangeSet }>`, which publishes the state together with its change set through `ReloadState` and emits `RevisionPublished`.
  - `watcher_failed(reason)` and `watcher_ready(dirs)` (recovery, D39), which emit health events.

  Events go out through a returned `Vec<DomainEvent>`; there is no bus (fnd §2.4).
- **Accept:** with `FakeClock` and `ScriptedChangeSource`: `should_publish_one_revision_when_burst_settles`, `should_publish_css_change_set_when_only_stylesheet_edited`, `should_not_publish_when_only_dot_git_changes`, `should_publish_with_rescan_when_overflow_signalled`, and `should_emit_degraded_when_watcher_fails`. Every command has a test entry point (fnd §5.2 bullet 3).
- **Impact:** model, port contract.

---

## Lane FS: path security, secure open, watcher adapter

### FS-01: Root capability: `openat` per component
- **Depends on:** APP-01. **Model:** opus. **Size:** M. Parallel with APP-02..04.
- **Read:** plan §5.3 (whole); ADR 0004 (D3); fnd §7 "Filesystem security" paragraph.
- **Create/Change:** `crates/preview-adapter-fs/src/{lib.rs,capability.rs}`, using `rustix` (feature `fs`) in the external allowlist:
  - `RootDir` holds an `OwnedFd` and is constructed only by `RootDir::open_root(canonical: &Path)`, the single ambient-authority entry point.
  - `RootDir::open(&SitePath) -> Result<Opened, OpenError>` opens every intermediate component with `openat(dir, name, O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC)`, then the final component with `O_RDONLY|O_NOFOLLOW|O_CLOEXEC`, and `fstat` classifies the result as a regular file, a directory or other.
  - `ELOOP`/`ENOTDIR` become `Forbidden(Symlink)`. Anything that is not a regular file or directory becomes `Forbidden(NotRegular)`. `ENOENT` becomes `NotFound`.

  Update ADR 0004's Consequences with a link to `capability.rs` (no status change).
- **Accept:** integration tests in `crates/preview-adapter-fs/tests/capability.rs` over `tempfile`: `should_refuse_when_final_component_is_symlink_to_outside`, `should_refuse_when_middle_component_is_symlink`, `should_refuse_when_symlink_points_inside_root`, `should_open_when_path_is_regular_file`, and `should_refuse_when_target_is_fifo`. `grep -rn 'canonicalize\|starts_with' crates/preview-adapter-fs/src/capability.rs` finds nothing.
- **Impact:** security boundary.

### FS-02: `SiteContent` adapter: index resolution, errors
- **Depends on:** FS-01, APP-03. **Model:** opus. **Size:** S.
- **Read:** plan §5.1 (rows 1–2), plan §5.3 (last sentence of paragraph 2); `ports/site_content.rs`.
- **Create/Change:** `crates/preview-adapter-fs/src/site_content.rs` implements `SiteContent for RootDir`. For a directory, it opens `index.html` relative to the directory fd (with no reopen by path). `ContentFile` wraps `std::fs::File::from(OwnedFd)` with `len` from `fstat`. Errors map to `ContentRefusal` with no path text.
- **Accept:** `should_return_directory_with_index_when_index_present`, `should_refuse_when_index_html_is_symlink`, and `should_not_leak_absolute_path_when_io_error` (Debug/Display of the error contains no temp-dir path).
- **Impact:** port contract, security boundary.

### FS-03: Root opening and reserved-path check
- **Depends on:** FS-01. **Model:** sonnet. **Size:** S. Parallel with FS-02.
- **Read:** plan §3 (bullets "Resolve the requested root…"), plan §5.3 (paragraph 1), plan §6 (last paragraph).
- **Create/Change:** `crates/preview-adapter-fs/src/root.rs`. `open_site_root(requested: &Path) -> Result<SiteRoot { dir: RootDir, display_path }, RootError>` canonicalizes once (a symlinked root is allowed), requires a readable directory, and fails with `RootError::ReservedPathConflict` if `__site_preview` exists as any entry type (`statat` with `AT_SYMLINK_NOFOLLOW`). Errors are `NotFound`, `NotDirectory`, `NotReadable` and `ReservedPathConflict`, each with an actionable message.
- **Accept:** `should_fail_with_reserved_conflict_when_site_preview_dir_exists`, `should_fail_when_root_is_file`, `should_open_when_root_given_via_symlink`, and `should_fail_when_root_unreadable` (chmod 000, skipped only if running as root, where the test asserts the effective uid and documents it, never `#[ignore]`).
- **Impact:** security boundary.

### TEST-04: `SiteContent` contract harness
- **Depends on:** FS-02, TEST-03. **Model:** opus. **Size:** M. Parallel with FS-04.
- **Read:** fnd §5.1; fnd §12 phase 2 (first acceptance clause).
- **Create/Change:** `crates/preview-testkit/src/contracts/site_content.rs` has `fn site_content_contract<F: SiteContentFactory>()`. The factory builds a site from a declarative tree that includes symlink entries. Contract cases cover: file, nested index, missing index, hidden, symlink final, symlink middle, in-root symlink, non-regular, and error text without paths. It is run from `crates/preview-testkit/tests/site_content_fake.rs` (`InMemorySite`) and `crates/preview-adapter-fs/tests/site_content_contract.rs` (`RootDir` over tempdir). Negative control: `BrokenFollowsSymlinks`, a test-local fake that follows in-root symlinks, makes the harness fail at the case `in_root_symlink`. Use the same assertion path for a test-local wrapper around `RootDir` that maps `Forbidden` to file contents.
- **Accept:** both real suites pass. Both broken implementations fail the same named case, asserted with a `should_fail_contract_when_*` test that captures the harness result.
- **Impact:** port contract.

### FS-04: Concurrent symlink-swap race test
- **Depends on:** FS-02. **Model:** opus. **Size:** S. Parallel with TEST-04, ARCH-08.
- **Read:** plan §5.3 (last paragraph).
- **Create/Change:** `crates/preview-adapter-fs/tests/symlink_race.rs`. One thread loops (bounded iterations, not time) atomically renaming between a real directory `d/` (containing its own `secret.txt`) and a symlink `d -> /outside` (containing `secret.txt` with a sentinel). Another thread repeatedly calls `RootDir::open("d/secret.txt")`. Assert the sentinel bytes are never returned over N iterations, and that both outcomes (opened inside, forbidden) were observed. The test is in the `os-watcher` nextest group if it needs isolation.
- **Accept:** the test passes repeatedly (`cargo nextest run -p preview-adapter-fs symlink_race --retries 0` run 5 times in the commit's verification). A negative variant that uses `std::fs::File::open(root.join(p))` on the same loop observes the sentinel; it lives under `#[cfg(test)]` as a test that asserts the naive approach is unsafe.
- **Impact:** security boundary.

---

## Lane HTTP: serving, MIME, Host, injection, SSE

### HTTP-01: Router, method gate, errors, internal namespace
- **Depends on:** APP-03. **Model:** sonnet. **Size:** M. Parallel with FS-02.
- **Read:** plan §5.1 (rows "Missing/forbidden", "POST…", "Internal reserved endpoints"; paragraph on `no-store`); fnd §2.2 (HTTP adapter bullet).
- **Create/Change:** `crates/preview-adapter-http/src/{lib.rs,router.rs,errors.rs,state.rs}`. `axum` and `tokio` go in the allowlist. `build_router(AppState { content: Arc<dyn SiteContent>, reload: ReloadState, host_policy… })` returns `Router`. Methods other than GET and HEAD get 405 with `Allow: GET, HEAD`. `/__site_preview/*` is matched before the file fallback and returns 404 for unknown internal paths, never touching disk. Error responses use a tiny fixed `text/plain` body (`404 Not Found`) with `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`. A file stub responds 501 until HTTP-02 (internal only, no `todo!`).
- **Accept:** in-process tests using `tower::ServiceExt::oneshot`: `should_return_405_with_allow_when_method_is_post` (PUT, PATCH, DELETE too), `should_not_touch_disk_when_internal_path_requested` (the `InMemorySite` records zero opens), and `should_set_no_store_when_error_response`.
- **Impact:** public API (HTTP).

### HTTP-02: GET/HEAD file streaming
- **Depends on:** HTTP-01, FS-02. **Model:** sonnet. **Size:** M.
- **Read:** plan §5.1 (rows 4–5, paragraph on streaming and Range), plan §9 (paragraph "Do not spawn…").
- **Create/Change:** `crates/preview-adapter-http/src/files.rs`. It runs `ResolveRequest` in `spawn_blocking`. GET streams the `ContentFile` through a blocking-reader-to-stream bridge (`tokio-util` `ReaderStream` over a `spawn_blocking` pipe, or chunked reads) with `Content-Length` = `len`. HEAD returns the same status and headers with an empty body and no read. Responses carry `Cache-Control: no-store` and no `Accept-Ranges`. A `Range` header is ignored, giving a full 200. Content-Type is `application/octet-stream` until HTTP-03.
- **Accept:** `should_stream_bytes_unchanged_when_get_file` (a 5 MiB random file compares equal byte for byte), `should_return_headers_without_body_when_head`, `should_return_full_200_when_range_requested`, and `should_omit_accept_ranges_when_serving`.
- **Impact:** public API (HTTP).

### HTTP-03: MIME mapping and overrides
- **Depends on:** CLI-01. **Model:** sonnet. **Size:** S. Parallel with HTTP-04.
- **Read:** plan §5.1 (MIME overrides block, HTML charset paragraph), plan §12 unit bullet 3; D21.
- **Create/Change:** `crates/preview-adapter-http/src/mime.rs`, using `mime_guess` plus the four overrides (`.gpx`, `.geojson`, `.kml`, `.wasm`). HTML is labeled `text/html; charset=utf-8` only when the file is valid UTF-8 (checked on the first 8 MiB, else unknown) per D21. JS is `text/javascript; charset=utf-8`, CSS is `text/css; charset=utf-8`, and an unknown extension is `application/octet-stream`. Every response gets `X-Content-Type-Options: nosniff`.
- **Accept:** a table-driven test covering HTML, CSS, JS, SVG, JSON, GPX, GeoJSON, KML, WASM, PDF and WebP (`should_map_content_type_when_extension_is_<x>`), plus `should_omit_utf8_charset_when_html_is_latin1`.
- **Impact:** public API (HTTP).

### HTTP-04: Directory slash redirect and index
- **Depends on:** CLI-01. **Model:** sonnet. **Size:** S. Parallel with HTTP-03.
- **Read:** plan §5.1 (rows 2–3).
- **Create/Change:** `crates/preview-adapter-http/src/directory.rs`. `RedirectToSlash` becomes a `307` to the same path plus `/`, preserving the query string. The `Location` header is percent-encoded from the validated components, never echoed from raw input. An index is served through the HTTP-02 path. A directory without an index returns 404 with no listing.
- **Accept:** `should_redirect_with_307_when_directory_lacks_trailing_slash`, `should_serve_nested_index_when_trailing_slash`, `should_return_404_without_listing_when_index_missing`, and `should_encode_location_when_directory_name_has_space`.
- **Impact:** public API (HTTP).

### HTTP-05: Host authority middleware
- **Depends on:** CLI-01, DOM-07. **Model:** opus. **Size:** S.
- **Read:** plan §5.4 bullets 1–3; D20.
- **Create/Change:** `crates/preview-adapter-http/src/host.rs` is an axum middleware layer applied to every route, internal routes included. It calls `HostPolicy::check`, and a rejection gets `421 Misdirected Request` with the fixed body `Host not allowed`. No CORS headers are ever added. `AppState` carries the `HostPolicy` built by composition from the bound address.
- **Accept:** `should_reject_with_421_when_host_is_rebinding_name`, `should_reject_when_host_port_differs`, `should_accept_when_host_is_bound_ip`, and `should_not_emit_cors_headers_when_origin_sent`.
- **Impact:** security boundary.

### HTTP-06: `lol_html` rewriter
- **Depends on:** E2E-01. **Model:** opus. **Size:** M. Parallel with HTTP-09.
- **Read:** plan §6 (steps 4–6, the script tag block); D21.
- **Create/Change:** `crates/preview-adapter-http/src/inject/{mod.rs,rewrite.rs}`. `lol_html` goes in the allowlist. `rewrite(html: &[u8], tag: &ScriptTag) -> Rewrite::{Injected(Vec<u8>), AlreadyPresent, Failed(reason)}`. It inserts before the real `</body>` end tag, or appends at document end when the body is absent. It detects an existing `<script src="/__site_preview/client.js">` and returns `AlreadyPresent`. `ScriptTag` renders `<script defer src="/__site_preview/client.js" data-instance="…" data-revision="…"></script>`, with attribute values from validated `InstanceId` and `Revision` (escaped regardless).
- **Accept:** `should_insert_before_real_body_end_when_script_contains_fake_body_tag`, `should_insert_before_body_end_when_comment_contains_fake_body_tag`, `should_insert_when_closing_tag_is_mixed_case` (`</BoDy>`), `should_append_when_body_end_absent`, `should_not_duplicate_when_client_already_present`, and `should_return_failed_when_parser_errors` (with a malformed-input case if `lol_html` reports one, else a forced memory-limit setting).
- **Impact:** public API (HTTP response transformation).

### HTTP-07: Injection in GET responses
- **Depends on:** HTTP-06, DOM-08, APP-02. **Model:** opus. **Size:** M.
- **Read:** plan §6 (steps 1–3, 6–7), plan §7.3 ("Snapshot ordering"); D19, D21.
- **Create/Change:** `crates/preview-adapter-http/src/inject/response.rs` maps the request (method, `Sec-Fetch-Dest`) and the resolved file (media, size, UTF-8 check) to `InjectionInput` and calls `decide`. When `Inject` is decided, it reads at most 8 MiB in `spawn_blocking`, rewrites with the snapshot from `Resolved::File`, sets `Content-Length` to the rewritten length and removes `ETag`/`Last-Modified`. When `Skip(warn)` is decided, it streams unchanged and emits warning `html-too-large` or `html-not-utf8` through a `WarningSink` (injected; once per path via a `Mutex<HashSet>`). `Rewrite::Failed` serves the original bytes and warns `html-rewrite-failed`. Source files are only ever opened read-only. HTTP-03 labels HTML `text/html; charset=utf-8` from its bytes alone; this task also drops the charset from the label when a `<meta charset>` names another encoding (ADR 0022), reusing HTTP-03's 8 MiB read rather than reading twice.
- **Accept:** `should_inject_snapshot_revision_when_document_get`, `should_not_modify_when_sec_fetch_dest_empty`, `should_set_content_length_of_rewritten_body_when_injected`, `should_serve_unchanged_and_warn_once_when_html_over_limit` (two requests produce one warning), `should_not_inject_when_head`, `should_not_inject_when_response_is_error`, and `should_leave_source_bytes_unchanged_when_served` (hash before and after).
- **Impact:** public API (HTTP response transformation).

### HTTP-08: CSP restriction warning
- **Depends on:** HTTP-07. **Model:** sonnet. **Size:** S. Parallel with HTTP-10.
- **Read:** plan §6 step 8, plan §15 bullet 1.
- **Create/Change:** `crates/preview-adapter-http/src/inject/csp.rs` has `detect(policies: &[impl AsRef<str>]) -> Option<CspWarning>`, which parses the `content` of every `<meta http-equiv="Content-Security-Policy">` in the head, captured by the rewriter's element handler (several metas and comma-separated policies are all enforced, so a slice, not an `Option`). It warns `csp-blocks-live-reload` once per path when the governing script list (`script-src-elem`, else `script-src`, else `default-src`) or connect list (`connect-src`, else `default-src`) allows none of `'self'`, `*`, `http:` or a host-source (a host-source may match this server, so it never warns; `https:` alone warns, since the server speaks plain `http`; `'strict-dynamic'` makes a script list block). It never alters CSP and never adds headers.
- **Accept:** `should_warn_when_script_src_excludes_self`, `should_warn_when_default_src_none_and_no_connect_src`, `should_not_warn_when_self_allowed`, and `should_preserve_meta_csp_bytes_when_injecting`.
- **Impact:** none.

### HTTP-09: Embedded `client.js` and its route
- **Depends on:** E2E-01. **Model:** sonnet. **Size:** M. Parallel with HTTP-06.
- **Read:** plan §6 (first paragraph), plan §7.3 (whole), plan §7.4 bullet 4; D15, D16.
- **Create/Change:** `crates/preview-adapter-http/assets/client.js` holds the plan §7.3 algorithm. A pure `decide(baseline, kind, state, reloading)` is exported on `globalThis.__sitePreviewTest` only when the script is loaded without `document.currentScript` (that is, under Node), so it is untestable-in-production noise-free. There are no cookies or storage. Add `crates/preview-adapter-http/src/client.rs` (`include_str!`, route `GET/HEAD /__site_preview/client.js`, `text/javascript; charset=utf-8`, `no-store`) and `crates/preview-adapter-http/assets/client.test.mjs` (`node --test`, no dependencies), covering: missing baseline adopts the state; a different revision reloads once; a different instance reloads; `reloading` suppresses a second reload; and equal state does nothing. In this task `decide` returns `'reload' | 'none'`. HTTP-12..16 extend it to dispatch hot updates (D29). A `reload` event with `full: true` or with no `changes` field always reloads. Register the xtask stage `client-js`, which runs `node --test` and fails with the D9 hint if `node` is missing.
- **Accept:** `node --test crates/preview-adapter-http/assets/` passes (5+ cases). `should_serve_embedded_client_when_route_requested`. `grep -E 'localStorage|sessionStorage|document.cookie' client.js` finds nothing (this stays true after HTTP-14, which uses `history.state` per D30).
- **Impact:** public API (HTTP).

### HTTP-10: SSE endpoint
- **Depends on:** HTTP-09, APP-02. **Model:** opus. **Size:** M. Parallel with HTTP-08.
- **Read:** plan §7.2 (whole), plan §7.4 bullets 2–3; D16.
- **Create/Change:** `crates/preview-adapter-http/src/sse.rs`. `GET /__site_preview/events` subscribes first and then sends `event: state` with `id: <rev>` and `data: {"instance_id":…,"revision":…}`, serialized with `serde_json` (allowlisted). It sends `event: reload` for each later changed value seen by the subscription. The keepalive comment interval is injectable (15 s default). Headers are `text/event-stream`, `no-store` and `nosniff`. The stream ends when the `ShutdownSignal` passed in `AppState` fires (its owner, `Shutdown`, is a tokio `watch` flag rather than `tokio-util`'s `CancellationToken`, so no new dependency; dropping the owner counts as firing) or when the `ReloadState` closes. `Last-Event-ID` is accepted and ignored.
- **Accept:** with `tokio::time::pause`: `should_send_state_baseline_when_connected`, `should_send_reload_with_new_revision_when_published`, `should_send_only_latest_when_several_published_before_poll`, `should_send_keepalive_when_interval_elapses`, and `should_end_stream_when_shutdown_token_cancelled`. This is the M2 component-level exit check: one publish produces exactly one `reload`.
- **Impact:** public API (SSE protocol), model.

### HTTP-11: SSE Origin guard and 64-connection limit
- **Depends on:** HTTP-10, DOM-07. **Model:** opus. **Size:** S.
- **Read:** plan §5.4 bullets 4–5, plan §7.4 bullets 1, 3.
- **Create/Change:** `crates/preview-adapter-http/src/sse_guard.rs`. It calls `OriginPolicy::check` before subscribing, and a rejection gets 403. A `tokio::sync::Semaphore` with 64 permits (configurable for tests) is acquired with `try_acquire_owned`; failure gets 503 with `Retry-After: 2`. The permit is moved into the stream so it is dropped on disconnect.
- **Accept:** `should_reject_with_403_when_origin_cross_site`, `should_accept_when_no_origin_headers`, `should_return_503_when_limit_reached` (limit set to 2), and `should_release_permit_when_client_disconnects` (drop the body, then a new connection succeeds).
- **Impact:** security boundary.

### HTTP-12: SSE `reload` payload: changes, `full`, gap rule
- **Depends on:** HTTP-11. **Model:** opus. **Size:** M.
- **Read:** D29, D30; plan §7.2; `crates/preview-adapter-http/src/sse.rs`.
- **Create/Change:** `crates/preview-adapter-http/src/sse.rs`: the `reload` event data becomes `{"instance_id":…,"revision":…,"changes":[{"path":"/css/site.css","kind":"css"}],"full":false}`. The `state` event keeps its plan §7.2 shape. Each connection tracks `last_sent_revision`. If the new revision ≠ `last_sent + 1` (an intermediate value was coalesced away), send `full: true` with `changes: []` (D30 gap rule). Serialize through a dedicated `ReloadPayload` DTO in the adapter; domain types carry no serde (fnd §2.2). The README protocol section documents the schema (DOC-04).
- **Accept:** `should_send_changes_when_single_css_revision_published`, `should_send_full_when_revisions_coalesced` (publish 2 before polling: one event, `full: true`), `should_send_full_when_change_set_full`, and `should_keep_state_event_shape_when_connected`.
- **Impact:** public API (SSE protocol), model.

### HTTP-13: Client: CSS hot swap
- **Depends on:** HTTP-12. **Model:** sonnet. **Size:** M. Parallel with HTTP-14.
- **Read:** D29, D30; `crates/preview-adapter-http/assets/client.js`.
- **Create/Change:** `client.js` handles a `reload` with `full: false` where every change has kind `css`. For each changed path, it finds `link[rel~=stylesheet]` elements whose resolved same-origin `href` pathname equals the path, **or** whose `sheet` imports it through a same-origin `CSSImportRule` (best-effort walk; a `SecurityError` counts as no match). For each match it clones the link with `?__sp=<revision>` (preserving other query parameters), inserts the clone after the old link, waits for `load` (or `error`), and then removes the old node, so there is no unstyled flash. If any changed css path has no matching link, it does a full reload. It updates `baseline.revision` after a successful hot apply, so a later `state` event comparison stays correct. Pure helpers (`matchStylesheetHref`, `cacheBustUrl`) are exported to the Node test hook. Extend `client.test.mjs` with a minimal fake-document fixture (no dependencies).
- **Accept:** in `node --test`: `should_cache_bust_href_when_query_present`, `should_match_link_when_href_is_relative`, `should_request_full_reload_when_no_stylesheet_matches`, and `should_advance_baseline_when_hot_applied`. Browser proof comes in E2E-09.
- **Impact:** public API (client behavior).

### HTTP-14: Client: JS reload with scroll restore
- **Depends on:** HTTP-12. **Model:** sonnet. **Size:** S. Parallel with HTTP-13.
- **Read:** D29, D30 (scroll and storage rules).
- **Create/Change:** in `client.js`, a `reload` with `full: false` containing a `js` change that is not accepted (HTTP-15) does the following before `location.reload()`: if `history.state` is `null` or a plain object, it calls `history.replaceState({...state, __sitePreview: {scrollX, scrollY, revision}}, "")`; otherwise (the site stores a non-object state) it reloads without restore. On startup the client checks `history.state?.__sitePreview`, restores the scroll after `load` (`scrollTo`, retried once after the first animation frame), and removes the key with `replaceState`. It never touches the URL, `sessionStorage`, `localStorage` or cookies. A mixed css+js change set takes the JS path, which subsumes CSS.
- **Accept:** in `node --test` with a fake `history`: `should_store_scroll_in_history_state_when_js_changes`, `should_preserve_existing_state_keys_when_storing_scroll`, `should_skip_restore_when_history_state_is_not_object`, and `should_remove_marker_when_scroll_restored`. The `grep` storage check from HTTP-09 still finds nothing.
- **Impact:** public API (client behavior).

### HTTP-15: Client: opt-in JS hot-accept API
- **Depends on:** HTTP-14. **Model:** opus. **Size:** M. Parallel with HTTP-16.
- **Read:** D29, D30.
- **Create/Change:** `client.js` exposes `window.__sitePreview = { accept(path, handler), version: 1 }` (path is a root-relative URL path; a later call replaces the handler for that path). A js-only, non-full change set in which **every** changed js path has a handler calls each handler with `{ path, url: cacheBusted, revision }`, so the page can `await import(url)`. A missing handler, a thrown error, a rejected promise or a timeout (5 s) leads to the HTTP-14 reload path. The baseline advances only after every handler succeeds. Document the API in a doc comment block at the top of `client.js` (DOC-04 copies it into the README).
- **Accept:** in `node --test`: `should_call_handler_with_cache_busted_url_when_path_accepted`, `should_reload_when_any_changed_js_path_unaccepted`, `should_reload_when_handler_rejects`, and `should_not_advance_baseline_when_handler_fails`.
- **Impact:** public API (page-facing JS API).

### HTTP-16: Client: asset cache-bust
- **Depends on:** HTTP-13. **Model:** sonnet. **Size:** S. Parallel with HTTP-15.
- **Read:** D29, D30.
- **Create/Change:** for a non-full change set of `asset` kind (optionally with css), `client.js` rewrites matching same-origin `img[src]`, `img[srcset]` and `source[srcset]` candidates, and `link[rel~=icon]`, with `?__sp=<revision>`. It re-swaps (HTTP-13 mechanism) every same-origin stylesheet whose rules' `cssText` references the path in a `url(...)` (best-effort; an unreadable sheet means no match). If any changed asset path matches nothing (for example, a GPX fetched by script), it does a full reload.
- **Accept:** in `node --test`: `should_rewrite_img_src_when_asset_matches`, `should_rewrite_only_matching_srcset_candidate_when_asset_changes`, and `should_full_reload_when_asset_unreferenced`.
- **Impact:** public API (client behavior).

---

## Lane CLI: composition, flags, lifecycle, JSON, signals, `--open`

### CLI-01: Thin vertical slice: `serve` a directory
- **Depends on:** HTTP-02, FS-03. **Model:** opus. **Size:** M.
- **Read:** plan §3 ("Command" and "Human startup output"), plan §9 (diagram); fnd §2.2 (Composition and CLI bullets); D4, D27.
- **Create/Change:**
  - `crates/preview-composition/src/{lib.rs,config.rs,bootstrap.rs,server.rs}`. `ServeConfig { root, host, port }`. `bootstrap.rs` is the only ambient-authority site: it calls `open_site_root`. `server.rs` has `run(config, out: &mut dyn StartupReporter) -> Result<(), StartupError>`: it builds a multi-thread runtime, binds a `TcpListener` (default `127.0.0.1:0`), builds the router with `ReloadState::new(instance id from pid + startup nanos)`, reports the URL, and serves until Ctrl-C (a minimal handler; CLI-05 replaces it).
  - `crates/preview-cli/src/{main.rs,args.rs,output.rs}`: `serve [directory]` (default `.`) and human output per plan §3, with `Watching:` reading `not yet available`.
  - `tests/fixtures/site/`: `index.html`, `styles.css`, `app.js`, `routes.gpx`, `data.json`, `images/sample.svg`, `guide/index.html`.
  - `crates/preview-cli/tests/serve_slice.rs` spawns the binary against a temp copy of the fixture, parses `Local:` from output with a bounded wait, GETs `/` and `/styles.css` with a minimal blocking HTTP client dev-dependency (allowlisted, justified in the commit), then kills the child through a drop guard.
- **Accept:** `should_serve_index_when_started_on_fixture` and `should_serve_stylesheet_bytes_unchanged_when_requested` pass. `cargo run -p preview-cli -- serve tests/fixtures/site` serves in a browser. The fixture tree hash is unchanged after the test. `architecture` passes: the cli depends only on composition.
- **Impact:** public API (CLI), dependency graph.

### CLI-02: Serve flags, bind rules, multiple instances
- **Depends on:** HTTP-05. **Model:** sonnet. **Size:** M.
- **Read:** plan §3 "Command" bullets (all); D20, D22.
- **Create/Change:** `crates/preview-cli/src/args.rs` adds `--host IP`, `--port N`, `--allow-remote`, `--debounce DUR` (parsed into `DebounceWindow` per D22), `--json`, `--open`, and `[directory]` before or after the flags. `--json` and `--open` are carried in `ServeConfig` and consumed by CLI-06 and CLI-07. Without `--allow-remote`, a non-loopback `--host` exits 2 with `refusing to bind <ip> without --allow-remote`. With it, print `site-preview: warning[remote-binding]: …no authentication…`. In `crates/preview-composition/src/server.rs`, an explicit port that is in use gives `StartupError::Bind`, which the cli maps to exit 1. Pass the `HostPolicy` from the bound address. Tests in `crates/preview-cli/tests/flags.rs`.
- **Accept:** `should_accept_directory_when_given_after_flags`, `should_accept_directory_when_given_before_flags`, `should_exit_2_when_debounce_is_zero`, `should_exit_2_when_host_is_wildcard_without_allow_remote`, `should_exit_1_when_explicit_port_in_use` (port occupied by a test listener on port 0, then passed explicitly), and `should_serve_two_instances_when_started_concurrently`.
- **Impact:** public API (CLI), security boundary.

### CLI-03: Composition: watcher pipeline and debounce task
- **Depends on:** FS-08, APP-04. **Model:** opus. **Size:** M. Parallel with TEST-05.
- **Read:** plan §8.3 (last paragraph), plan §9 ("Do not spawn…", "All background tasks…"); `ports/change_source.rs`, `commands/`.
- **Create/Change:** `crates/preview-composition/src/watch_pipeline.rs` runs one owned task: `select!` over `ChangeSource::next()` (through a blocking bridge or an async adapter handle), `sleep_until(deadline)` and cancellation. It calls `ChangeCoordinator::record/overflow/settle`. On `Settled{rescan}` or `needs_reconcile` it calls the fs adapter's `reconcile()` hook (FS-09 fills it; until then a full rescan via FS-05). The `Clock` is backed by `tokio::time::Instant`. Wire it into `server.rs`, with `--debounce` from config. Add a `Watching: N eligible directories` count to the startup report.
- **Accept:** composition tests with `tokio::time::pause` and `ScriptedChangeSource`: `should_publish_one_revision_when_events_burst_within_window`, `should_publish_after_overflow_when_channel_overflowed`, and `should_stop_task_when_cancelled` (the join handle completes). One real-FS smoke test: editing `index.html` produces a `reload` on SSE within a bounded wait.
- **Impact:** model (wiring), dependency graph.

### CLI-04: Lifecycle, startup ordering, startup refusals
- **Depends on:** E2E-03, DOM-09. **Model:** opus. **Size:** M.
- **Read:** plan §3 (JSON protocol bullet on exit codes), plan §9 (whole); fnd §6.1 item 1 invariants.
- **Create/Change:** `crates/preview-composition/src/lifecycle.rs`. A `Server` handle drives the domain `Lifecycle`: `start(config)` runs the ordered steps (root, listener, state, routes, watcher, ready); `stop()` is idempotent. Each step that fails releases everything acquired so far (`Drop` guards), and the lifecycle state is observable through `server.state()` for conformance. Startup refusals map to `StartupError::{InvalidRoot, ReservedPath, Bind, Runtime}`, and the cli maps them to exit 1 with an actionable message. Watcher init failure leads to `Serving` plus degraded health, not exit.
- **Accept:** `should_exit_1_with_message_when_reserved_path_exists`, `should_exit_1_when_root_missing`, `should_release_listener_when_watcher_init_panics_or_fails` (port reusable immediately), `should_serve_degraded_when_watcher_unavailable` (an injected failing `ChangeSource`; HTTP still 200), and `should_be_noop_when_stop_called_twice`.
- **Impact:** model, public API (exit codes).

### CLI-05: Signals and graceful shutdown
- **Depends on:** CLI-04. **Model:** opus. **Size:** M.
- **Read:** plan §3 (exit codes), plan §7.4 bullet 5, plan §9 (shutdown line, last paragraph); D27.
- **Create/Change:** `crates/preview-composition/src/shutdown.rs` handles SIGINT and SIGTERM via `tokio::signal::unix`. The shutdown order is: stop accepting (the ADR 0045 accept loop closes the listener; it replaced `axum::serve`), trigger the SSE `Shutdown` (open streams end; HTTP-10 already triggers it on SIGINT), stop the watcher (drop it and join the pipeline task), then join the remaining tasks with a bounded timeout. The process exits 0. Tests are black-box in `crates/preview-cli/tests/lifecycle.rs`.
- **Accept:** `should_exit_0_when_sigterm_received`, `should_exit_0_when_sigint_received_with_open_sse_connection` (exit within a bounded wait; no hang), and `should_close_listener_when_shut_down` (connect refused afterwards).
- **Impact:** model.

### CLI-06: `--json` ready and watcher events; final human output
- **Depends on:** CLI-05. **Model:** sonnet. **Size:** M. Parallel with CLI-07.
- **Read:** plan §3 ("Human startup output", "JSON startup protocol"); D19.
- **Create/Change:** `crates/preview-cli/src/output.rs` has `JsonReporter`, which writes `{"event":"ready","url":…,"pid":…,"watching":bool}` once after readiness, then `stdout().flush()`, and later `{"event":"watcher","state":"watching"|"degraded","reason":…}` on health changes (consuming domain events from composition through a channel). Nothing else goes to stdout in JSON mode. Warnings use the D19 format on stderr. The human output shows `Watching: N eligible directories` or `Watching: unavailable (<reason>); live reload disabled`. Switch the E2E harness (E2E-01) to `--json`.
- **Accept:** `should_emit_single_ready_line_when_json` (stdout parsed as JSONL, exactly one `ready`), `should_flush_ready_when_stdout_is_pipe` (read before any further output, bounded wait), `should_emit_watcher_degraded_event_when_watcher_fails_at_runtime`, and `should_keep_stdout_json_only_when_warnings_occur`.
- **Impact:** public API (machine interface).

### CLI-07: `--open` browser launcher
- **Depends on:** CLI-04. **Model:** sonnet. **Size:** S. Parallel with CLI-06.
- **Read:** plan §2 table row "Opening browser", plan §3 bullet "Never automatically open…"; D27.
- **Create/Change:** `crates/preview-composition/src/launcher.rs` defines trait `UrlOpener` with `SystemOpener`, which uses `std::process::Command::new("open"|"xdg-open").arg(url)` with no shell, is spawned after ready, and is not waited on beyond a spawn check. A failure becomes warning `open-failed`. A recording fake is used in tests.
- **Accept:** `should_invoke_opener_with_url_when_open_flag_set`, `should_not_invoke_opener_when_flag_absent`, `should_warn_and_keep_serving_when_opener_fails`, and `should_pass_url_as_single_argument_when_launching` (no shell metacharacter interpretation).
- **Impact:** security boundary (process spawn).

---

## Lane FS (continued): watcher adapter

### FS-05: Eligible-directory traversal
- **Depends on:** E2E-02, DOM-06. **Model:** sonnet. **Size:** S.
- **Read:** plan §8.1 steps 1–2.
- **Create/Change:** `crates/preview-adapter-fs/src/traversal.rs`. `eligible_dirs(&RootDir, start: &RelPath) -> Result<BTreeSet<RelPath>, TraversalError>` is iterative, uses `openat` + `readdir` through the capability (no path reopen), and never follows symlinks. It prunes with the DOM-02 predicate before descent, and it reports unreadable dirs as partial errors without aborting.
- **Accept:** `should_exclude_node_modules_subtree_when_traversing`, `should_not_follow_symlinked_directory_when_traversing`, `should_include_root_when_traversing`, and `should_report_unreadable_directory_when_permission_denied`.
- **Impact:** security boundary (watch scope).

### FS-06: notify registration and degraded reporting
- **Depends on:** FS-05, APP-04. **Model:** opus. **Size:** M. Parallel with FS-07.
- **Read:** plan §2.1, plan §8.1 steps 3–6, plan §8.4; fnd §7 (row "Reliable local feedback loop").
- **Create/Change:** `crates/preview-adapter-fs/src/watcher/{mod.rs,registration.rs}`. `notify` (`RecommendedWatcher`) goes in the allowlist. `NotifyWatcher::start(root, display_root)` holds the watcher for the process lifetime and calls `watch(abs, NonRecursive)` for each dir from FS-05. Absolute paths are built only here, from `display_root` + `RelPath`. Registered dirs are tracked in a `WatchSet`. A per-directory failure is collected as `RegistrationError { dir: RelPath, reason }`. Init failure returns a `Failed` batch so the coordinator degrades.
- **Accept:** `should_register_every_eligible_directory_when_started` (count equals the FS-05 set), `should_not_register_ignored_directory_when_started`, and `should_report_directory_and_reason_when_registration_fails` (a dir with permissions removed after traversal; a reason that names the relative dir).
- **Impact:** security boundary (watch scope).

### FS-07: notify event translation
- **Depends on:** FS-05, DOM-05. **Model:** sonnet. **Size:** S. Parallel with FS-06.
- **Read:** plan §8.2; `crates/preview-domain/src/change.rs`.
- **Create/Change:** `crates/preview-adapter-fs/src/watcher/translate.rs` has `translate(notify::Event, root: &Path) -> Vec<ChangeEvent>`. It maps `EventKind` variants to `ChangeKind` (`Access` to AccessOnly; `Modify(Metadata)` to MetadataOnly; `Modify(Name)` to Rename; and the `Any`/`Other` variants to Ambiguous). It strips the root prefix component-wise and drops paths outside the root. `notify::Event` never escapes the adapter.
- **Accept:** table tests over constructed `notify::Event`s for each kind, plus `should_drop_event_when_path_outside_root` and `should_emit_two_events_when_rename_has_both_paths`.
- **Impact:** none.

### FS-08: Bounded callback bridge and overflow flag
- **Depends on:** FS-06, FS-07. **Model:** opus. **Size:** M.
- **Read:** plan §8.3 (last paragraph), plan §8.4.
- **Create/Change:** `crates/preview-adapter-fs/src/watcher/bridge.rs`. The `notify` callback uses `try_send` on a bounded `tokio::sync::mpsc` (capacity configurable, default 1024). When the channel is full, it sets an `AtomicBool` overflow flag and notifies a `tokio::sync::Notify`; it never blocks. A runtime `notify` error is sent as `Failed`. `NotifyChangeSource` implements `ChangeSource`. `next()` returns `Overflow` first if the flag is set (swap false), then batches queued events.
- **Accept:** `should_signal_overflow_when_channel_full` (capacity 1, feed 100 synthetic events via the callback function directly), `should_never_block_callback_when_full` (the callback returns within a bounded wait on a full channel), and `should_deliver_failed_when_notify_reports_error`.
- **Impact:** port contract.

### TEST-05: `ChangeSource` contract harness
- **Depends on:** FS-08, TEST-03. **Model:** opus. **Size:** M. Parallel with CLI-03.
- **Read:** fnd §5.1; `ports/change_source.rs`.
- **Create/Change:** `crates/preview-testkit/src/contracts/change_source.rs`. The contract covers: events after a write are eventually delivered; `Overflow` is reported before subsequent events; `Closed` follows drop or cancellation, after which `next()` keeps returning `Closed`; and no events are delivered for ignored subtrees (scripted vs. real). It is run with `ScriptedChangeSource` and `NotifyChangeSource` (real tempdir; real-FS cases use `wait_until` and the `os-watcher` group). Negative control: a test-local fake that drops `Overflow` fails the named case.
- **Accept:** both suites pass. The negative control is detected by `should_fail_contract_when_overflow_is_dropped`.
- **Impact:** port contract.

### FS-09: Directory create/remove reconciliation
- **Depends on:** CLI-03. **Model:** opus. **Size:** M.
- **Read:** plan §8.1 ("New directories"), plan §8.4 (retry sentence).
- **Create/Change:** `crates/preview-adapter-fs/src/watcher/reconcile.rs`. `reconcile(hint: ReconcileHint::{Subtree(RelPath), Full})` re-traverses the subtree (or the nearest existing ancestor for removals), computes a `WatchDiff` (DOM-06), registers additions and unwatches removals. Re-registration failures are bounded: one retry per settle, no loop. Implement the CLI-03 hook. Atomic-save (write a temp file, rename over) is covered by classification plus the final rename event.
- **Accept:** real-FS tests: `should_watch_new_nested_directory_when_created_then_file_edited` (an edit in `a/b/` after `mkdir -p` publishes a revision), `should_rewatch_directory_when_deleted_and_recreated`, and `should_drop_stale_watch_when_directory_removed` (the internal set no longer contains it).
- **Impact:** security boundary (watch scope), model.

---

## Lane TEST (continued): coverage, mutation, proofs

### TEST-06: LLVM coverage and overall floor
- **Depends on:** FORMAL-09. **Model:** sonnet. **Size:** M. Parallel with FORMAL-10.
- **Read:** fnd §5.3 bullets 1–3; D17, D18.
- **Create/Change:** `tools/xtask/src/coverage.rs` adds `cargo xtask coverage`. It requires `cargo-llvm-cov`, runs `cargo llvm-cov nextest --workspace --locked --lcov --output-path target/coverage/lcov.info` (all in-process suites, including integration and conformance), then prints the summary. `coverage-floor.toml` holds `overall_line = 90` and a per-crate floor for `preview-domain` and `preview-application` (start at the measured value, rounded down, minimum 90). The task fails if below the floor and prints a diff. Register the `coverage` stage in `check`. If the baseline is below 90, add the missing tests to reach it; if that is impossible in-task, stop and report.
- **Accept:** `cargo xtask coverage` exits 0 and writes LCOV. Temporarily raising the floor to 100 makes it exit non-zero (verified by a unit test of the threshold evaluator). `list-checks` shows `coverage` in `check`.
- **Impact:** none.

### TEST-07: Changed-line coverage gate
- **Depends on:** TEST-06. **Model:** opus. **Size:** M. Parallel with SEC-04.
- **Read:** fnd §5.3 bullet 3; fnd §12 phase 2 (clause "an untested changed line…"); D17.
- **Create/Change:** `tools/xtask/src/changed_lines.rs` provides `cargo xtask coverage --changed-since <rev>` (default `origin/main`, falling back to `HEAD~1`). It parses `git diff -U0 <rev>` for added and modified lines in `crates/**/src/**/*.rs`, joins them with LCOV `DA:` records, and requires ≥95% of instrumented changed lines to be covered. It prints the uncovered `file:line` list. Lines that are not instrumented are excluded and counted separately in the report.
- **Accept:** unit tests over synthetic diff and LCOV inputs: `should_fail_when_changed_line_uncovered`, `should_pass_when_changed_lines_covered`, and `should_ignore_non_instrumented_lines_when_computing_ratio`. Running against the real repo HEAD succeeds.
- **Impact:** none.

### TEST-08: Mutation testing, baseline, `mutation.yml`
- **Depends on:** TEST-07. **Model:** sonnet. **Size:** M. Parallel with SEC-04.
- **Read:** fnd §5.3 bullets 4–5; fnd §12 phase 2 (clause "a known mutation survives…").
- **Create/Change:** `.cargo/mutants.toml` scopes to `preview-domain`, `preview-application` and `preview-adapter-http/src/inject`, excluding browser and E2E tests via the nextest filter. `tools/xtask/src/mutation.rs` provides `cargo xtask mutation [--in-diff <rev>]`; it requires `cargo-mutants` and writes `target/mutants/` plus a summary. Commit `doc/mutation-baseline.md` with the score and triaged survivors (equivalent or needs-test). Add `.github/workflows/mutation.yml` (weekly `schedule` and `workflow_dispatch`, SHA-pinned, uploads the report). If tooling is unavailable in the sandbox, commit the config and workflow and record the blocker in the baseline doc.
- **Accept:** a baseline run completes, or the blocker is recorded. At least one known survivor is listed with its triage. `cargo xtask mutation --in-diff HEAD~1` runs on a small diff.
- **Impact:** none.

### TEST-09: Miri lane and concurrency-proof scope
- **Depends on:** SEC-04. **Model:** sonnet. **Size:** S. Parallel with SEC-05.
- **Read:** fnd §10 (Scheduled), fnd §12 phase 5 ("targeted Loom/Shuttle/Kani…"); D25.
- **Create/Change:** `cargo xtask formal-verify --miri` runs `cargo +<pinned nightly> miri nextest run -p preview-domain -p preview-application` (with the nightly from `tools/versions.toml`). Add a Miri job to the scheduled `formal.yml` (created in FORMAL-09). Add `doc/verification-scope.md` listing which properties are verified by what (proptest, Quint conformance, Miri, fuzz) and stating that Loom, Shuttle and Kani are not used and why (D25), with the trigger for adopting them.
- **Accept:** a Miri run passes, or a reviewed tooling blocker is recorded in `doc/verification-scope.md`. The workflow passes `actionlint`.
- **Impact:** none.

---

## Lane FORMAL: Quint models, ITF replay, conformance

### FORMAL-01: Quint toolchain pin and `lifecycle.qnt`
- **Depends on:** E2E-07. **Model:** opus. **Size:** M.
- **Read:** fnd §6.1 item 1, fnd §6.2, fnd §6.4 (bullet "PR:"); `crates/preview-domain/src/lifecycle.rs`; D13.
- **Create/Change:** `formal/package.json` and `formal/package-lock.json` pin `@informalsystems/quint` at an exact version (D9). `tools/xtask/src/conformance.rs` begins with `cargo xtask conformance --typecheck`, which runs `npx --prefix formal quint typecheck formal/*.qnt` and fails with the D9 hint when Node or Quint is missing. `formal/lifecycle.qnt` covers fnd §6.1 item 1 states, actions and invariants (`noResponsesWhenStopped`, `singleListenerOwner`, `tasksReleasedAfterStop`, `stopIdempotent`, `failedStartupHoldsNothing`), with `formal/scenarios.qnt` holding the named runs `startServeStop`, `bindFailure`, `watcherFailsDuringStart` and `doubleStop`.
- **Accept:** `cargo xtask conformance --typecheck` passes. `quint run --invariant` for each invariant passes over 1000 samples. Each named scenario runs via `quint test`.
- **Impact:** model.

### FORMAL-02: `watcher.qnt`
- **Depends on:** FORMAL-01. **Model:** opus. **Size:** M. Parallel with FORMAL-03.
- **Read:** fnd §6.1 item 2; plan §8.2, §8.3; `crates/preview-domain/src/{debounce.rs,change.rs}`.
- **Create/Change:** `formal/watcher.qnt` models abstract files (eligible, ignored, temp), events (create, modify, rename, delete, atomic-save, ignored-temp, duplicate, reordered, root delete/recreate, overflow, shutdown) and a debounce tick. Invariants: `onlyEligibleTriggerReload`, `noLostEligibleChange` (dirty is eventually published once settled), `boundedReloadsPerBurst`, and `failureIsExplicit`. Add scenarios to `scenarios.qnt`: `burstOneReload`, `gitOnlyNoReload`, `atomicSave`, `overflowRescan` and `dirRecreate`.
- **Accept:** typecheck passes. Invariants hold over the simulation budget (30 traces × 25 steps to start, fnd §6.4). The scenarios pass.
- **Impact:** model.

### FORMAL-03: `reload.qnt`
- **Depends on:** FORMAL-01. **Model:** opus. **Size:** M. Parallel with FORMAL-02.
- **Read:** fnd §6.1 item 3; plan §7.2, §7.3; D16, D29, D30.
- **Create/Change:** `formal/reload.qnt` models a server revision and instance, browsers with a baseline and connection state, connect/disconnect/reconnect, publish and restart. Safety invariants: `monotonicRevision`, `noUncommittedClaim` (no notification revision exceeds the committed one), and `atMostOneReloadPerChange`. Liveness (`reconnectObservesNewer`) is stated under explicit fairness assumptions in a separate `temporal` block. Change kinds (css, js, html, asset) and the `full` flag (D29, D30) are modeled abstractly: each publish carries a set of kinds and a full flag, and each browser has `hotApplied`/`reloaded` counters. Additional safety invariants: `noHotApplyAcrossGap` (a browser hot-applies only when the event revision equals its baseline + 1), `htmlChangeIsFull`, and `hotApplyAdvancesBaseline`. Scenarios: `editDuringFetch`, `disconnectEditReconnect`, `restartSamePort`, `cssOnlyHot`, `coalescedCssThenJsFull` and `htmlFull`.
- **Accept:** typecheck passes. The safety invariants hold over the budget. The scenarios pass. The liveness assumptions are documented in comments.
- **Impact:** model.

### FORMAL-04: Quint Connect evaluation and ITF replay core
- **Depends on:** FORMAL-01. **Model:** opus. **Size:** M. Parallel with FORMAL-02, FORMAL-03.
- **Read:** fnd §6.3 (whole, especially the last paragraph); D13.
- **Create/Change:** evaluate `quint-connect` (version observed in fnd: 0.1.2; use the latest stable, locked) against the lifecycle model. Record the result as `doc/adr/NNNN-conformance-harness.md` (the next free number; Proposed): adopt it as the generator/replay integration, or not. Regardless of the verdict, implement the explicit replay path: `crates/preview-conformance/src/{lib.rs,itf.rs,replay.rs,report.rs}`. `itf.rs` is a narrow ITF JSON decoder (`serde`/`serde_json`, test-only crate) supporting `#bigint`, `#set`, `#map` and records. `replay.rs` has `trait Driver { type Obs; fn apply(&mut self, action, params) -> Result<(), DriverError>; fn observe(&self) -> Self::Obs }` and `fn replay(trace, driver, project_expected) -> Result<(), Divergence>`, comparing after **every** step. `report.rs` formats model, seed, trace id, step, action, expected, actual and diff.
- **Accept:** unit tests decode a committed tiny ITF fixture (`formal/traces/regression/itf-decoder-sample.itf.json`). `should_report_step_and_diff_when_observation_diverges` passes using a toy driver. The ADR is indexed.
- **Impact:** none (test-only crate).

### FORMAL-05: Lifecycle conformance driver
- **Depends on:** FORMAL-04. **Model:** opus. **Size:** M.
- **Read:** fnd §6.3 steps 1–5; `formal/lifecycle.qnt`; `crates/preview-composition/src/lifecycle.rs`.
- **Create/Change:** `crates/preview-conformance/src/drivers/lifecycle.rs` drives the real composition `Server` with real adapters over a tempdir, port 0 and a `ScriptedChangeSource` for watcher failure. A bind failure is induced by pre-occupying a port. Observations are projected to `{state, listenerOwned, tasksAlive}`. Add `crates/preview-conformance/tests/lifecycle_conformance.rs`, which replays each named scenario (exported via `quint run --out-itf` into `formal/traces/regression/lifecycle-*.itf.json`, committed) plus N seeded random traces generated at test time into `formal/traces/generated/`.
- **Accept:** all committed lifecycle traces replay with zero divergence. Generated traces with a recorded seed replay. The test output lists the seed.
- **Impact:** none.

### FORMAL-06: Watcher conformance driver
- **Depends on:** FORMAL-02, FORMAL-05. **Model:** opus. **Size:** M. Parallel with FORMAL-07.
- **Read:** fnd §6.3; `formal/watcher.qnt`; `crates/preview-application/src/commands/`.
- **Create/Change:** `crates/preview-conformance/src/drivers/watcher.rs` drives the real `ChangeCoordinator` and `ReloadState` with `FakeClock` and the real DOM-05 classification, translating model file kinds into `ChangeEvent`s with real names (`.git/HEAD`, `index.html`, `.index.html.swp`). Observations are `{revision, dirty, health}`. Add committed traces for the 5 scenarios and a test file `watcher_conformance.rs`.
- **Accept:** zero divergence on committed and generated traces.
- **Impact:** none.

### FORMAL-07: Reload conformance driver
- **Depends on:** FORMAL-03, FORMAL-05. **Model:** opus. **Size:** M. Parallel with FORMAL-06.
- **Read:** fnd §6.3; `formal/reload.qnt`; D16.
- **Create/Change:** `crates/preview-conformance/src/drivers/reload.rs` drives the real `ReloadState` and the real SSE handler in-process (via `tower::ServiceExt` with paused time). A model browser connection is a real SSE body stream: disconnect drops it, reconnect opens a new one, and restart builds a new `ReloadState` with a new instance id. Observations are the sequence of `(event, instance, revision, kinds, full)` received per connection plus the committed revision, so the gap rule (HTTP-12) and the `full` derivation (DOM-10) are checked against the model. The model's client decision is compared against the observed server events only (D16). Add committed traces and `reload_conformance.rs`.
- **Accept:** zero divergence on committed and generated traces.
- **Impact:** none.

### FORMAL-08: Conformance negative controls
- **Depends on:** FORMAL-06, FORMAL-07. **Model:** opus. **Size:** M.
- **Read:** fnd §6.3 step 7; fnd §12 phase 3 acceptance.
- **Create/Change:** `crates/preview-conformance/tests/negative_controls.rs` uses test-local faulty implementations behind the same driver seams: `NonMonotonicReloadState` (publishes revision − 1 on the 2nd publish), `NoDuplicateSuppressionCoordinator` (publishes per event instead of per burst), `LeakyLifecycle` (keeps the listener after stop), and `GapIgnoringSse` (sends `full: false` across a coalesced gap, violating `noHotApplyAcrossGap`). Each replays a named committed trace and asserts `Divergence { step, action, expected, actual }` at the exact expected step. Rerunning with the same seed reproduces the same step.
- **Accept:** the 4 tests pass, each asserting the precise step index and action name. A failure message sample is included in the commit body.
- **Impact:** none.

### FORMAL-09: Trace corpus, `conformance` stage, `formal.yml`
- **Depends on:** FORMAL-08. **Model:** opus. **Size:** M.
- **Read:** fnd §6.4 (whole), fnd §10 ("Conformance (Quint)" bullet); D13, D18.
- **Create/Change:** `cargo xtask conformance` (full) runs typecheck, `quint test` scenarios, bounded random simulation with invariants, **regression freshness** (regenerate each named-scenario ITF with its fixed seed and compare byte for byte with `formal/traces/regression/`; a stale fixture fails), generated traces (the seed, Quint version, family, count and max steps are recorded in `formal/traces/generated/manifest.json`), replay of both sets, and the negative controls. Register the `conformance` stage in `check`. Add `.github/workflows/formal.yml`: a job running `cargo xtask conformance` and uploading failing ITFs as artifacts, plus a `schedule` job placeholder for FORMAL-10 and TEST-09. Add `doc/formal-models.md` covering models, stimuli, observations, invariants, scope, how to promote a counterexample, and the D13 rule (model-first from now on).
- **Accept:** `cargo xtask conformance` exits 0. Editing one committed trace byte makes it fail with `stale regression trace <file>`. `check` includes the stage. The workflow passes `actionlint`.
- **Impact:** model (gate).

### FORMAL-10: Apalache `formal-verify` scheduled lane ⛳
- **Depends on:** FORMAL-09. **Model:** sonnet. **Size:** S. Parallel with TEST-06.
- **Read:** fnd §6.4 (bullet "Scheduled"), fnd §12 phase 3 (last sentence).
- **Create/Change:** `cargo xtask formal-verify` runs `quint verify` with pinned Apalache (JVM, dev-only; version in `tools/versions.toml`) for each model's safety invariants with bounded steps, plus a larger multi-seed simulation. A `schedule` job in `formal.yml` uploads counterexamples. Record the baseline result in `doc/formal-models.md` ("Verification baseline").
- **Accept:** a successful bounded verification baseline is recorded, **or** a reviewed tooling blocker is recorded (for example, no JVM or network in the sandbox). Never a fabricated pass.
- **Impact:** none.

---

## Lane SEC: supply chain, secrets, workflows, fuzz

### SEC-01: cargo-deny policy and `security` stage
- **Depends on:** OPS-01. **Model:** sonnet. **Size:** S. Parallel with DOM-01.
- **Read:** fnd §8 (paragraph 1); D23.
- **Create/Change:** `deny.toml`. Advisories: deny vulnerabilities and yanked crates; `unmaintained = "workspace"` fails an unmaintained direct dependency and does not report a transitive one (D40), with a reviewed ignore list citing ADRs. Licenses: allow MIT, Apache-2.0, BSD-2/3-Clause, ISC, Unicode-3.0 and Zlib (adjust after a real run; each addition is justified in a comment). Bans: duplicates warn, wildcard versions deny. Sources: crates.io only, with no unknown git. `tools/xtask/src/security.rs` has `cargo xtask security`, whose `deny` stage requires cargo-deny and runs `cargo deny --locked check`; register it in `check`.
- **Accept:** `cargo xtask security` passes. Adding `foo = { git = "https://example.invalid/x" }` to a crate makes it fail (a fixture-based test in `tools/xtask/tests/security_fixtures.rs` using a mini workspace).
- **Impact:** security boundary (supply chain).

### SEC-02: Gitleaks and actionlint
- **Depends on:** SEC-01. **Model:** sonnet. **Size:** S. Parallel with DOM-02.
- **Read:** fnd §8 (paragraphs 1–2); D9, D23.
- **Create/Change:** `.gitleaks.toml` (default rules plus an allowlist for `tests/fixtures/` only if needed). `cargo xtask security` adds `gitleaks git --no-banner --redact` and `actionlint` (both pinned binaries; a missing tool fails per D9). Add `.github/workflows/security.yml`, which runs the security stage on PR and weekly with SHA-pinned actions and `permissions: contents: read`.
- **Accept:** `cargo xtask security` passes. A test fixture containing a fake AWS-style key in a temp repo is detected (xtask test). A workflow with an unpinned `uses: actions/checkout@v4` fails a pinning check added to the security stage.
- **Impact:** security boundary.

### SEC-03: CodeQL and Dependabot
- **Depends on:** SEC-02. **Model:** haiku. **Size:** S. Parallel with DOM-03.
- **Read:** fnd §8 (paragraph 1); D23.
- **Create/Change:** `.github/workflows/codeql.yml` (Rust language, SHA-pinned, `security-events: write` on that job only). `.github/dependabot.yml` covers the `cargo`, `github-actions` and `npm` (`/formal`, `/tests/e2e/browser`) ecosystems weekly, with grouped minor updates.
- **Accept:** both files pass `actionlint` (the workflow) and YAML validation. The security-stage pinning check passes.
- **Impact:** none.

### SEC-04: Fuzz crate, pinned nightly, `SitePath` target
- **Depends on:** FORMAL-09. **Model:** sonnet. **Size:** M. Parallel with TEST-07, TEST-08.
- **Read:** fnd §5 (Fuzz row); D25.
- **Create/Change:** `fuzz/Cargo.toml` (cargo-fuzz layout, not a workspace member, its own `[workspace]` table) and `fuzz/rust-toolchain.toml` pinning the dated nightly. Add `fuzz/fuzz_targets/decode_path.rs`: for arbitrary bytes, `decode_request_path` never panics, and any `Ok` path contains no `.`, `..`, empty or `/`-containing component and passes `eligibility::path` consistently. Add `architecture.toml` `fuzz_only = ["preview-fuzz"]` and inventory support for it (ARCH-01 code). `cargo xtask fuzz <target> [--time N] [--reproduce <artifact>]` requires cargo-fuzz and the nightly. Commit the seed corpus `fuzz/seeds/decode_path/` (the plan §12 unit inputs).
- **Accept:** `cargo xtask fuzz decode_path --time 60` finishes with no crash, or a reviewed tooling blocker is reported. `cargo xtask architecture` recognises the fuzz crate. A negative fixture with an unlisted fuzz crate fails.
- **Impact:** dependency graph.

### SEC-05: Fuzz targets: rewriter, Host, change sequences; `fuzz.yml` ⛳
- **Depends on:** SEC-04. **Model:** opus. **Size:** M. Parallel with TEST-09.
- **Read:** fnd §12 phase 5 (fuzz clause); plan §6 step 4.
- **Create/Change:** add these fuzz targets:
  - `fuzz/fuzz_targets/rewrite_html.rs`: never panics; on `Injected`, the output contains exactly one client tag, and removing it yields the input bytes.
  - `fuzz/fuzz_targets/host_header.rs`: never panics; any `Allow` result has a port equal to the bound port.
  - `fuzz/fuzz_targets/change_sequence.rs`: arbitrary `ChangeEvent`/tick sequences into `ChangeCoordinator`; the revision is monotonic, nothing is published for ignored-only input, and a published `ChangeSet` with `full == false` has ≤ 64 entries, all of kind css/js/asset.

  Add `.github/workflows/fuzz.yml` (nightly schedule and `workflow_dispatch`; each target runs for a bounded time; crashes are uploaded as artifacts). Add `doc/security.md` § "Fuzzing" with triage steps. Exposing `inject::rewrite` to the fuzz crate requires a public test-support entry point; justify it in an ADR only if visibility must widen beyond `pub` in `preview-adapter-http`.
- **Accept:** each target runs for 60 s with no untriaged crash, or a reviewed blocker is recorded. The workflow passes `actionlint`.
- **Impact:** none.

---

## Lane E2E: black-box, browser, agent workflow

### E2E-01: Black-box harness and HTTP/security suite ⛳
- **Depends on:** CLI-02, HTTP-03, HTTP-04, FS-04. **Model:** opus. **Size:** L.
- **Read:** plan §12 "HTTP and security integration tests" (bullets 1–2, 8–9), plan §11 M1 exit gate; fnd §7 (rows 1–4); D15.
- **Create/Change:** `crates/preview-cli/tests/support/mod.rs` (`PreviewProcess::start(args)`): it copies `tests/fixtures/site` into a tempdir, spawns the binary, discovers the URL (human `Local:` line now; CLI-06 switches it to `--json`), kills it and asserts the exit on drop, and provides a `get/head/request(method, path, headers)` helper plus `tree_hash(dir)`. `crates/preview-cli/tests/http_security.rs` covers:
  - `/` and a nested index; slash redirect; no listing; GET and HEAD; 404, 403 and 405; `no-store`.
  - `.git/config`, `.env`, `vendor/x.js` and `node_modules/x.js` are never served.
  - Encoded traversal (`/%2e%2e/`, `/..%2f`, `/%252e%252e/`) gets 400 or 404 with no content.
  - Symlink to outside, symlink in the middle of the path, in-root symlink.
  - Host mismatch gets 421.
  - The tree hash is unchanged after the whole suite.

  The test fixtures add `.git/config`, `.env`, `vendor/` and `node_modules/` at copy time (not committed as dot-dirs, so they don't confuse git).
- **Accept:** every case named above passes as its own `should_*` test. The M1 exit gate holds: `cargo nextest run -p preview-cli` is green and the fixture is unmodified.
- **Impact:** none.

### E2E-02: Injection and SSE black-box suite ⛳
- **Depends on:** HTTP-07, HTTP-08, HTTP-12. **Model:** sonnet. **Size:** M. Parallel with DOC-03.
- **Read:** plan §12 "HTTP and security integration tests" (bullets 3–7, Origin bullet), plan §11 M2.
- **Create/Change:** `crates/preview-cli/tests/injection_sse.rs` covers:
  - HTML rewrite exactly once, and its `Content-Length` matches.
  - A fake `</body>` in a script or comment, and `</BODY>`.
  - `Sec-Fetch-Dest: empty` leaves the HTML unchanged.
  - A 9 MiB HTML file is unchanged and its stderr warning appears once.
  - No injection into JSON, SVG, GPX, KML, `.pdf` or a 404.
  - A cross-site `Origin` on SSE gets 403.
  - The SSE baseline `state` matches the `data-revision` of the served HTML.
  - 65 SSE connections produce one 503.
  - The source hash is unchanged.
- **Accept:** each listed case is its own passing test.
- **Impact:** none.

### E2E-03: Watcher black-box suite (Linux) ⛳
- **Depends on:** FS-09, TEST-05. **Model:** sonnet. **Size:** M.
- **Read:** plan §12 "Watcher integration tests" (whole), plan §11 M3 exit gate.
- **Create/Change:** `crates/preview-cli/tests/watcher.rs` (nextest group `os-watcher`). An SSE reader helper in `support/` collects events with a bounded wait. Cases:
  - Update HTML, CSS, JS, GPX and JSON; create and delete a file.
  - Atomic save (temp file, then rename).
  - Create a nested directory, then edit inside it.
  - Rename or delete a watched directory, recreate it, and edit inside it.
  - A burst of 20 edits produces one revision.
  - Changes under `.git/`, editor temp files and `node_modules/` produce no revision within 2× the debounce window, plus a positive control edit.
  - With the watcher made unavailable via a tiny inotify limit, HTTP still works and a warning appears (if lowering limits is impossible unprivileged, use the composition-level failing source from CLI-04 and document it).
  - Overflow is not forced through the binary; there is no production test knob. It is covered at composition level by CLI-03 (`should_publish_after_overflow_when_channel_overflowed`) and at adapter level by FS-08. This file cross-references those tests in a comment.

  macOS runs come in OPS-03.
- **Accept:** all cases pass on Linux. No case asserts exact latency (plan §12, last line).
- **Impact:** none.

### E2E-04: Playwright harness
- **Depends on:** CLI-06. **Model:** sonnet. **Size:** M.
- **Read:** plan §12 "Browser integration test" step 1–2; D8, D15.
- **Create/Change:** `tests/e2e/browser/{package.json,package-lock.json,playwright.config.ts,support/preview.ts}`. Pin `@playwright/test` to the 1.56.x release that matches Chromium 1194 and record the version in `tools/versions.toml`. The config sets `use.browserName = chromium`, captures traces and screenshots on failure, and expects `PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers` from the environment. `preview.ts` builds the binary path from `CARGO_TARGET_DIR`, spawns it with `--json --port 0`, reads the first JSONL `ready`, and kills it in teardown. Add one smoke spec, `loads.spec.ts`. Register the xtask `browser` stage (`npm ci --prefix tests/e2e/browser` then `npx playwright test`), which fails with the D9 hint if Node or Chromium is missing, and add it to `test` and `check`.
- **Accept:** `cargo xtask test --only browser` runs the smoke spec green in the sandbox without `playwright install`.
- **Impact:** none.

### E2E-05: Browser smoke: load, edit, reload, assets, widths
- **Depends on:** E2E-04. **Model:** sonnet. **Size:** M.
- **Read:** plan §12 browser steps 3–5 and 8; plan §10 steps 6–7.
- **Create/Change:** `tests/e2e/browser/reload.spec.ts` covers:
  - The initial text is visible, and there is no reload loop: a navigation counter via `page.on('framenavigated')` stays at 1 across a 2 s observation window driven by `expect.poll`.
  - Editing the HTML in the tempdir shows the new text.
  - There are no `pageerror` events and no failed requests to the origin.
  - The CSS, JS, SVG and GPX fetches succeed.
  - Screenshots at 390, 768 and 1440 px widths are attached as artifacts.
  - Teardown: the server exits 0.
- **Accept:** the spec passes. The artifacts appear in `tests/e2e/browser/test-results/` on failure.
- **Impact:** none.

### E2E-06: Browser reconnect and restart recovery
- **Depends on:** E2E-05. **Model:** opus. **Size:** M. Parallel with DOC-04.
- **Read:** plan §12 browser steps 6–7, plan §7.3 (last two paragraphs), plan §11 M2 exit gate.
- **Create/Change:** `tests/e2e/browser/reconnect.spec.ts`:
  - Block the SSE route via `page.route('**/__site_preview/events', r => r.abort())`, edit the HTML, unblock and let `EventSource` reconnect; exactly one navigation follows.
  - Stop the server, then restart it on the **same explicit port** (taken from the first `ready` URL); exactly one recovery navigation follows, and the count is stable afterwards (no infinite loop).
  - Republishing the revision manually is covered by an edit, which proves "updating revision reloads exactly once".
- **Accept:** both tests pass with navigation-count assertions.
- **Impact:** none.

### E2E-09: Browser hot reload: CSS, assets, JS scroll, accept API
- **Depends on:** E2E-05, HTTP-15, HTTP-16. **Model:** sonnet. **Size:** M. Parallel with E2E-06.
- **Read:** D29, D30; `tests/e2e/browser/reload.spec.ts` (harness use).
- **Create/Change:** `tests/e2e/browser/hot.spec.ts`. Each test sets `window.__marker = <random>` and counts navigations. Add fixture pages under the temp copy: a long page; a page with `@import`; and a page with `accept`.
  - **CSS:** editing `styles.css` changes the computed style, the marker survives, there are 0 navigations, and no flash (the old link stays until the new one loads, asserted by link count ≥ 1 at every observed moment).
  - **@import:** editing an imported CSS file swaps the owning link with no navigation.
  - **Asset:** replacing `images/sample.svg` updates the `img` `currentSrc` with no navigation. An unreferenced asset (`routes.gpx`) gives exactly one navigation.
  - **JS:** after scrolling to y = 1500, editing `app.js` gives exactly one navigation and restores `scrollY` within ±2 px.
  - **Accept API:** on the accept-page, editing the accepted module calls the handler (a DOM text change from the re-imported module) with 0 navigations. A handler that throws gives exactly one navigation.
  - **HTML:** editing `index.html` gives exactly one navigation (a full reload).
  - **Gap:** two quick edits to css then js produce exactly one navigation.
- **Accept:** all cases pass as named tests. The marker assertions prove no navigation for the hot paths.
- **Impact:** none.

### E2E-07: `AGENTS.md`, example skill, agent workflow test ⛳
- **Depends on:** E2E-06, E2E-09, DOC-04. **Model:** sonnet. **Size:** S.
- **Read:** plan §10 (whole), plan §11 M4 exit gate.
- **Create/Change:** `AGENTS.md` (the plan §10 instructions, adjusted to the final flags and JSON schema, plus the isolation caveat). Add `examples/claude-skill/static-html-preview/SKILL.md`, an example skill for end users. It is not placed under `.claude/` (rule 11). Add `tests/e2e/browser/agent-workflow.spec.ts`, which scripts the AGENTS.md steps: start, parse ready, check `watching:true`, screenshot at 3 widths, collect the accessibility snapshot and console errors, edit, observe reload, then stop on both success and a forced-failure path.
- **Accept:** the spec passes. Every command in `AGENTS.md` is exercised by that spec (asserted by a comment table in the spec, verified by DOC-05 later). M4 exit gate: the agent workflow completes with no manual step.
- **Impact:** none.

### E2E-08: Soak and performance measurement
- **Depends on:** SEC-05. **Model:** sonnet. **Size:** M. Parallel with OPS-02.
- **Read:** plan §13 (whole); D28.
- **Create/Change:** `cargo xtask bench` (non-blocking, not in `check`) runs:
  - startup time (spawn to `ready`, median of 10) on a generated 1,000-file site with nested dirs and an ignored 10k-file `node_modules`;
  - save-to-SSE `reload` latency (median and p95 over 50 saves);
  - idle CPU over 30 s (from `/proc` on Linux or `ps` on macOS);
  - a soak of 200 saves with 10 SSE clients and a 64-connection cap check, asserting no leaked fds and a stable RSS.

  It writes `target/bench/report.json`. Add `doc/performance.md` with the procedure, the first recorded results and the environment.
- **Accept:** `cargo xtask bench` completes and writes a report. `doc/performance.md` lists measured values against the plan §13 targets, with any miss noted for owner review.
- **Impact:** none.

---

## Lane OPS: gate composition, CI matrix, release

### OPS-01: `cargo xtask test` and `check`; CI runs `check`
- **Depends on:** ARCH-07. **Model:** sonnet. **Size:** M. Parallel with DOC-01.
- **Read:** fnd §9 (whole), fnd §10 (first block); D18.
- **Create/Change:** `tools/xtask/src/{test.rs,check.rs}`. `test` runs `cargo nextest run --workspace --locked --profile ci` plus `cargo test --doc`, plus the registered extra stages (`client-js` and `browser` once they exist). `check` runs `quick`, `architecture` (with fixtures), `test`, and every stage registered as `in_check` in the catalog. It prints a final table of stage, status and duration, and exits non-zero if any stage failed or could not run. There is no "skipped" status. `list-checks --json` includes `in_check`. `ci.yml` switches the job to `cargo xtask check` (installing the pinned tools the registered stages need).
- **Accept:** `cargo xtask check` exits 0. A test with a fake failing stage makes `check` exit non-zero and print it as `FAILED`. A test with a missing tool shows `UNAVAILABLE` and a non-zero exit. **From this task on, rule 2 requires `check`.**
- **Impact:** none.

### OPS-02: CI job topology and failure evidence
- **Depends on:** SEC-05, TEST-08. **Model:** sonnet. **Size:** M. Parallel with E2E-08.
- **Read:** fnd §10 "PR blocking (Linux)" (whole), fnd §9 (last paragraph).
- **Create/Change:** split `ci.yml` into the jobs `architecture`, `static`, `fast-tests`, `integration`, `conformance`, `coverage` (with the changed-line gate on the pushed commit range), `security`, `browser` and `artifact-smoke` (a release build, run outside the repo on the fixture; extended in OPS-05). Each job calls one xtask stage (`cargo xtask check --only <stage>`, added here) so local and CI behavior stay in parity. Upload JUnit, LCOV, Playwright traces and failing ITFs as artifacts. Caches are keyed by lockfile and toolchain. A final `required` job `needs:` all of them.
- **Accept:** `actionlint` passes. `cargo xtask check --only coverage` works locally. The orchestrator confirms all jobs are green on `main`.
- **Impact:** none.

### OPS-03: Linux+macOS native matrix
- **Depends on:** OPS-02. **Model:** sonnet. **Size:** M.
- **Read:** fnd §10 ("Cross-platform required matrix"), plan §11 M5 (runner list), plan §12 watcher heading; D2.
- **Create/Change:** add a `platform` job matrix covering `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-15` (arm64) and `macos-15-intel`. It runs fast-tests, integration (adapter-fs, watcher E2E-03, symlink race) and the cli black-box suites natively. There is no Windows (D2). Fix any macOS-specific behavior the matrix reveals (FSEvents coalescing, `/private/var` canonical root, `ELOOP` vs `EMLINK`) in the adapter with a test, or record a known limitation in `doc/runbook.md`.
- **Accept:** all 4 platforms are green on `main` twice in a row (confirmed by the orchestrator). `grep -i windows .github/workflows/*.yml` finds nothing.
- **Impact:** none.

### OPS-04: CI matrix green twice on `main`, `doc/sdlc.md`, required checks ⛳
- **Depends on:** OPS-03. **Model:** sonnet. **Size:** S.
- **Read:** fnd §11 (whole); D5, D31.
- **Create/Change:** `doc/sdlc.md` covers the Definition of Ready, the agent procedure, the Definition of Done, the gate list and the integration procedure (direct on `main`, one signed-off commit per task, the `required` check as the CI result to watch, no force-push, review at ⛳, red CI on `main` fixed immediately on `main`). Add `CONTRIBUTING.md` content for outside contributors (DCO, `cargo xtask check`, ADR rule, pull-request guidance) and `.github/pull_request_template.md` (Impact line, gates run, evidence) for outside contributions. Add `doc/required-checks.md` documenting the checks that must be green on `main` (the `required` aggregate job and its members). It enables no branch protection and no rulesets.
- **Accept:** the files exist, and the `required` check is green on `main` for two consecutive pushes (confirmed by the orchestrator). Integration stays direct on `main`.
- **Impact:** none.

### OPS-05: `release-check`: packaged-binary smoke
- **Depends on:** OPS-04. **Model:** sonnet. **Size:** M. Parallel with DOC-05.
- **Read:** plan §11 M5 (bullets "Use musl…", "Test the published archive"), fnd §7 (rows "No JS runtime…", "No modifications…").
- **Create/Change:** `tools/xtask/src/release.rs` adds `cargo xtask release-check [--target T]`. It builds `--release --locked` (for Linux targets, `x86_64/aarch64-unknown-linux-musl` via `rustup target add` and `musl-tools`), packages `site-preview-<target>.tar.gz` (binary, README, licenses, `THIRD-PARTY-NOTICES`), extracts it into a fresh temp dir, and runs the extracted binary with `env -i PATH=/usr/bin:/bin` (no cargo, node or npm on PATH; asserted). It runs `--version` and `serve --json --port 0` on a copy of the fixture, GETs `/`, edits a file, awaits an SSE `reload`, sends SIGTERM and checks exit 0, and verifies the fixture tree hash. Record the minimum kernel expectations in `doc/runbook.md`. **Third-party notices (ADR 0043):** `THIRD-PARTY-NOTICES` is generated from `Cargo.lock` for the shipped crate graph (normal dependencies of `site-preview` on that target, no dev or build-only tools) by `cargo-about`, pinned in `tools/versions.toml` with its config committed; it holds each crate's name, version, licence and licence text (MIT and Apache-2.0 copyright notices included), and for the MPL-2.0 crates (`cssparser`, `cssparser-macros`, `dtoa-short`, `selectors`) the MPL-2.0 text and where their source is available (MPL-2.0 §3.2). `release-check` regenerates it, fails when the archived copy differs, and fails when any crate in the shipped graph is missing from it.
- **Accept:** `cargo xtask release-check` passes for the host target in the sandbox, or reports a missing musl target as a tooling blocker. The `artifact-smoke` CI job calls it. The extracted archive contains `THIRD-PARTY-NOTICES`, naming the four MPL-2.0 crates with the MPL-2.0 text; deleting one crate's entry from the packaged copy makes `release-check` fail; a missing `cargo-about` is an actionable failure, not a skip.
- **Impact:** none.

### OPS-06: `release.yml`: archives, SHA256SUMS, provenance, SBOM ⛳
- **Depends on:** OPS-05, DOC-05. **Model:** opus. **Size:** M.
- **Read:** plan §11 M5 (whole); fnd §8 (paragraph 2), fnd §10 ("Release"); D23, D24.
- **Create/Change:** `.github/workflows/release.yml` triggers on `push` of tags `v*` only. A build matrix on native runners (`macos-15`, `macos-15-intel`, `ubuntu-24.04`, `ubuntu-24.04-arm`) runs `cargo xtask release-check --target <t>`, which produces the 4 archive names from plan §11 M5. A `publish` job (with `contents: write`, `id-token: write` and `attestations: write`, in that job only) downloads the artifacts, writes `SHA256SUMS`, generates a CycloneDX SBOM via pinned `cargo-cyclonedx`, runs `actions/attest-build-provenance` (SHA-pinned), and creates a draft GitHub release. A `verify` job downloads the **published** archives on each OS, checks `SHA256SUMS`, and reruns the smoke. There is no PR trigger and no secrets beyond `GITHUB_TOKEN`, and there is no crates.io publish (D24).
- **Accept:** `actionlint` passes. A `workflow_dispatch` dry run on a test tag (by the orchestrator) produces 4 archives, `SHA256SUMS`, an SBOM and attestations in a draft release, and `verify` is green.
- **Impact:** security boundary (release trust).

### OPS-07: v0.1.0: changelog, install docs, tag handoff
- **Depends on:** OPS-06, E2E-08. **Model:** haiku. **Size:** S.
- **Read:** plan §11 M5 (last bullet and exit gate), plan §16.
- **Create/Change:** `CHANGELOG.md` (the 0.1.0 entry). In the README Install section: a minimal `curl … | tar xz` install command per platform, `sha256sum -c` verification, and `gh attestation verify`. Add `doc/release-checklist.md`, mapping each plan §16 item to the evidence (CI job, test name, report). **The orchestrator or owner pushes tag `v0.1.0`**, never the agent.
- **Accept:** every plan §16 checkbox has linked evidence in `doc/release-checklist.md`. After the owner tags, the release workflow is green, and the 4 archives preview the fixture on clean runners (the plan §11 M5 exit gate).
- **Impact:** public API (first release).

---

## Lane DOC: architecture, testing, security docs, README, consistency

### DOC-01: `doc/architecture.md`, `doc/architecture-rules.md`
- **Depends on:** ARCH-07. **Model:** haiku. **Size:** S. Parallel with OPS-01.
- **Read:** fnd §2 (whole), fnd §3.3; `architecture.toml`; `tools/xtask/src/architecture/rules.rs`; D10–D12.
- **Create/Change:** `doc/architecture.md` covers the crate map, the dependency law diagram, ownership per crate, ports (to be filled as APP lands; list the planned ports) and a "how to add a crate" checklist. `doc/architecture-rules.md` is a catalog table with rule id, statement, enforcement (compiler, xtask graph, xtask semantic, trybuild), negative fixture path and ADR.
- **Accept:** every rule id from `rules.rs` appears in the catalog table (DOC-05 will automate this). Links resolve.
- **Impact:** none.

### DOC-02: `doc/testing.md`
- **Depends on:** TEST-04, TEST-02. **Model:** haiku. **Size:** S. Parallel with E2E-01.
- **Read:** fnd §5 (table), fnd §5.1, fnd §5.2; D15, D17.
- **Create/Change:** `doc/testing.md` covers levels and their locations in this repo, naming and given/when/then, the bounded-poll rule, the contract harness pattern (how to add an adapter), proptest seeds, nextest groups and profiles, and the commands per level (from `list-checks`).
- **Accept:** every command shown exists in the `list-checks` output.
- **Impact:** none.

### DOC-03: `doc/security.md`
- **Depends on:** HTTP-11. **Model:** sonnet. **Size:** S. Parallel with E2E-02.
- **Read:** plan §5.2–§5.4, plan §15; fnd §7, fnd §8; D3, D20, D21.
- **Create/Change:** `doc/security.md` covers the threat model (local developer, browser on the same machine, LAN with `--allow-remote`), HTTP-serving confinement (D3; "not a sandbox"), path rules, Host and Origin rules, injection, the no-telemetry and logging policy, the `.well-known` limitation, the `--allow-remote` warning, how to report a vulnerability, and a "Fuzzing" placeholder for SEC-05. Add `SECURITY.md` at the root pointing to it.
- **Accept:** each plan §15 bullet appears. Each security rule names its enforcing test.
- **Impact:** none.

### DOC-04: README completion and `doc/runbook.md`
- **Depends on:** CLI-06, CLI-07, HTTP-15. **Model:** sonnet. **Size:** M. Parallel with E2E-06.
- **Read:** plan §1, §3, §15; D19, D29, D30.
- **Create/Change:** `README.md` covers what it is, quick start, the CLI reference (every flag), the JSON protocol schema (`ready` and `watcher`) and the warning codes table (D19), the SSE protocol and hot-reload behavior (what hot-swaps, what reloads, the scroll restore through `history.state`, and the `window.__sitePreview.accept` API; D29, D30), the HTTP contract summary, limitations (plan §15), the dev setup (`cargo xtask doctor`, `install-tools`, `check`) and links to `doc/`. The install section is added in OPS-07. `doc/runbook.md` covers troubleshooting: inotify watch quota (`fs.inotify.max_user_watches`), network mounts, overflow, port in use, the CSP warning, reserved-path conflict, and degraded watcher.
- **Accept:** every flag in `site-preview serve --help` appears in the README (verified by DOC-05). Every warning code emitted in the source (`grep 'warning\['`) appears in the table.
- **Impact:** none.

### DOC-05: Documentation consistency check
- **Depends on:** OPS-04. **Model:** sonnet. **Size:** M. Parallel with OPS-05.
- **Read:** fnd §10 ("docs-rule consistency"), fnd §12 phase 6 ("documentation consistency checks").
- **Create/Change:** `tools/xtask/src/docs.rs` provides the `docs` stage, which is in `check`. It verifies:
  - every ADR file is listed in `doc/adr/README.md` and vice versa, with a valid status;
  - every rule id in `rules.rs` is in `doc/architecture-rules.md`;
  - every `cargo xtask <cmd>` in Markdown files exists;
  - every `serve` flag from clap (via `--help`) is in the README;
  - every warning code is in the README;
  - every relative Markdown link resolves;
  - the commands in `AGENTS.md` are referenced by the agent workflow spec.

  Fixtures provide negative cases.
- **Accept:** the stage passes on the repo. Fixtures with an unindexed ADR, an unknown xtask command and a broken link each fail with a precise message.
- **Impact:** none.

---

## Traceability

### plan.md §12 test matrix

| Item | Tasks |
|---|---|
| Unit: URL decoding (`%2e%2e`, `%2f`, `%5c`, double encoding, malformed `%`, NUL, Unicode, spaces) | DOM-01, TEST-02, SEC-04 |
| Unit: ignore rules match components, not substrings | DOM-02 |
| Unit: MIME overrides and Content-Type list | HTTP-03 |
| Unit: one global debounce, one revision per burst | DOM-04, APP-04, CLI-03 |
| Unit: render-time revision vs initial SSE `state`; later `reload` detects change | DOM-03, HTTP-09 (`client.test.mjs`), HTTP-10, E2E-02 |
| Unit: instance change distinguishable from revision change | DOM-03, HTTP-09 |
| HTTP: `/`, nested index, slash redirect, no listing | APP-03, HTTP-04, E2E-01 |
| HTTP: GET/HEAD; 404/403/405; no-store; Content-Length after rewrite | HTTP-01, HTTP-02, HTTP-07, E2E-01, E2E-02 |
| HTTP: rewrite exactly once; source bytes unchanged | HTTP-06, HTTP-07, E2E-02 |
| HTTP: fake `</body>` in script or comment; mixed case | HTTP-06, E2E-02 |
| HTTP: `Sec-Fetch-Dest: empty` not modified | DOM-08, HTTP-07, E2E-02 |
| HTTP: HTML over the limit served unchanged with warning | DOM-08, HTTP-07, E2E-02 |
| HTTP: no injection into JSON/SVG/GPX/KML/downloads/errors | DOM-08, HTTP-07, E2E-02 |
| HTTP: `.git/config`, `.env`, vendor, node_modules, symlink outside/middle, atomic swap | DOM-02, FS-01, FS-04, E2E-01 |
| HTTP: Host mismatch, forbidden remote bind, disallowed Origin, port in use | DOM-07, HTTP-05, HTTP-11, CLI-02, E2E-01, E2E-02 |
| HTTP: internal-route conflict reported at startup | FS-03, CLI-04 |
| Watcher: update HTML/CSS/JS/GPX/JSON; create/delete | E2E-03, OPS-03 (macOS) |
| Watcher: rename/atomic save; nested dir create then edit | FS-09, E2E-03 |
| Watcher: rename/delete watched dir, recreate, edit | FS-09, E2E-03 |
| Watcher: burst gives one revision | CLI-03, E2E-03 |
| Watcher: `.git`/temp/excluded give no revision | DOM-05, FS-06, E2E-03 |
| Watcher: unavailable watcher warns, HTTP continues | CLI-04, CLI-06, E2E-03 |
| Watcher: simulated overflow gives rescan / reload | FS-08, CLI-03, E2E-03 |
| Watcher tests on macOS and Linux | E2E-03, OPS-03 |
| Browser 1–2: start `--json --port 0`, read ready, navigate | E2E-04 |
| Browser 3: initial text, no reload loop | E2E-05 |
| Browser 4: edit, new text | E2E-05 |
| Browser 5: no exceptions or missing assets | E2E-05 |
| Browser 6: disconnect, edit, reconnect, one recovery reload | E2E-06 |
| Browser 7: restart on same port, no infinite reload | E2E-06 |
| Browser 8: shutdown, exit status, cleanup | E2E-05, CLI-05 |
| Bounded waits, no exact-latency assertions | TEST-01, ARCH-07, E2E-03 |

### Hot reload (D29, owner decision; supersedes plan §7.3 "always full reload")

| Item | Tasks |
|---|---|
| Debounced revision carries the changed eligible paths, capped, with full fallback | DOM-10, APP-02, APP-04, SEC-05 |
| SSE `reload` payload `{instance_id, revision, changes, full}`; no hot apply across a coalesced gap | HTTP-12, FORMAL-03, FORMAL-07, FORMAL-08 |
| CSS swapped in place (including `@import`), no flash; unmatched means full reload | HTTP-13, E2E-09 |
| JS change reloads with scroll restore (no storage APIs) | HTTP-14, E2E-09 |
| Opt-in ES-module hot-accept API; unaccepted or failed means full reload | HTTP-15, E2E-09 |
| Images and CSS `url()` assets cache-busted; unreferenced means full reload | HTTP-16, E2E-09 |
| HTML change means full reload; baseline/instance logic intact | DOM-10, HTTP-09, E2E-06, E2E-09 |

### plan.md §16 definition of done

| Item | Tasks |
|---|---|
| One executable, no Rust/Node/npm/per-site setup | OPS-05, OPS-06, OPS-07 |
| `serve .` serves HTML and static assets on localhost | CLI-01, CLI-02, E2E-01 |
| Source files never modified | HTTP-07, E2E-01, E2E-02, OPS-05 |
| File changes give one debounced reload (or one hot update, D29) | DOM-04, DOM-10, CLI-03, E2E-03, E2E-05, E2E-09 |
| Connect/disconnect/reconnect/restart never reload endlessly | HTTP-09, HTTP-10, E2E-06, FORMAL-03, FORMAL-07 |
| `--json --port 0` stable flushed URL | CLI-06, E2E-04 |
| Playwright MCP can inspect and repeat after edits | E2E-07 |
| Dotfiles, excluded dirs, traversal, symlink escapes, DNS rebinding handled | DOM-01, DOM-02, DOM-07, FS-01, FS-04, HTTP-05, E2E-01 |
| Watcher failure visible, HTTP usable | DOM-09, CLI-04, CLI-06, E2E-03 |
| CI and smoke tests pass on macOS ARM64/x86_64 and Linux ARM64/x86_64 | OPS-03, OPS-06 |
| v0.1.0 archives and SHA-256 checksums | OPS-06, OPS-07 |

### foundation.md §13 deliverable inventory

| Item | Tasks |
|---|---|
| ARCH-01 workspace topology and toolchain pin | FND-01 |
| ARCH-02 policy schema and Cargo graph checker | ARCH-01, ARCH-02, ARCH-03 |
| ARCH-03 semantic module/library restrictions | ARCH-05, ARCH-06, ARCH-07, ARCH-09 |
| ARCH-04 negative-fixture harness | ARCH-01, ARCH-04 |
| ARCH-05 public API compile-fail fixtures | ARCH-08 |
| TEST-01 nextest, deterministic fakes/testkit contracts | TEST-01, TEST-03, TEST-04, TEST-05 |
| TEST-02 property tests and seed persistence | TEST-02 |
| TEST-03 LLVM coverage, changed-code gate, ratchet | TEST-06, TEST-07 |
| TEST-04 mutation testing, baseline, triage | TEST-08 |
| FORMAL-01 lifecycle model and scenarios | FORMAL-01 |
| FORMAL-02 watcher model and scenarios | FORMAL-02 |
| FORMAL-03 reload model and scenarios | FORMAL-03 |
| FORMAL-04 Quint Connect evaluation and ITF replay | FORMAL-04 |
| FORMAL-05 observation equivalence and negative controls | FORMAL-05, FORMAL-06, FORMAL-07, FORMAL-08 |
| FORMAL-06 generated/committed trace CI and Apalache | FORMAL-09, FORMAL-10 |
| SEC-01 advisories/licenses/registries | SEC-01, ARCH-03, OPS-05 (third-party notices in each archive, ADR 0043) |
| SEC-02 secrets, workflow analysis, provenance | SEC-02, SEC-03, OPS-06 |
| SITE-01 directory capability and path security | DOM-01, DOM-02, FS-01..FS-04, TEST-04 |
| SITE-02 HTTP serving / injection contract | APP-03, HTTP-01..HTTP-08 |
| SITE-03 FS watcher and event normalization | DOM-05, DOM-06, FS-05..FS-09, TEST-05 |
| SITE-04 reload delivery, reconnect, cancellation | APP-02, HTTP-09..HTTP-16, CLI-05 |
| SITE-05 E2E/browser/cross-platform suite | E2E-01..E2E-07, OPS-03 |
| OPS-01 xtask consolidated gate | FND-03, OPS-01 |
| OPS-02 CI matrix, caches, failure evidence | FND-05, OPS-02, OPS-03 |
| OPS-03 release artifact smoke/attestation | OPS-05, OPS-06 |
| DOC-01 ADRs, architecture contracts, agent guide | `doc/adr/` (planner), ARCH-05, FORMAL-04, DOC-01..DOC-05, OPS-04, E2E-07 |

### foundation.md §12 phase acceptance

| Phase | Acceptance clause | Tasks |
|---|---|---|
| 0 | Clean checkout builds the empty CLI, xtask runs with no external runtime; bad/missing toolchain gives a clear error; crates compile; tool versions committed | FND-01..FND-05 |
| 1 | Each forbidden edge, domain std-fs call, unregistered crate, unexpected build dep, testkit prod dep fails with the right diagnostic; selectors find targets; positive and negative fixtures in CI | ARCH-01..ARCH-08, OPS-01 |
| 2 | Broken fake fails the same contract as broken real adapter | TEST-04, TEST-05 |
| 2 | Untested changed line trips the changed-code gate | TEST-07 |
| 2 | Known mutation survivor visible | TEST-08 |
| 2 | Failing test is non-zero locally and in CI; gate runs on clean checkout | OPS-01, OPS-02 |
| 3 | Removing monotonicity / duplicate suppression fails at a named step; same seed reproduces | FORMAL-08 |
| 3 | Committed corpus freshness and generated traces on every push to `main` | FORMAL-09 |
| 3 | Apalache baseline or reviewed tooling blocker | FORMAL-10 |
| 4 | Fresh dir served by one binary; browser refreshes incl. atomic save | CLI-01, E2E-03, E2E-05 |
| 4 | Path escapes and hostile origins rejected; source never changes | E2E-01, E2E-02 |
| 4 | Reconnection and shutdown tests pass; no Node/npm at runtime; model-conformant | E2E-06, CLI-05, OPS-05, FORMAL-05..FORMAL-07 |
| 5 | Platform artifacts pass real FS/HTTP tests; clean restart leaks nothing | OPS-03, OPS-05, CLI-04, E2E-08 |
| 5 | Traversal and symlink race tests pass | FS-04, E2E-01 |
| 5 | Fresh fuzz runs have no untriaged crash | SEC-04, SEC-05 |
| 5 | Verified properties and scope documented; performance procedure and approved budget before blocking | TEST-09, E2E-08 |
| 6 | Fresh machine installs and runs each artifact; required checks green | OPS-06, OPS-07 |
| 6 | Agent can find remediation for every broken rule; enforced PR rules; doc consistency | ARCH-04, DOC-01, DOC-05, OPS-04 |

---

## Deferred (P1)

These items come from plan §14. They are not tasked. Each one needs observed user friction plus an ADR before it gets a task.

1. ~~CSS-only refresh that keeps page state~~: moved into P0 by D29 (HTTP-12..16, E2E-09).
2. Polling fallback for network filesystems and broken native notifications.
3. Configurable SPA fallback.
4. A one-shot build command.
5. Machine-readable health/status and richer errors beyond `ready` and `watcher`.
6. Local HTTPS.
7. Custom ignore patterns, or selectively exposing `.well-known`.
8. Browser-console forwarding.
9. HTTP byte ranges.
10. A Homebrew tap and formula, plus automated release signatures.

Also deferred by decision: Windows support (D2), crates.io publishing (D24), `cargo-vet` and `cargo-machete` (D23), and Loom, Shuttle and Kani (D25).
