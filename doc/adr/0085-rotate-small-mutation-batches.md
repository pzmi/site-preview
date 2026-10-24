# 0085. Rotate small scheduled mutation batches

Date: 2026-10-18

## Status

Proposed

## Context

ADR 0068 runs the complete mutation scope weekly. The recorded 497-mutant baseline
needed about 52 minutes with two workers; the current scope has about 500 mutants.
The maintainer requested fewer mutants per session after a rerun exceeded 25 minutes.
A long timeout permits a full analysis but does not make it useful as a short session.

## Decision

Use cargo-mutants 27.1.0's existing zero-based `--shard K/N` support. Expose it through
`cargo xtask mutation`, validate it before execution, and label subset summaries explicitly.
Keep local invocations without `--shard` as full runs over their selected scope.

The weekly workflow runs one of ten consecutive slices, selected by UTC epoch week
modulo ten. At the current scope this is about 50 mutants. Manual dispatch accepts
`auto` (the same weekly shard), `full`, or an explicit `K/N`. Manual runs and retries
do not advance rotation. Keep the upstream slice strategy for incremental-build locality.

Limit batch mutation steps to 20 minutes and batch jobs to 30 minutes. Full manual
runs allow 330 minutes for mutation and 350 for the job. Retain summary/report upload
on step failure; an incomplete run still fails. Do not change the per-mutant timeout,
mutant exclusions, test selection, score formula or finding exit-code policy.

## Consequences

Scheduled sessions do roughly one tenth of the previous mutation work, plus the
baseline build/test and setup costs. This bounds mutant count relative to the current
scope, not wall time or an absolute count as the project grows.

Full coverage takes ten scheduled runs with an unchanged mutant list. Source changes
can move mutants between slices; no complete or current baseline score is inferred
from partial reports. Use a manual full run to refresh the baseline or `--in-diff` for
focused checks. Full weekly analysis from ADR 0068 is replaced by rotating weekly batches.
