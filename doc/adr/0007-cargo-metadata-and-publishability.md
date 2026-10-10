# 0007. Cargo metadata and publishability

Date: 2026-10-10

## Status

Accepted

## Context

Release and registry tooling read workspace metadata. Internal crates are implementation details.

## Decision

`[workspace.package]` sets `license = "MIT OR Apache-2.0"`, `repository = "https://github.com/pzmi/site-preview"`, `rust-version` equal to the pinned toolchain, keywords, categories (`command-line-utilities`, `development-tools`, `web-programming::http-server`) and `readme`. Only `preview-cli` (binary `site-preview`) is publishable. Every other crate has `publish = false`.

## Consequences

- Dual license files are committed.
- Actual crates.io publishing is blocked by path dependencies; see ADR 0025.
