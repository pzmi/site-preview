# Architecture

How site-preview is divided into crates, which way dependencies may point, who owns what,
and how to add a crate. The rules that keep this true are cataloged in
[`architecture-rules.md`](architecture-rules.md) and enforced by `cargo xtask architecture`
from [`architecture.toml`](../architecture.toml). The reasoning is in
[`foundation.md`](foundation.md) section 2 and [ADR 0002](adr/0002-multi-crate-workspace.md);
product behaviour stays in [`plan.md`](plan.md).

## Crate map

| Crate | Category | Responsibility |
|:--|:--|:--|
| `preview-domain` | production | Pure policies, transitions and invariants; no I/O. |
| `preview-application` | production | Commands, queries, use cases and port traits. |
| `preview-adapter-fs` | production | Capability-scoped filesystem access and the file watcher. |
| `preview-adapter-http` | production | HTTP handlers, HTML injection and the live-reload transport. |
| `preview-composition` | production | The only wiring root; builds adapters and runs the server. |
| `preview-cli` | production | The `site-preview` binary: flags, exit codes, delegation. |
| `preview-testkit` | test only | Reusable handwritten fakes and the bounded poll helper. |
| `preview-conformance` | test only | Drives the real code from Quint traces. |
| `xtask` | tool only | Automation and the architecture checker. |

Under `crates/` and `tools/xtask`. Categories are the `[workspace]` tables of
`architecture.toml`; each crate is in exactly one.

## The dependency law

Source-code dependencies point inward. Data and events may flow the other way.

```text
preview-cli ──► preview-composition
                     ├──► preview-adapter-fs ────► preview-application ──► preview-domain
                     ├──► preview-adapter-http ──► preview-application
                     └──► preview-application
```

The exact allowed edges are `[dependencies.normal]` in `architecture.toml`:

| Crate | May depend on (normal edges) |
|:--|:--|
| `preview-domain` | nothing in the workspace |
| `preview-application` | `preview-domain` |
| `preview-adapter-fs` | `preview-application`, `preview-domain` |
| `preview-adapter-http` | `preview-application`, `preview-domain` |
| `preview-composition` | `preview-domain`, `preview-application`, `preview-adapter-fs`, `preview-adapter-http` |
| `preview-cli` | `preview-composition` |
| `preview-testkit` | `preview-domain`, `preview-application` |
| `preview-conformance` | `preview-domain`, `preview-application`, `preview-testkit`, `preview-adapter-fs`, `preview-adapter-http` |
| `xtask` | nothing in the workspace |

Consequences the checker enforces:

- Adapters never name a sibling adapter or composition; the CLI never names a concrete
  adapter (`ARCH-forbidden-edge`, `ARCH-layer-isolation`).
- A production crate never reaches a test-only or tool-only crate over normal or build
  edges, directly or transitively (`ARCH-production-closure`).
- A dev-dependency may target a test-only crate or a crate in the dependent's normal list
  (`ARCH-forbidden-dev-edge`). Workspace build edges need an entry in `[dependencies.build]`
  (`ARCH-undeclared-build-dep`).
- Third-party crates are allowed per crate, per dependency kind and per feature in
  `[external.*]` (`ARCH-unlisted-external`, `ARCH-external-feature`); every package comes
  from crates.io unless an ADR-backed source exception says otherwise (`ARCH-external-source`).

## Ownership per crate

- **Domain** owns value objects, validated identifiers, policy decisions, typed domain
  events, transition logic, invariants and refusal reasons. It performs no disk, network,
  process, environment, clock, randomness, runtime or HTTP access
  (`ARCH-domain-purity`, [ADR 0012](adr/0012-security-policies-are-pure-domain-code.md)).
  Time and nondeterminism are explicit inputs.
- **Application** owns use cases, command and query types, port traits and deterministic
  orchestration. It names no adapter technology; no HTTP or watcher type escapes an adapter.
