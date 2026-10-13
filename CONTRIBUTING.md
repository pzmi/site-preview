# Contributing

Thanks for your interest. The project is pre-alpha. See [doc/tasks.md](doc/tasks.md) for current work.

Planned work is done by the maintainer and agents as one signed-off commit per task, integrated
directly on `main` ([doc/sdlc.md](doc/sdlc.md)). Outside contributions are welcome as pull
requests; this page says what a pull request needs. For anything larger than a small fix, open an
issue first so the design can be agreed before you write code.

## Sign off your commits (DCO)

Sign off every commit under the [Developer Certificate of Origin](https://developercertificate.org/):

```text
git commit -s
```

That adds a `Signed-off-by:` line with your name and email. Use short imperative subjects (for
example `Add debounce timer`), with a body that explains why when it is not obvious.

## Build and test

The Rust toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it on first use.

```text
cargo xtask doctor          # report missing dev tools and wrong versions
cargo xtask install-tools   # install the pinned cargo tools (not the scanners or Apalache)
cargo xtask quick           # formatter, architecture, Clippy, fast tests: while you work
cargo xtask check           # the full gate: run it before you open a pull request
cargo xtask list-checks     # every check, machine-readable
```

`quick` is not `check`. `check` is the same gate CI runs, split over several jobs
([doc/required-checks.md](doc/required-checks.md)). It needs:

- **Node 22.x** and npm, for the Quint models (`npm ci --prefix formal` installs the locked
  Quint; nothing is fetched at run time) and the browser tests.
- **Chromium** for the Playwright browser stage. `install-tools` does not install it: run
  `npx --prefix tests/e2e/browser playwright install chromium` once, or point
  `PLAYWRIGHT_BROWSERS_PATH` at a directory that has the build the pinned Playwright launches
  (`chromium_headless_shell-1194`). Without it the stage fails with that hint;
  `cargo xtask test --only browser` runs it alone.
- **The pinned cargo tools** that `cargo xtask install-tools` installs: `cargo-nextest`,
  `cargo-llvm-cov`, `cargo-deny` and the others in `tools/versions.toml`.
- **Gitleaks and actionlint** for the security stages, and **Apalache** (with a JVM) for
  `formal-verify`: `install-tools` does not install these, it prints the release page of the
  pinned version. Install them by hand; `cargo xtask doctor` names what is missing.

If a required tool is missing, the command fails and says how to install it; it never reports
success for a stage it did not run. `cargo xtask check --only <stage>` runs single stages.

## Architecture and decisions

Read [doc/architecture.md](doc/architecture.md) and [doc/foundation.md](doc/foundation.md) for the
rules and [doc/plan.md](doc/plan.md) for the product plan. Crate and module boundaries, purity of
the domain and the test conventions are enforced by checks; fix the structure instead of loosening
`architecture.toml`, a lint or a test.

**The ADR rule:** a change that adds or reverses an architectural decision (a dependency or crate
edge, a protocol or security rule, a tool or CI topology) adds an Architecture Decision Record in
the same pull request: the next free `doc/adr/NNNN-kebab-title.md`, in the Nygard format (Status,
Context, Decision, Consequences), listed in [doc/adr/README.md](doc/adr/README.md), or supersedes
an existing one. Behaviour that touches the lifecycle, watcher or reload protocol starts with the
Quint model ([doc/formal-models.md](doc/formal-models.md)).

## Pull requests

CI runs on pull requests. The maintainer integrates accepted changes directly on `main` as one
signed-off commit; pull requests are not merged through branch protection.

- Fill in the [pull request template](.github/pull_request_template.md): the impact, the gates you
  ran and the evidence.
- `cargo xtask check` must pass, and the `Required` check of your pull request must be green.
- Do not suppress, skip or delete tests or lints to get green; fix the cause.
- Add tests for behaviour changes, named `should_<outcome>_when_<condition>` with `// given`,
  `// when` and `// then` sections.
- Keep a pull request to one change, and do not include secrets, personal paths or private
  content: fixtures use synthetic content.
- New dependencies are added with `cargo add` and need a reason in the description.

## Reporting problems

Bugs and feature requests go through issue templates. Security issues must be reported privately; see [SECURITY.md](SECURITY.md).

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as `MIT OR Apache-2.0`, without any additional terms or conditions.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
