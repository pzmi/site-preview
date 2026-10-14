# qa

Dispatch: inline

## Inputs

implement.{affectedCrates, filesChanged}, intake.{criteria, taskId}

## Steps

1. **Is there a surface?** CLI flags, output or exit codes, HTTP responses, HTML injection, SSE, the watcher-to-reload loop: yes. Pure domain, tooling, CI or docs: record `{skipped: "no runnable surface: <reason>"}` and pass.
2. **Build** the binary: `cargo build --locked --bin site-preview` (or the `cargo xtask` build subcommand once one exists). Serve a scratchpad copy of a fixture site (`tests/fixtures/`), so edits never touch committed fixtures; never serve the repo root.
3. **Exercise the CLI and HTTP surface.**
   - Start it in the background with output to the scratchpad: `${CARGO_TARGET_DIR:-target}/debug/site-preview serve --json --port 0 <fixture> > <scratch>/out.jsonl 2> <scratch>/err.log`. Read the `ready` line for the URL (poll the file with a bounded loop; do not sleep blindly).
   - Golden path: `curl -sS -D - <url>` returns the page, `Cache-Control: no-store`, the injected client tag when HTML.
   - At least one refusal per changed area: `curl -sS -o /dev/null -w '%{http_code}' <url>.env` (404/403, no body leak); a bad Host (`-H 'Host: evil.example'`) rejected; `-X POST` gives 405; for SSE, a cross-site `Origin` rejected.
   - Exit codes: a bad flag or missing root exits nonzero with a helpful stderr line.
   - Stop it with SIGINT (`kill -INT <pid>`, the PID you started; never by pattern) and confirm exit 0 and that the port is closed.
4. **Browser** (when injection, SSE or the reload loop changed): run the Playwright journey with the preinstalled Chromium (`PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`; never `playwright install`): load the page, edit a fixture file, confirm one reload and the new content.
5. **Regressions**: a neighbouring command or route that used to work. Any regression goes back to implement as a finding.
6. Confirm the served files are byte-for-byte unchanged by serving (compare against the fixture before your own edits; `git status` shows no fixture change).
7. Write `qa.json`.

## Output

```
{ skipped: string?, exercised[], refusalsTested[], browserChecked: bool?, regressions[], issues[] }
```

## Sensors

Skipped with a reason, or `exercised` non-empty with at least one refusal tested, the server stopped, and `regressions` empty.

## Gates

To ship: no regression.