- **FS adapter** owns the root-directory capability, byte reading, metadata and the watcher.
  Only it, plus the bootstrap file of composition, may obtain ambient filesystem authority
  (`ARCH-fs-authority`, [ADR 0004](adr/0004-rustix-openat-capability.md)).
- **HTTP adapter** owns routes, handlers, content negotiation, HTML injection and the SSE
  transport. It converts requests into application commands and queries and holds no
  domain policy.
- **Composition** constructs dependencies, selects configuration, wires tasks and
  cancellation, and owns the async runtime and process resources
  ([ADR 0028](adr/0028-composition-owns-runtime-and-processes.md)). It is not a service
  locator for the domain.
- **CLI** parses flags, maps outcomes to exit codes, picks loopback defaults and delegates
  through composition.
- **Testkit** owns fixed clocks, scripted change sources, in-memory content stores,
  recording sinks and the one sanctioned sleep, `wait_until` in `src/poll.rs`.
- **Conformance** interprets Quint stimuli, drives real operations and compares observable
  state with the model.
- **xtask** owns the commands in `cargo xtask list-checks`, including the architecture checker.

`unsafe` code is forbidden everywhere (`ARCH-unsafe-code`).

## Ports

Ports are trait definitions in `preview-application/src/ports/`, implemented by adapters
and by testkit fakes (foundation.md section 2.3: a trait only at a nondeterministic or
replaceable boundary). Their shapes are [ADR 0038](adr/0038-application-port-shapes.md);
the task names come from `doc/tasks.md`.

| Port | Shape | Real implementation | Fake |
|:--|:--|:--|:--|
| `SiteContent`: open a validated path as a file or a directory with an optional index | sync, `&dyn`; defined by APP-01 | `preview-adapter-fs` (FS-01, FS-02) | testkit in-memory store (TEST-03) |
| `ChangeSource`: next batch of events, overflow, failure or close | async, cancellation safe, generic; defined by APP-01 | `preview-adapter-fs` watcher (FS-05..08) | testkit scripted source (TEST-03) |
| `Clock`: current tick | sync; defined by APP-01 | composition | testkit fixed clock (TEST-03) |
| Notification publication to connected pages (the `ReloadState` latest-state channel) | planned by APP-02 | application, served by `preview-adapter-http` | testkit recording sink (TEST-03) |

Application-level queries and commands planned alongside them: `ResolveRequest`
(APP-03) and the change commands record, settle, overflow and health (APP-04). Each port's
doc comment states its contract; the contract harnesses (TEST-04, TEST-05) run it against
every implementation.

## Adding a crate

Do all of this in one commit.

1. Create `crates/<name>/Cargo.toml` and `src/lib.rs` (or `main.rs`) with a crate-level doc
   comment stating the single responsibility. Set `[lints] workspace = true` and
   `publish = false` unless it is the published binary.
2. Add the path to `members` in the root `Cargo.toml`. Members are explicit; a crate on disk
   that is not a member fails `ARCH-not-a-member`.
3. Put the crate in exactly one `[workspace]` category in `architecture.toml`
   (`ARCH-unknown-crate`, `ARCH-duplicate-category`).
4. Add its `[dependencies.normal]` entry (empty list if none) with only the edges the law
   allows, and `[dependencies.build]` entries if any.
5. Add `[external.normal]`, `[external.dev]` and `[external.build]` entries for each
   third-party dependency with its features, added with `cargo add` and the reason in the
   commit message.
6. If a rule names the crate (`ARCH-missing-selector`), check the rule's selectors; do not
   list it under `[semantic] absent`.
7. Update the tables in this file and, if the crate changes the law or a rule, add or
   supersede an ADR.
8. Run `cargo xtask quick` and `cargo xtask architecture`; commit `Cargo.lock` with the change.

Related: [`architecture-rules.md`](architecture-rules.md) (catalog, adding a rule,
requesting an exception), [`adr/README.md`](adr/README.md).
