# 0041. Workflow pin parsing, secret-scan preconditions and the unmaintained-crate scope

Date: 2026-10-11

## Status

Accepted (owner, 2026-10-14)

## Context

The APP-04 checkpoint review of the supply-chain stages (SEC-01, SEC-02) found three gaps:

- The action-pin check matched `uses:` by line prefix. A flow mapping (`- {uses: x@v4}`), a
  quoted key (`"uses":`), a space before the colon (`uses :`) and a job-level reusable
  workflow all passed unchecked, and `owner/repo@v4#@<sha>` passed because the ref was taken
  after the last `@`.
- `gitleaks git` exits 0 outside a git repository and scans only what a shallow clone holds;
  the pin check and the Gitleaks config were resolved from the current directory, so a run
  from a subdirectory checked nothing and passed.
- `deny.toml` said unmaintained crates "are reported but do not fail". Under cargo-deny 0.18
  and later, `unmaintained` is a scope: `"workspace"` fails on an unmaintained crate that a
  workspace member depends on directly and does not report one that is only transitive.

## Decision

- The pin check parses each workflow and local action as YAML with `yaml-rust2` (default
  features off, so no `encoding_rs`), building its own tree from the parser's marked events
  so every key keeps its line and column and duplicate keys are all kept. It walks
  `jobs.<id>.uses`, `jobs.<id>.steps[*].uses` and `runs.steps[*].uses`. It fails closed: a
  line holding the token `uses` more often than those walked keys and values account for is
  an error, and so are YAML merge keys and parse errors. A ref is pinned only when it is a
  local `./` path, `docker://<image>@sha256:<64 lowercase hex>`, or
  `owner/repo[/path]@<40 lowercase hex>` where the ref is everything after the first `@`.
  `yaml-rust2` was chosen over `serde_yaml` (unmaintained), `serde_yml` (deprecated) and
  `saphyr` (its successor, which pulls `ordered-float`, `thiserror` and a separate parser
  crate for spans this check builds itself).
- xtask resolves the repository root from its own manifest location (`CARGO_MANIFEST_DIR`,
  two levels up), never the current directory. The pin check fails when
  `.github/workflows` is missing or holds no workflow.
- The secret scan runs only when the root is the top of a git work tree, the clone is not
  shallow and the history has at least one commit; each refusal says how to fix it. It fails
  when Gitleaks does not report a positive number of scanned commits.
- `unmaintained = "workspace"` is kept on purpose: an unmaintained direct dependency fails
  the `deny` stage, where it is our choice to replace it. An unmaintained crate reached only
  transitively is not reported by the stage; it is ours to act on only through the direct
  dependency that pulls it in. Vulnerabilities and yanked crates fail at every depth.
- The cargo-deny stage is named `deny`, so the catalog id `security` names only the
  `cargo xtask security` command.

## Consequences

- A comment or script line in a workflow that mentions `uses` as a word fails the check and
  must be reworded. Composite actions and reusable workflows are covered by the same walk.
- The secret scan cannot run from a source archive or a `fetch-depth: 1` checkout; CI
  already checks out with `fetch-depth: 0`.
- An unmaintained transitive crate can enter the lock file without failing the gate;
  Dependabot and review of lock-file changes remain the defence there.
- xtask gains `yaml-rust2` and its dependencies `arraydeque`, `hashlink` and `foldhash`
  (MIT/Apache-2.0 and Zlib, all allowed by `deny.toml`).
