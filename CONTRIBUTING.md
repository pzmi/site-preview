# Contributing

Thanks for your interest. The project is pre-alpha; the tooling below is planned and may not exist yet. See [doc/tasks.md](doc/tasks.md) for current work.

## Build and test (planned)

- The Rust toolchain is pinned via `rust-toolchain.toml`.
- `cargo xtask quick` - formatter, Clippy, fast tests (not a substitute for `check`).
- `cargo xtask check` - the full PR-quality gate; the same gate runs in CI.
- Quint (formal models) is a development-only tool that needs Node; the installed binary never does.

If a required local tool is missing, the command fails with instructions rather than reporting success.

## Architecture

Read [doc/foundation.md](doc/foundation.md) for architecture rules and [doc/plan.md](doc/plan.md) for the product plan. Respect crate and module boundaries; they are enforced by checks.

## Commits and sign-off

- Use short imperative commit subjects (for example `Add debounce timer`), with a body explaining why when it is not obvious.
- Sign off every commit under the [Developer Certificate of Origin](https://developercertificate.org/): `git commit -s`.

## Pull requests

- Fill in the pull request template.
- All gates must be green (`cargo xtask check`).
- Do not suppress, skip, or delete tests or lints to get green; fix the cause.
- Include test evidence for behavior changes, and note any model, architecture, or security impact.

## Reporting problems

Bugs and feature requests go through issue templates. Security issues must be reported privately; see [SECURITY.md](SECURITY.md).

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as `MIT OR Apache-2.0`, without any additional terms or conditions.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
