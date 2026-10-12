# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project

site-preview is a zero-config local live-reload server for static sites, in Rust: run
`site-preview serve .` in a directory with an `index.html`, and the browser reloads when a file
changes. One binary, no Node at runtime, loopback by default, source files never modified (the
reload script is injected into served responses only). It is a public, open-source repository.

## Docs

| File | What it holds |
|:--|:--|
| `doc/plan.md` | Product spec: CLI (§3), safe serving (§5), HTML injection (§6), reload protocol (§7), watcher (§8), lifecycle (§9) |
| `doc/foundation.md` | Engineering rules: workspace and dependency law (§2), enforcement (§3), tests (§5), Quint conformance (§6), commands (§9), agent contract (§11) |
| `doc/formal-models.md` | Quint models, stimuli, observations, invariants, scope, the conformance gate, promoting a counterexample; model first (D13) |
| `doc/tasks.md` | Task order, dependencies, model per task, ⛳ checkpoints, the live gate, decisions |
| `doc/adr/` | Decisions, Nygard style (`NNNN-kebab-title.md`: Status, Context, Decision, Consequences); index in `doc/adr/README.md` |

foundation.md names `docs/`; this repo uses `doc/`. Decisions in `doc/tasks.md` and Accepted
ADRs override earlier plan text.

## Workflow

- `/orchestrate [ids|lane]`: run ready tasks through background agents in worktrees, merge,
  gate, push, watch CI, stop at ⛳ checkpoints. The orchestrator never implements.
- `/work <ID>`: one task through intake, plan, implement, test, polish, review, qa, ship,
  babysit. State in `.work/<ID>/` (gitignored).
- `/review [range]`: read-only review; default range since the newest `checkpoint-*` tag.

## The gate

```bash
cargo xtask quick     # fmt check, clippy, fast unit/component tests: while working
cargo xtask check     # the full blocking gate (foundation.md §9): before every commit
cargo xtask architecture   # crate/module policy and its negative fixtures
cargo xtask list-checks    # every check, machine-readable
```

Before `xtask` exists, the gate is:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
```

`doc/tasks.md` says which gate is live. `quick` is not `check`; never report one as the other.
Once an `xtask` subcommand covers a repeatable job, use it instead of an ad-hoc command; a new
repeatable job becomes an `xtask` subcommand. A missing tool is an actionable failure, never a
skipped-but-green check.

## Enforced rules

- **Dependency law** (foundation.md §2.1, §3.2): cli → composition → adapters → application →
  domain. Adapters never reference sibling adapters or composition. Production crates never
  depend on testkit or conformance. A new crate gets its workspace `members` entry and its
  `architecture.toml` entry in the same commit. An architecture failure is fixed by fixing the
  structure, never by loosening `architecture.toml` (unless the task says so, with an ADR).
- **Pure domain** (§2.2, §3.3): no `std::fs`, `std::net`, `std::process`, env, clock,
  randomness, tokio or HTTP in `preview-domain`.
- **Language** (§3.3, §4): `unsafe_code` forbidden; no `todo!`, `unimplemented!`, `dbg!`; no
  `#[ignore]`; no `#[allow(...)]` without an ADR; zero clippy warnings.
- **Tests** (§5): `should_<outcome>_when_<condition>` with `// given`, `// when`, `// then`; no
  sleeps in deterministic tests (bounded condition polling only in OS-watcher tests, with the
  reason); real objects or testkit fakes, no mocking frameworks; dynamic ports, tempdirs.
- **Model first** (§6.2): a lifecycle, watcher or reload change updates the Quint model,
  scenarios and conformance driver before the Rust code. Never remove an invariant to get green.
- **Security** (plan.md §5): one decode, component deny list, per-component `openat` with
  `O_NOFOLLOW`; never `canonicalize` + `starts_with`; Host/Origin checks (§5.4); no shell
  interpolation; no secrets, absolute paths or query strings in logs.
- **Dependencies**: `cargo add`, named with its reason in the commit message; `Cargo.lock`
  committed, builds `--locked`; GitHub Actions pinned by commit SHA.
- **Decisions**: a change that adds or reverses an architectural decision adds an ADR (next
  free number) or supersedes one, in the same commit.

## Commits

- `git commit -s` (DCO sign-off). Subject `<ID>: summary`, imperative, under 72 chars; the
  body says why. Use the trailer block your instructions give, exactly; add none of your own.
- Stage only your own files (never `git add -A`). Never `--no-verify`, never force-push, never
  push tags; `checkpoint-*` tags are local.
- No model ids in repo content (code, docs, commit message bodies). The attribution trailer block the
  session supplies (`Co-Authored-By`, `Claude-Session`) is the only exception.

## Sandbox

- Chromium for Playwright is preinstalled (`PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`). Never
  run `playwright install`.
- Node is for dev-time Quint only (`formal/`); the shipped binary never needs it.
- `gh` may be unavailable: use the GitHub MCP tools (`actions_list`, `actions_get`,
  `get_job_logs`) for CI.
- Hooks: SessionStart installs the pinned toolchain and fetches crates (remote sessions);
  every edited `*.rs` is rustfmt'ed; Stop runs the fast gate when Rust files changed.
- Temporary files go in the session scratchpad, never `/tmp` or the repo.

## Public repository

Never commit secrets, tokens, credentials, internal hostnames, private-repo content or personal
paths. Fixtures and examples use synthetic content.
