# 0061. Apalache in a scheduled `formal-verify` lane

Date: 2026-10-13

## Status

Proposed (planner default — owner may override)

## Context

foundation.md section 6.4 asks the scheduled lane for bounded verification of the Quint
invariants through a pinned Apalache and a larger multi-seed simulation. ADR 0052 kept every
download out of the pull-request gate (the TypeScript evaluator only) and left Apalache and
the `adm-zip` advisories to this task. Running `quint verify` showed three things:

- Quint fetches `https://github.com/apalache-mc/apalache/releases/download/v<version>/apalache.tgz`
  into `~/.quint/apalache-dist-<version>` on first use, unpacks it with `tar` (not `adm-zip`,
  which only the Rust-evaluator download uses), and does not check a checksum.
- `lifecycle.qnt` failed with an empty `error:` after Apalache's parser pass: its translation
  overflows the default JVM thread stack on the nested `match` of `transition`. `-Xss64m`
  fixes it. Then its pass that inlines definitions ran out of a 3 GB heap, because
  `startRun` nested three `startTurn` calls as arguments and inlining copies an argument at
  every use. Binding the turns one by one (`val m1 = ...`) removes the growth; the model's
  behaviour is unchanged (`formal-freshness` still agrees with every committed trace).
- Apalache's default search re-checks invariants along every path of an exponential tree.
  Checking them once after each step (`search.invariant.mode = after`) took a third of the
  time at the same depth, with the same verdict.

## Decision

- `cargo xtask formal-verify` (not in `check`; catalog rows `formal-verify`,
  `formal-verify-bounded`, `formal-verify-simulate`, `in_check = false`) runs:
  `quint verify --invariants <every conjunct of allInvariants>` for each model and step
  relation to the depth in `BOUNDED`, with `--apalache-version` taken from
  `tools/versions.toml` and the tuning above; then `quint run` over three seeds of 5000
  traces of 50 steps per relation.
- JVM runs get `-Xss64m -Xmx3g` appended to the caller's `JVM_ARGS`, at most two at a time,
  each on its own port, in `target/formal-verify/` (logs, Apalache run directories with a
  counterexample as ITF and TLA+, the ITF of a failing simulation).
- A missing or too old (below 17) JVM, or an Apalache that cannot be downloaded, fails with
  the fix. A failure is reported as a counterexample only when Quint says `[violation]`;
  anything else is a tooling failure and verifies nothing.
- `formal.yml` gains the `formal-verify` job (schedule and manual dispatch, 120 minutes,
  `contents: read`, the Apalache release cached) that uploads `target/formal-verify/` on
  failure. TEST-09 adds its Miri job beside it.
- Liveness is not scheduled: see `doc/formal-models.md`, "Verification baseline".

## Consequences

- Depths are small (the baseline table gives them and the durations): Apalache covers every
  nondeterministic choice up to the depth, not a sample, but the cost grows about tenfold per
  step in these models. The simulation covers depth; Apalache covers breadth. Raising a depth
  needs a measured duration in the baseline.
- The Apalache release is downloaded unchecked, in a job with no secrets and a read-only
  token. Pinning the SHA-256 of `apalache.jar` and checking it before a run is a follow-up if
  the lane ever gets more privilege.
- A model written with deeply nested pure calls can pass `quint run` and still not translate;
  `formal-verify` is where that shows.
