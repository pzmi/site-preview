# Releases

How the owner cuts a release of site-preview, rehearses it with a dry run, and checks the result.
The workflow is [`.github/workflows/release.yml`](../.github/workflows/release.yml); the design and
its reasons are in [ADR 0078](adr/0078-release-workflow.md). Agents never push tags, dispatch the
workflow or publish a release ([`sdlc.md`](sdlc.md#integration-procedure)); every step below is
the owner's.

## What a run produces

A **draft** GitHub release, never a published one, holding:

| Asset | What it is |
|:--|:--|
| `site-preview-<target>.tar.gz` (four) | `site-preview-<target>/` with the binary, `README.md`, both licences and `THIRD-PARTY-NOTICES`, built and smoked on its native runner by `cargo xtask release-check` ([ADR 0076](adr/0076-release-check-archive-and-notices.md)) |
| `site-preview-<target>.cdx.json` (four) | the CycloneDX SBOM of that archive's crate graph (`cargo xtask sbom`) |
| `SHA256SUMS` | the SHA-256 of the eight files above |
| `site-preview.intoto.jsonl` | the Sigstore bundle of the build-provenance attestation of all nine files above, one JSON line; not in `SHA256SUMS`, which it attests |

The targets are `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-musl` and
`x86_64-unknown-linux-musl`. Each archive, each SBOM and `SHA256SUMS` also has a build-provenance
attestation, stored by GitHub and in Sigstore's public log, and attached to the release as
`site-preview.intoto.jsonl` for offline checks and SLSA tooling. Nothing is published to crates.io
(D24). The archives are reproducible from the commit: build paths are remapped out of the binary
and the tarball has sorted entries, fixed modes, owner 0 and the commit time ([ADR
0078](adr/0078-release-workflow.md)).

The jobs, in order:

| Job | Token | What it does |
|:--|:--|:--|
| `prepare` | read, `checks: read` | validates the tag, requires its commit on `main` and the CI check `Required` green on it |
| `build` (four runners) | read | `cargo xtask release-check`: build, notices, archive, smoke |
| `sbom` | read | `cargo xtask sbom`: one SBOM per archive, kept as a run artifact |
| `publish` | write | builds nothing: downloads the archives and SBOMs, writes `SHA256SUMS`, attests, re-reads the tag (it must still name the built commit), creates the draft and reads its assets back |
| `verify` (four runners) | read | `SHA256SUMS`, `gh attestation verify` (as below), and the smoke of each archive as published |

## One-time setup (owner)

A tag ruleset keeps a release tag from moving after its run read it. In the repository's
**Settings → Rules → Rulesets**, add a tag ruleset, enforcement **Active**, target **Include by
pattern** `v*`, with the rules **Restrict updates** and **Restrict deletions** (no bypass list, or
the owner only). Creating a `v*` tag stays allowed; moving or deleting one is refused. `dry-run-*`
tags are not covered, so a dry run can still be cleaned up. `publish` checks the tag again right
before it creates the draft; the ruleset closes the window after that.

## Dry run

A dry run tests the whole pipeline on a throwaway tag named `dry-run-<name>`. Its draft is a
pre-release titled `DRY RUN ... (delete, never publish)`. Run it from a commit of `main` that
contains `release.yml` and whose CI (`Required`) is green; `prepare` refuses any other commit,
exactly as for a release:

```sh
git fetch origin
git tag dry-run-1 origin/main
git push origin dry-run-1          # dry-run-* does not match v*, so this starts nothing
gh workflow run release.yml --ref dry-run-1 -f tag=dry-run-1 -f dry_run=true
gh run list --workflow release.yml --branch dry-run-1 --event workflow_dispatch   # the run of this tag
gh run watch <run id from the line above>
```

`gh workflow run` does not print the run it started, and `gh run list --limit 1` without a filter
can pick another run (a tag push, or a second dispatch); filter by the tag (`--branch` matches a
run's ref, a tag included) and wait a few seconds if the list is still empty. `--ref` and `tag`
must be the same tag: the workflow refuses to build anything else, so the provenance names the
commit that was built. Check the result as in [Checking a release](#checking-a-release), with
`--source-ref refs/tags/dry-run-1`, then clean up (the attestations stay; they are harmless, and
none of them passes a check pinned to a `v*` tag):

```sh
gh release delete dry-run-1 --yes --cleanup-tag   # the draft and the remote tag
git tag -d dry-run-1
```

## Cutting a release

1. `main` is green (`Required`, [`required-checks.md`](required-checks.md)), and the workspace
   `version` in `Cargo.toml` and `CHANGELOG.md` name the new version.
2. Tag the commit on `main` and push the tag; the push starts the workflow:

   ```sh
   git tag -a v0.1.0 -m "site-preview 0.1.0" <commit on main>
   git push origin v0.1.0
   ```

   `prepare` fails unless the tag is `v` plus the workspace version, its commit is on `main`, and
   `Required` concluded success on that commit.
3. Wait for every job to be green, `verify` included. Check the draft as below.
4. Publish the draft by hand: `gh release edit v0.1.0 --draft=false` (or the web page). The tag
   already exists, so publishing creates or moves none. A tag with a semver pre-release suffix
   (`v0.2.0-rc.1`, any `-`) was created as a pre-release; `--draft=false` keeps it one, so it
   never becomes the repository's Latest release. A tag without a suffix is a normal release.

If a run fails after `publish` created the draft, delete the draft (`gh release delete v0.1.0
--yes`, without `--cleanup-tag`, so the tag stays; the ruleset would refuse deleting it anyway)
and rerun from the tag:
`gh workflow run release.yml --ref v0.1.0 -f tag=v0.1.0 -f dry_run=false`, finding the run with
`gh run list --workflow release.yml --branch v0.1.0`. A run refuses a tag that already has a
release, draft or not.

## Checking a release

On the run page: green `Prepare`, four green `Build`, `SBOMs`, `Publish the draft release`, four
green `Verify`. Then, from an empty directory, as a user would:

```sh
gh release download <tag>                        # a draft needs a token with write access
sha256sum -c SHA256SUMS                          # macOS: shasum -a 256 -c SHA256SUMS
gh attestation verify site-preview-x86_64-unknown-linux-musl.tar.gz --repo pzmi/site-preview \
  --signer-workflow pzmi/site-preview/.github/workflows/release.yml --source-ref refs/tags/<tag>
tar -xzf site-preview-x86_64-unknown-linux-musl.tar.gz
site-preview-x86_64-unknown-linux-musl/site-preview --version
```

`--repo` alone accepts any attestation the repository's workflows made, a dry run's included.
`--signer-workflow` requires the signature of `release.yml` and `--source-ref` the release's own
tag, so only an artifact the release workflow built from that tag passes. The same check, plus
`--source-digest` and `--deny-self-hosted-runners`, is what `verify` runs on `SHA256SUMS` and
each archive, and once more on each archive with `--bundle site-preview.intoto.jsonl`, the
release's own copy of the attestation (no lookup of GitHub's attestation store).

The same smoke the workflow runs is available locally on a downloaded archive, from a checkout of
the tag: `cargo xtask release-check --target <target> --archive <path>`.
