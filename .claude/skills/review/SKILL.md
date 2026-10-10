---
name: review
description: Read-only review of a git range (default since the newest checkpoint-* tag) against site-preview's enforced architecture rules, the cited plan.md/foundation.md sections and ADRs, path-confinement and Host/Origin security, HTML injection, the SSE reload protocol, the watcher, concurrency and shutdown, Quint conformance, test quality and supply chain. Produces findings by severity with file:line and a verdict. Use at review checkpoints (⛳ tasks in doc/tasks.md) or when asked to /review.
---

# /review

Review a range of commits (or an uncommitted diff against `HEAD`) and report. **This skill never edits a file, never commits, never tags, never pushes and never merges.** Fixes are the implementer's job, after the report.

## Input

`/review [range]` where `range` is any git range (`a..b`, a single commit, `HEAD~N`), or `HEAD` plus "uncommitted" for the working tree.

Default range: `<newest checkpoint tag>..HEAD`, found with
`git tag --list 'checkpoint-*' --sort=-creatordate | head -1`. If there is no such tag, use the root commit to `HEAD` and say so. If the range is empty, report a clean verdict for an empty range and stop.

## Procedure

1. `git log --oneline <range>` and `git diff --stat <range>`. Note the task ids in the subjects (`<ID>: summary`).
2. For each task id, read its section in `doc/tasks.md`: the sections of `doc/plan.md` and `doc/foundation.md` it cites, and its `Accept:` clauses. Read those sections, and the Accepted ADRs in `doc/adr/` that touch the changed crates. Decisions recorded in `doc/tasks.md` and later ADRs override earlier plan text.
3. Read every changed file in full (not just the hunks), plus the tests beside it.
4. Run the live gate (`cargo xtask check`, or what `doc/tasks.md` names; the fallback before `xtask` exists is in `CLAUDE.md`) and record pass/fail and the test count. If it rewrites tracked files, report which as a finding; do not revert or keep them yourself.
5. Walk the checklist below. Only report what you verified in the code; cite `file:line`.

## Checklist

**Enforced architecture** (`architecture.toml`, `cargo xtask architecture`; foundation.md §2.1, §2.2, §3.3): dependency direction cli → composition → adapters → application → domain; domain free of `std::fs`, `std::net`, `std::process`, env reads, uncontrolled clock, randomness, tokio, HTTP and adapter symbols; adapters never reference sibling adapters or composition; HTTP handlers never construct filesystem or watch adapters; commands never import query handlers or HTTP DTOs; queries never mutate; no `axum`/`notify` type escapes its adapter; production crates never depend on testkit, conformance or fuzz; a new crate has its workspace member and `architecture.toml` entry in the same commit, and the policy has a negative fixture for each new rule. A change that loosens `architecture.toml`, `[workspace.lints]`, `deny.toml` or a test without an ADR is HIGH.

**Language rules**: `unsafe_code` forbidden; no `todo!`, `unimplemented!`, `dbg!`; no `#[ignore]`; no `#[allow(...)]` or coverage exclusion without an ADR; no `unwrap`/`expect` on input-derived values in production paths.

**Plan fidelity and ADRs**: behaviour matches the cited sections and every `Accept:` clause; a deviation is recorded in `doc/plan.md`/`doc/tasks.md` in the same commit or is a finding. A decision without an ADR (new crate boundary, dependency of consequence, security or protocol policy, rule exception) is MEDIUM; code contradicting an Accepted ADR is MEDIUM, HIGH if it loosens a security or architecture rule. A superseding ADR marks the old one `Superseded by NNNN` and `doc/adr/README.md` lists both.

**Path confinement** (plan.md §5.2, §5.3): one normalisation function before any open; percent-decoding exactly once; invalid encoding, NUL, backslash, encoded separators, `.` and `..` refused; deny list matched on whole components (leading `.`, `node_modules`, `vendor`, `.venv`, `__pycache__`, editor temp and `~` names), never substrings; every component opened with `openat` relative to the previous directory fd, `O_NOFOLLOW | O_CLOEXEC` (`O_DIRECTORY` for intermediates), the final fd checked as a regular file and read by fd. `canonicalize` + `starts_with`, a string-prefix check, or a path re-opened after a check is HIGH. No directory listing; internal `__site_preview` routes never served from disk; a reserved-path conflict fails startup.

