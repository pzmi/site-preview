# 0081. ADR allocation and landing command

Date: 2026-10-14

## Status

Accepted (owner, 2026-10-14)

## Context

Parallel lanes picked the same "next free" ADR number twice, and merging their index edits
duplicated and disordered rows of `doc/adr/README.md`, which the `docs` stage (ADR 0077) then
failed on. Landing lane commits on `main` ran through an untracked orchestrator script that
resolved every conflict by deleting conflict markers, kept the integration side of
`Cargo.lock`, ran the gate and pushed to a session branch it assumed.

## Decision

- `cargo xtask adr new "<title>" [--number N] [--date YYYY-MM-DD] [--status TEXT]` writes
  `doc/adr/NNNN-<kebab-title>.md` from `template.md` and inserts its index row in order. The
  number is `--number` (reserved by the orchestrator; refused when a file or row uses it), else
  one above the highest number used by a file or a row; gaps are never reused.
- `cargo xtask adr index` compares the rows with the files; `--fix` rewrites them, one per file,
  sorted, with the file's title and a status derived from its Status paragraph: the first word,
  `Superseded by NNNN`, then `Amended by NNNN, ...` for the ADRs named after "amended by" and
  `Amended YYYY-MM-DD` for a dated amendment that names none. Prose around the table is kept;
  conflict markers inside the table are dropped, outside it are an error.
- `cargo xtask land <ref>... (--push <remote> <branch> [--also <branch>]... | --no-push)
  [--log <path>]` replaces the script. It refuses unless the checkout is on the branch, clean,
  with no operation in progress, and equal to the remote branch after a fetch. It cherry-picks
  on a detached `HEAD`; only `Cargo.lock` (integration side, then `cargo update -w`) and the ADR
  index (`adr index --fix`) are resolved automatically, any other conflict aborts and names the
  files. After `cargo update -w` it runs `cargo xtask check`, fast-forwards the branch only when
  green, and pushes only then, never forced, retrying apparent network failures after 2, 4, 8
  and 16 seconds. Every program runs as `Command` with arguments, never through a shell.

## Consequences

Numbers and index rows are no longer hand-allocated or hand-merged, and a red gate or an
unresolvable conflict leaves `main` untouched. The derived status column drops free-text
amendment notes in favour of numbers and dates. Owner: FND-05 (tooling lessons).
