# 0052. Quint toolchain: lockfile-only evaluator; the lifecycle model's two step relations

Date: 2026-10-12

## Status

Accepted (owner, 2026-10-14)

## Context

FORMAL-01 pins Quint (ADR 0010) and retrofits `lifecycle.qnt` onto the code (ADR 0014).
Quint 0.33.0's `run` and `test` default to a Rust evaluator that is downloaded from GitHub at
first use (v0.7.0, outside any lockfile, refused by the sandbox proxy); foundation.md §4 says
CI must not fetch an unpinned binary. `npx quint` would also fetch Quint from the registry
when it is not installed. `quint test` exits 0 when `--match` selects nothing. And the code
differs from foundation.md §6.1's wording: a watcher failure during start-up is degraded
serving, not `Failed`; refusals before `Start` leave the machine `Stopped`; start-up is
synchronous, so the domain machine's stop-during-start-up (`Stopping { awaiting }`) states
are unreachable through `Server` (ADR 0051); the drain, and the forced exit at a second
signal, happen inside `JoinTasks` (ADR 0045).

## Decision

- `formal/package.json` pins `@informalsystems/quint` 0.33.0 exactly, `package-lock.json`
  is committed and `.npmrc` sets `ignore-scripts=true` (Quint needs no install script).
- xtask runs Quint as `npx --prefix formal --no -- quint`, never fetching it, and always with
  `--backend typescript`, so the evaluator is the locked package. A missing Node or Quint
  fails with its install command (`npm ci --prefix formal`).
- `cargo xtask conformance` runs `formal-typecheck` (alone with `--typecheck`),
  `formal-scenarios` (each module of `scenarios.qnt`, matching and counting its `run`
  names) and `formal-simulate` (every conjunct of each model's `allInvariants`, under each
  step relation, 1000 traces of 25 steps, seed printed, `QUINT_SEED` repeats it). None is
  in `check` until FORMAL-09 registers `conformance` (ADR 0019).
- `lifecycle.qnt` mirrors `LifecycleState::step` row for row and counts resources by the
  inputs that acquire them and the effects that release them. `step` is the domain machine's
  contract (any accepted input, including stops during start-up); `serverStep` is the
  composition's `Server` (synchronous `startServer`, `refuseStart`, blocking `stopServer`),
  which FORMAL-05 replays. Clients send requests and open streams in both. The drain and the
  forced exit stay below the model. Scenarios have one module per relation.

## Consequences

- Simulation is slower than with the Rust evaluator (about 150 traces/s here); a faster
  evaluator needs its own pinned, checksummed download.
- `npm audit` reports adm-zip advisories through Quint; Quint uses it only to unpack
  downloads (Apalache, the Rust evaluator), which this lane never makes. FORMAL-10 revisits.
- FORMAL-04's ITF decoder needs `{"tag", "value"}` variants and `#tup`; `--mbt` adds
  `mbt::actionTaken` and `mbt::nondetPicks` (option variants).
