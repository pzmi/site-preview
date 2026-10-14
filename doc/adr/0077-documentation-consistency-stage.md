# 0077. Documentation consistency stage: a warning-code registry and the `docs` checks

Date: 2026-10-13

## Status

Proposed (planner default — owner may override).

## Context

Documentation drifts silently. Earlier merges left `doc/adr/README.md` with rows listed twice,
rows out of order and titles that no longer matched their files, and ADR number clashes had
to be fixed by hand. The README tables of `serve` flags and warning codes, the rule catalog in
`doc/architecture-rules.md`, the `cargo xtask` commands in prose and `AGENTS.md` (which the
agent workflow spec replays) each have a source of truth in code, and nothing compared them.

The warning codes had no single source: eleven string literals and constants in five crates,
printed as `warning[<code>]` (ADR 0020). A check that greps emit sites would miss any new
style of call.

## Decision

- **A `docs` stage** (`tools/xtask/src/docs.rs`, task DOC-05) is in `check` and runs in the
  `static` CI job. It reports `path:line: message` for the first drift of each of: the ADR
  index against `doc/adr/*.md` (every file listed once, in ascending order, with the file's
  title and status; the number in the title and the link matches the file name; no number
  used twice; a status that starts with `Proposed`, `Accepted` or `Superseded by NNNN`,
  followed by free annotation such as `. Amended by 0069`); every rule id of `rules.rs` in
  `doc/architecture-rules.md`, and no unknown `ARCH-*` id there; every `cargo xtask <cmd>` in
  a code span or fence of any Markdown file is a clap subcommand; every flag of
  `site-preview serve --help` (read from the binary built from the tree) in the README flags
  table and no other flag there; every warning code in the README Warnings table and no
  other; every relative Markdown link, and `#anchor` into a Markdown file, resolves; the
  `sh` commands of `AGENTS.md` are the rows of the agent workflow spec's `EXERCISED` map and
  comment table. Gaps in ADR numbers are allowed (a number may be reserved by a concurrent
  task).
- **Warning codes have one registry**, `preview-domain::warning_code`, a macro that declares
  each constant and the `ALL` list together. Production code names a code only through it
  (re-exported by `preview-composition` for the CLI, which may not depend on the domain). A
  code is a `WarningCode` newtype with a private field, so only the registry macro can build
  one and `WarningSink::warn` and `warning_line` accept nothing else: a literal does not
  compile, which replaced an earlier scan of call sites that a local constant or a tuple
  could slip past. The stage reads the registry file and compares it with the README table.
- **Which files**: Markdown files come from `git ls-files --cached --others --exclude-standard`
  (tracked, plus new and not ignored, so a document is checked before `git add`), and from a
  directory walk outside git. The repository run requires `doc/adr` with at least one ADR;
  only the fixture trees may have none.
- **Fixtures**: `tests/docs-invalid/<case>/` are small trees with an `expected.txt` listing
  the exact diagnostics (an unindexed ADR, an unknown xtask command, a broken link and
  anchor, a duplicate ADR number, an invalid status, a title number that differs from the file
  name); `tests/docs-valid/` holds a tree that must produce none. The stage and a unit test
  run both. The fixtures are not part of the documentation the stage scans.

## Consequences

- A new ADR is added to the index in the same commit with the file's exact title and status,
  or `check` fails; a status change in a file is mirrored in the index.
- A new warning is a line in `warning_code.rs` and a row in the README table.
- A renamed or removed `serve` flag, xtask command, rule id or documented path fails the gate
  instead of leaving stale prose.
- The stage builds `preview-cli` (a debug build) to read its help; the `static` job already
  holds a warm cache for the workspace.
