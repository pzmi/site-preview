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

The targets are `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-musl` and
`x86_64-unknown-linux-musl`. Each archive and SBOM also has a build-provenance attestation, stored
by GitHub and in Sigstore's public log. Nothing is published to crates.io (D24).

The jobs, in order: `prepare` (validates the tag), `build` (four runners), `publish` (SBOMs,
`SHA256SUMS`, attestations, the draft, and a read-back of the draft's assets), and `verify` (four
runners: `SHA256SUMS`, `gh attestation verify`, and the smoke of each archive as published).

## Dry run

A dry run tests the whole pipeline on a throwaway tag named `dry-run-<name>`. Its draft is a
pre-release titled `DRY RUN ... (delete, never publish)`. Run it from a commit of `main` that
contains `release.yml`:

```sh
git tag dry-run-1 origin/main
git push origin dry-run-1          # dry-run-* does not match v*, so this starts nothing
gh workflow run release.yml --ref dry-run-1 -f tag=dry-run-1 -f dry_run=true
gh run watch "$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
```

`--ref` and `tag` must be the same tag: the workflow refuses to build anything else, so the
provenance names the commit that was built. Check the result as in [Checking a
release](#checking-a-release), then clean up (the attestations stay; they are harmless):

```sh
gh release delete dry-run-1 --yes
git push origin :refs/tags/dry-run-1 && git tag -d dry-run-1
```

## Cutting a release

1. `main` is green (`Required`, [`required-checks.md`](required-checks.md)), and the workspace
   `version` in `Cargo.toml` and `CHANGELOG.md` name the new version.
2. Tag the commit on `main` and push the tag; the push starts the workflow:

   ```sh
   git tag -a v0.1.0 -m "site-preview 0.1.0" <commit on main>
   git push origin v0.1.0
   ```

   `prepare` fails unless the tag is `v` plus the workspace version and its commit is on `main`.
3. Wait for every job to be green, `verify` included. Check the draft as below.
4. Publish the draft by hand: `gh release edit v0.1.0 --draft=false` (or the web page). The tag
   already exists, so publishing creates or moves none.

If a run fails after `publish` created the draft, delete the draft (`gh release delete v0.1.0
--yes`, which keeps the tag) and rerun from the tag:
`gh workflow run release.yml --ref v0.1.0 -f tag=v0.1.0 -f dry_run=false`. A run refuses a tag
that already has a release, draft or not.

## Checking a release

On the run page: four green `Build`, `Publish the draft release`, four green `Verify`. Then, from
an empty directory, as a user would:

```sh
gh release download <tag>                        # a draft needs a token with write access
sha256sum -c SHA256SUMS                          # macOS: shasum -a 256 -c SHA256SUMS
gh attestation verify site-preview-x86_64-unknown-linux-musl.tar.gz --repo pzmi/site-preview
tar -xzf site-preview-x86_64-unknown-linux-musl.tar.gz
site-preview-x86_64-unknown-linux-musl/site-preview --version
```

The same smoke the workflow runs is available locally on a downloaded archive, from a checkout of
the tag: `cargo xtask release-check --target <target> --archive <path>`.
