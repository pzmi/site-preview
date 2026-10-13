# 0061. Apalache in a scheduled `formal-verify` lane

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Amended 2026-10-13 (checkpoint FORMAL-10
review): the Apalache release is verified by its SHA-256 before it runs; see "Amendment".

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
- The Apalache release was first downloaded unchecked by Quint. The amendment below replaces
  that with a pinned digest.
- A model written with deeply nested pure calls can pass `quint run` and still not translate;
  `formal-verify` is where that shows.

## Amendment (2026-10-13): a pinned SHA-256 for the Apalache release

The checkpoint review found that Quint downloads `apalache.tgz` without a checksum and that
`formal.yml` cached `~/.quint`, so a poisoned cache entry or a replaced release asset would run
unverified JVM code in the lane. Quint itself offers no digest option.

- `tools/versions.toml` pins `sha256` beside `tools.apalache.version`: for 0.62.1 (the version
  Quint 0.33.0 pins as its default), `38202a8c4ebeb3ab73e0fe5a7e4bb948442d46ad5de8a7fb45c9e5cac2beb2a6`,
  computed from the real download of
  `https://github.com/apalache-mc/apalache/releases/download/v0.62.1/apalache.tgz`
  (192,000,835 bytes; its unpacked files are byte-identical to the tree Quint had unpacked from
  the same URL the day before). A new version needs a new hash, computed the same way.
- Before Quint runs, `formal-verify` (stage `formal-verify-bounded`) fetches the tarball with
  `curl --proto =https` into `target/apalache/`, or reuses one already there, hashes it, and
  fails with both digests when the hash differs. It then removes any existing
  `apalache-dist-<version>/apalache` and unpacks the verified tarball into a fresh directory
  there (under `$QUINT_HOME`, else `~/.quint`, where Quint looks), moved into place in one
  rename, so Quint always finds the binary and never downloads. What runs is unpacked from a
  verified tarball on every run; an unpacked tree is never trusted on its own.
- `formal.yml` no longer caches `~/.quint`. The tarball may sit in the cached `target/`, which
  is harmless because it is hashed before every use.
- `doctor` probes the binary at that location instead of `apalache-mc` on `PATH`.
- The release workflow (OPS-06) must restore no caches at all, so that no cached tool or
  build output reaches a release artifact; the digest check here protects only this lane.

