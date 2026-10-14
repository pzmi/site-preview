# 0024. P0 supply-chain controls and release evidence

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

foundation.md §8 lists many supply-chain tools and signed releases. plan.md §14 defers automated release signatures to P1.

## Decision

P0 uses cargo-deny, Gitleaks, actionlint, CodeQL (Rust) and Dependabot. Releases ship `SHA256SUMS`, GitHub build-provenance attestations and a CycloneDX SBOM. `cargo-vet`, `cargo-machete` and detached signatures are deferred.

## Consequences

- Keyless attestation needs no key management.
- Revisit cargo-vet once an audit policy exists.
