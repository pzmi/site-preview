# 0078. The release workflow: tag-run only, drafts only, write in one job, dry runs on `dry-run-*` tags

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Implements D23 (ADR 0024) for releases;
keeps D24 (ADR 0025, no crates.io) and ADR 0061 (no caches in the release workflow).

## Context

OPS-06 asks for `release.yml`: on a `v*` tag, four archives built on native runners, `SHA256SUMS`,
a CycloneDX SBOM, build-provenance attestations, a draft release, and a `verify` job that tests
the published archives on each OS. Its acceptance also asks for a `workflow_dispatch` dry run on
a test tag. Three things constrain the design:

- A dispatched run's provenance names the ref and commit the workflow ran from. A dispatch from
  `main` that checks out another tag would attest a commit it did not build.
- A draft release is visible only to a token that can write contents, so a read-only `verify`
  job cannot download its assets.
- `cargo cyclonedx` writes one SBOM per workspace member, for the host target by default, with
  `bom-ref`s that contain the checkout's absolute path.

## Decision

- **Triggers:** `push` of `v*` tags, and `workflow_dispatch` with a required `tag` and a `dry_run`
  boolean (default `true`). No pull-request trigger. Every run must run *from* the tag
  (`--ref <tag>`) and the `tag` input must equal that ref, so the provenance's source is the tag's
  commit. `prepare` validates in the shell from environment variables only, against fixed
  patterns, before printing anything: a release tag is `v<semver>`, equal to `v` + the workspace
  version, on a commit of `main`; a dry-run tag is `dry-run-<name>`. The name decides the mode:
  `dry_run` true with a `v*` tag, or false with a `dry-run-*` tag, is refused.
- **Drafts only, in both modes.** `gh release create --draft --verify-tag` is the workflow's one
  release write: it never publishes, `--verify-tag` stops gh from creating a tag, and an existing
  release for the tag (draft or not) fails the job instead of receiving assets. A dry run's draft
  is also a pre-release titled `DRY RUN <tag> (delete, never publish)`. A dry run therefore
  exercises every step, attestations included, and the result stays private until deleted.
  Uploading workflow artifacts only was rejected: it would not test the release step or `verify`.
  Publishing is the owner's manual step after `verify` is green ([`release.md`](../release.md)).
- **Permissions:** `permissions: {}` at the top; `contents: read` in `prepare` and `build`;
  `contents: write`, `id-token: write` and `attestations: write` in `publish` only;
  `contents: read` and `attestations: read` in `verify`. `GITHUB_TOKEN` is the only credential.
  A test in `tools/xtask/src/release.rs` fails if any other job (or the workflow) gains a write
  scope, if a trigger other than these appears, or if a cache, a secret or `cargo publish` does.
- **Jobs:** `build` runs `cargo xtask release-check --target <t>` on `macos-15`,
  `macos-15-intel`, `ubuntu-24.04` and `ubuntu-24.04-arm`, with the toolchain from
  `rust-toolchain.toml` and tools built at their pins, no cache restored. `publish` writes
  `cargo xtask sbom` (one `site-preview-<t>.cdx.json` per archive, the target's normal graph,
  checked to list every crate `release-check` ships, the checkout path cut from `bom-ref`s,
  `SOURCE_DATE_EPOCH` the commit time), `SHA256SUMS` over all eight files, the provenance of all
  eight (`actions/attest-build-provenance` v4.2.2), and the draft. It then downloads the draft's
  assets back, checks them against `SHA256SUMS` and byte for byte against what it uploaded, and
  hands exactly those bytes to `verify` as a run artifact. `verify`, on each OS, checks
  `SHA256SUMS`, runs `gh attestation verify` (signer workflow, source ref and digest, no
  self-hosted runner), and smokes the archive with the new
  `release-check --target <t> --archive <path>`, which builds nothing.
- Actions are SHA-pinned; `download-artifact` is v8.0.1, not the six-day-old v8.0.2.

## Consequences

- `verify` tests the bytes the draft serves, read back by `publish`, not a second download of its
  own; giving `verify` a write token was the alternative and was rejected.
- A dry run leaves a draft, a `dry-run-*` tag and public attestations (Sigstore's log is
  append-only); the owner deletes the first two. A rerun needs the earlier draft deleted.
- Without caches a release run compiles everything, cargo-about and cargo-cyclonedx included.
- `release-check`'s smoke has run only on Linux before; its first macOS run is the first dry run.
