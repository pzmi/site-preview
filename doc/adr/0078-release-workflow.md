# 0078. The release workflow: tag-run only, drafts only, write in one job, dry runs on `dry-run-*` tags

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Implements D23 (ADR 0024) for releases;
keeps D24 (ADR 0025, no crates.io) and ADR 0061 (no caches in the release workflow). Amended
2026-10-14 (checkpoint OPS-06 review): the trust boundary of `publish`, pinned verification,
pre-releases, the tag and CI re-checks, reproducible archives; see "Amendment".

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

## Amendment (2026-10-14): the release trust boundary

The checkpoint review of OPS-06 found that the one job holding the write token and the OIDC
identity also compiled and ran third-party code, that a user's `gh attestation verify --repo`
accepted any attestation of the repository (a dry run's included), that a pre-release tag became
Latest, and that neither the tag nor CI was re-checked. This amendment replaces the `Jobs`
bullet where they disagree.

- **`publish` builds nothing.** The SBOMs move to a job `sbom` (`contents: read`, after `build`):
  it installs the pinned `cargo-cyclonedx` (built from crates.io, a graph `Cargo.lock` and
  `cargo deny` do not cover), runs `cargo xtask sbom` and uploads the SBOMs as a run artifact.
  `publish` has no checkout and no `cargo`, `rustup` or `install-tools` step: it downloads the
  archives and SBOMs, writes `SHA256SUMS`, attests, re-reads the tag, creates the draft and
  reads it back. `should_build_nothing_in_publish_when_release_yml_is_read` fails if a build
  tool, or an action other than download-artifact, upload-artifact and attest-build-provenance,
  enters `publish`.
- **Pinned verification.** The documented check is
  `gh attestation verify <file> --repo <repo> --signer-workflow <repo>/.github/workflows/release.yml --source-ref refs/tags/<tag>`
  (release notes, [`release.md`](../release.md); OPS-07's README install section takes the
  same). `verify` runs it, with `--source-digest` and `--deny-self-hosted-runners`, on each
  archive and on `SHA256SUMS`, which is now attested too.
- **Dry runs rehearse main.** `prepare` requires the tagged commit on `main` in both modes.
- **CI must be green.** `prepare` (with `checks: read`, its only addition) reads the check runs
  named `Required` of the tagged commit, made by GitHub Actions, and fails unless there is at
  least one and every latest one concluded `success`. This holds for dry runs too.
- **Pre-releases.** Any tag with a semver pre-release suffix (a `-`), and every dry run, is
  created `--prerelease`; publishing with `--draft=false` keeps it a pre-release, so it is never
  Latest.
- **The tag is re-read.** Right before the draft, `publish` peels the remote tag through the
  API (an annotated tag through its tag objects) and requires the commit `prepare` peeled from
  `GITHUB_SHA`. An owner tag ruleset restricting update and deletion of `v*` closes the window
  after that ([`release.md`](../release.md#one-time-setup-owner)).
- **Untrusted values in `env:` only.** `${{ inputs.* }}` and `${{ github.ref_name }}` appear
  only as whole `env:` values (`should_read_inputs_and_the_ref_name_only_in_env_values_when_release_yml_is_read`).
- **Reproducible archives** (ADR 0076 amendment): the release build remaps `$CARGO_HOME` and the
  checkout with `--remap-path-prefix` (and fails if either path survives in the binary); the
  tarball is ustar with sorted entries, modes 0755/0644, owner 0 and the commit time
  (`SOURCE_DATE_EPOCH` when set), compressed with `gzip -n`. `release-check` lists the archive
  with `tar -tvzf` before extracting it and requires exactly one directory and the five regular
  files, no link and no other top-level entry. The smoke skips JSON lines (a warning) before
  `ready`.
- **No musl-tools.** rustc links the `*-linux-musl` targets with the system `cc` and the musl
  the Rust target ships (self-contained `libc.a` and start files); `musl-gcc` is never called
  (a `release-check` with a failing `musl-gcc` first on `PATH` passes) and `Cargo.lock` has no
  `cc` crate. The unpinned apt install is dropped from `release.yml` and the `artifact-smoke` CI
  job. A dependency that compiles C would bring it back, pinned, with this ADR amended.
- **Notices stay in `build`.** A separate notices job was considered and not taken:
  `cargo-about` runs in `build`, which is read-only, holds no secret and already compiles the
  whole third-party graph into the shipped binary; moving it would add a hand-off, not a
  boundary. Its stderr is kept in `logs/` with the smoke logs.

Consequences: a release needs a green `Required` on its commit before tagging, and a
pre-release tag no longer needs a manual "pre-release" tick. The owner adds the tag ruleset once
(a human-only step). The OPS-06 acceptance still lacks a real dry run: agents never push tags or
dispatch workflows, so the first dry run is the owner's.
