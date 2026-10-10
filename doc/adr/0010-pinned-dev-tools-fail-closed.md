# 0010. Pinned dev tools, fail closed when missing

Date: 2026-10-10

## Status

Accepted

## Context

Quality gates depend on external tools (cargo-nextest, cargo-deny, cargo-llvm-cov, cargo-mutants, cargo-fuzz, quint and others). A missing tool must never look like a passing gate.

## Decision

Pin every tool in `tools/versions.toml` (Node tools in their lockfiles). `cargo xtask doctor` reports missing or mismatched tools. Every stage calls `tools::require` and exits non-zero with an actionable install command when a tool is absent.

## Consequences

- In the sandbox, a blocked install is reported as a tooling blocker, never as a pass.