**Host and Origin** (plan.md §5.4): loopback bind by default; non-loopback only with `--allow-remote` plus a stderr warning; Host must match the bound IP and port (narrow `localhost` alias), arbitrary DNS names refused even in remote mode; no permissive CORS; SSE refuses a conflicting `Origin` or cross-site `Sec-Fetch-Site`.

**HTML injection** (plan.md §6): only successful GET `text/html` with `Sec-Fetch-Dest` `document`/`iframe` (or absent); `lol_html`, never byte search for `</body>`; 8 MiB bound, larger served unchanged with one warning; no duplicate tag; render-time instance/revision snapshot taken before reading the file, attributes escaped; `Content-Length` recomputed and `ETag` dropped; CSP never rewritten; source files never written.

**SSE and reload** (plan.md §7): subscribe to the `watch` channel **before** reading the snapshot; `state` baseline then `reload` events with monotonic revision and per-process instance id; latest-value channel, no per-client queues; 64-connection limit with 503 and the permit released on disconnect and shutdown; keepalive; `no-store` and `nosniff`; client keeps no cookies, storage or browser ids and reloads at most once per change.

**Watcher** (plan.md §8): the eligibility predicate is the same function serving uses; non-recursive watches on eligible directories only, symlinks not followed; ignored components checked before publishing; one global trailing-edge debounce; the `notify` callback never blocks (bounded channel, overflow flag → full rescan and at least one reload); runtime failure → degraded state reported, HTTP keeps serving, no spin retries.

**Concurrency and lifecycle** (plan.md §9): every spawned task owned, cancellable and joined or bounded on shutdown; no blocking I/O on the async runtime outside `spawn_blocking`; shutdown order listener → SSE → watcher → tasks and idempotent; no lock held across `.await`; `--json` stdout strictly JSONL, flushed after `ready`, exactly one `ready`; exit codes per plan.md §3.

**Security hygiene**: no shell interpolation of paths or URLs; no secrets, absolute paths or query strings in logs or error bodies; public repo: no tokens, internal hostnames, model ids or private-repo content in any file.

**Formal conformance** (foundation.md §6): a lifecycle, watcher or reload behaviour change updates the Quint model, named scenarios and the conformance driver in the same change (model first); the driver drives real code and compares after every step; a negative control (deliberately faulty component) exists and still fails; regression traces committed and fresh; no invariant removed to get green (HIGH).

**Tests** (foundation.md §5): each `Accept:` clause has a test that would fail without the change; the right level (unit, property, component, contract, architecture, integration, e2e, compile-fail, conformance, browser); names `should_<outcome>_when_<condition>` with `// given / when / then`; no sleeps in deterministic tests (bounded polling with a reason only in OS-watcher tests); dynamic ports and tempdirs; real objects or testkit fakes, no mocking frameworks; a fake and its real adapter share one contract harness; property counterexamples and regressions committed with their seed. Missing tests for a new branch is MEDIUM.

**Supply chain** (foundation.md §8): a new dependency added with `cargo add`, justified in the commit message, licence accepted by `deny.toml`, no git or foreign-registry source without an ADR; `Cargo.lock` committed and consistent (`--locked` passes); GitHub Actions pinned by full commit SHA with minimal `permissions`; no `pull_request_target` with checkout of untrusted code.

**Docs and housekeeping**: a doc comment's behavioural claim names the test that pins it, and that test exists; stale docs are findings; no unrelated files; commit subject `<ID>: summary` with a `Signed-off-by:` line.

## Output

Findings only, most severe first. Severity: **HIGH** (wrong behaviour, security, broken invariant, rule loosened or suppressed), **MEDIUM** (missing test, missing ADR, doc drift, likely bug), **LOW** (style, naming, nit).

```
## Review <range> (<n> commits, tasks <ids>)
gate: <command> pass|fail (<test count>)

### HIGH
- crates/x/src/file.rs:42: what is wrong; why it matters; suggested fix direction
### MEDIUM
- ...
### LOW
- ...

Verdict: CLEAN | CHANGES REQUESTED (<n> HIGH, <m> MEDIUM)
```

Omit empty severities. A clean range gets an explicit `Verdict: CLEAN` with one line on what was checked. Any HIGH means CHANGES REQUESTED.

## After the review

A checkpoint task ends with a local annotated tag `checkpoint-<ID>` (e.g. `checkpoint-S9`). The orchestrator creates it, only once the review has passed (clean, or every HIGH and MEDIUM fixed), and never pushes it. This skill does not tag.
