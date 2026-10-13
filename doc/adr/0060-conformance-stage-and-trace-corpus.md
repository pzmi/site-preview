# 0060. The `conformance` stage in `check`, the trace corpus manifests and `formal.yml`

Date: 2026-10-12

## Status

Proposed (planner default — owner may override). Amended by 0069.

## Context

FORMAL-09 completes foundation.md section 6.4: regression freshness, generated traces with a
recorded seed, Quint version, family, count and length, replay of both sets, the negative
controls, the stage registered in `check` (D18, ADR 0019), and a `formal.yml` workflow. Before
it, the parameters that regenerate each committed trace (its `--init` and `--max-steps`) lived
only in comments in `formal/*_traces.qnt`, the decoder fixture's command lived only in a
commit message, every replay test generated its own traces with its own seed, and the replay
and controls ran inside `unit-tests` (so `quick` needed Quint).

## Decision

- **Regression manifest:** `formal/traces/regression/manifest.toml` lists each committed
  trace with its model, `main`, `init`, step, `max_steps`, seed and invariant. TOML rather
  than JSON: the file is reviewed by hand and takes comments, and xtask already parses TOML
  (no new dependency). Unit tests in `tools/xtask/src/conformance.rs` require exactly one
  entry per committed `*.itf.json` and per script of a `*_traces.qnt` module, with the length
  and initial action that the script's doc comment states; unknown keys are refused.
- **`formal-freshness`** regenerates every entry from the repository root (the ITF records
  the model path as given, `formal/<model>`) into `formal/traces/generated/regression/` and
  compares the files byte for byte after blanking the top-level `#meta.description` and
  `#meta.timestamp` (Quint's creation date, ADR 0053). Nothing else is normalized. A
  difference fails with `stale regression trace formal/traces/regression/<file>` and names
  the regenerated copy to review and commit. A stale trace is often invisible to the replay:
  a ghost or browser field the driver does not observe (D16) can drift unnoticed.
- **`formal-generate`** writes 30 traces of 25 steps for each driver's family (lifecycle
  `serverStepMostlyBound`, reload `step`, watcher `step`) with one seed per xtask run (shared
  with `formal-simulate`, printed, `QUINT_SEED` repeats it), and records the seed, Quint
  version, backend and per family the model, main module, step, invariant, count, maximum
  steps, status and files in `formal/traces/generated/manifest.json`. A failed family is
  recorded as `failed` and its traces are kept.
- **Replay once per gate.** `unit-tests` (and `quick`) run `--exclude preview-conformance`.
  `conformance-replay` runs that crate's tests except the controls with
  `CONFORMANCE_GENERATED_MANIFEST` set, so the replay tests read their family from the
  manifest (refusing one whose parameters differ from the test's) instead of generating;
  `conformance-controls` runs `binary(negative_controls)`. Run directly, the tests still
  generate their own traces and remove the stale manifest.
- **`check` runs all seven stages** after the `test` stages and before `security`. Quint runs
  within a stage go in parallel, one per core, so the stage costs about 80 s on four cores
  instead of about 140 s run one at a time.
- **`formal.yml`** runs `cargo xtask conformance` on push to `main`, pull requests, nightly
  (fresh seed) and on demand, as the `Conformance (Quint)` job of foundation.md section 10.
  On failure it uploads `formal/traces/generated/` (the generated traces, their manifest and
  the regenerated regression traces) and nextest's JUnit report. Permissions are
  `contents: read`; actions are pinned by SHA (upload-artifact v7.0.1, from April 2026; the
  newer v7.0.2 was five days old). A `scheduled` job is a placeholder that checks nothing,
  for FORMAL-10 (`formal-verify`) and TEST-09 (Miri).

## Consequences

- `check` now needs Node and `npm ci --prefix formal`, as CI already provided. `quick`
  no longer does.
- Pull requests run the conformance stages twice, once in ci.yml's `check` and once in
  formal.yml (about 2 minutes of runner time). OPS-04 can choose which job is required.
- A change to a model or script that changes a trace fails `formal-freshness` until the
  regenerated trace is reviewed and committed, together with any catch point it moves in
  ADR 0059's table.
- From FORMAL-09 on, the model-first workflow of foundation.md section 6.2 is mandatory
  (D13); `doc/formal-models.md` describes the models, stimuli, observations, invariants,
  scope and how to promote a counterexample.
- Coverage (TEST-06) must include `preview-conformance`'s replay, which `unit-tests` no
  longer runs, or the conformance-only paths of the real code drop out of the measurement.
