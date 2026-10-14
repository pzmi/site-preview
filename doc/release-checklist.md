# Release checklist: v0.1.0

Maps each item of the plan's definition of done ([`plan.md`](plan.md) section 16) to the evidence
for it, so the owner can see what is proven before pushing the tag. Status is one of:

- **done**: automated evidence exists and runs in the gate or in CI; the evidence column names it.
- **pending**: an owner step that cannot happen before the tag.
- **gap**: no evidence; none today.

Done means the check exists and passes at the commit this file was written for. It does not
replace reading the `Required` result ([`required-checks.md`](required-checks.md)) on the commit
you tag. Test names are grep-able; CI job names are the display names in
[`required-checks.md`](required-checks.md). How the release workflow works is in
[`release.md`](release.md).

## Plan section 16

| # | Item | Status | Evidence |
|:--|:--|:--|:--|
| 1 | One executable works without Rust, Node, npm or per-site setup | done (notices pending, see below) | `cargo xtask release-check` extracts the packaged archive and runs the binary with `env -i PATH=/usr/bin:/bin`, refusing a `PATH` that holds cargo or node (`tools/xtask/src/release.rs`, `CLEAN_PATH`; [ADR 0076](adr/0076-release-check-archive-and-notices.md)); CI job `Artifact smoke` (linux musl x86_64); the release workflow's `Build` jobs repeat it on all four native runners. Clean-runner proof for all four archives: item 11. [`runbook.md`](runbook.md#minimum-system-expectations-released-binaries) |
| 2 | `site-preview serve .` serves HTML and ordinary static assets on localhost | done | `crates/preview-cli/tests/serve_slice.rs`; `crates/preview-cli/tests/http_security.rs`; Chromium: `should_load_the_fixture_page_when_the_server_is_ready` (`tests/e2e/browser/loads.spec.ts`), `should_serve_css_js_svg_and_gpx_when_fetched_from_the_page` (`reload.spec.ts`); CI jobs `Integration`, `Browser` |
| 3 | Source assets and HTML are never modified by the tool | done | `should_leave_source_tree_unchanged_when_pages_and_streams_are_served` (`crates/preview-cli/tests/injection_sse.rs`); `should_leave_source_bytes_unchanged_when_served` (`crates/preview-cli/tests/serve_slice.rs`); `should_leave_tree_unchanged_when_suite_has_run` (`crates/preview-cli/tests/http_security.rs`); `should_leave_source_bytes_unchanged_when_injected` (`crates/preview-adapter-http/tests/inject.rs`); CI job `Integration` |
| 4 | File changes reliably cause one debounced browser reload | done | `should_publish_one_revision_when_twenty_edits_burst` and `should_publish_nothing_when_git_editor_temp_and_node_modules_change` (`crates/preview-cli/tests/watcher.rs`); `should_send_reload_when_index_html_is_edited` (`watch_reload.rs`); `should_show_new_text_after_one_reload_when_html_is_edited` (`tests/e2e/browser/reload.spec.ts`); `should_navigate_exactly_once_and_stay_stable_when_html_changes` (`hot.spec.ts`); the Quint watcher and reload models replayed against the code ([`formal-models.md`](formal-models.md)), CI job `Conformance (Quint)`; OS watcher tests on all four runners, CI job `Platform` |
| 5 | Initial connection, disconnect, reconnection and restart do not reload endlessly | done | `should_show_initial_text_without_reload_loop_when_page_loads` (`reload.spec.ts`); `should_not_reload_when_stream_reconnects_to_an_unchanged_server`, `should_reload_once_when_stream_reconnects_after_an_edit_made_while_it_was_down`, `should_reload_once_and_stay_when_server_restarts_on_the_same_port` (`reconnect.spec.ts`); reload-protocol model conformance (`crates/preview-conformance/tests/reload_conformance.rs`); CI jobs `Browser`, `Conformance (Quint)` |
| 6 | `--json --port 0` provides a stable, flushed URL for automation | done | `should_emit_single_ready_line_when_json`, `should_flush_ready_when_stdout_is_pipe`, `should_keep_stdout_json_only_when_warnings_occur` (`crates/preview-cli/tests/json.rs`); `should_print_the_documented_ready_line_when_started_as_agents_md_says` (`agent-workflow.spec.ts`); [`README.md`](../README.md#machine-output---json); CI jobs `Integration`, `Browser` |
| 7 | Playwright MCP can inspect the actual rendered page and repeat after edits | done | `should_complete_the_whole_workflow_when_an_agent_follows_agents_md` (`tests/e2e/browser/agent-workflow.spec.ts`) plays the workflow of [`AGENTS.md`](../AGENTS.md) (start, read `ready`, open and inspect, edit, observe the reload, stop) and `should_match_the_commands_in_agents_md_when_compared_with_what_this_spec_exercises` fails when the document drifts; example skill: [`examples/claude-skill/static-html-preview/SKILL.md`](../examples/claude-skill/static-html-preview/SKILL.md); CI job `Browser`. Note: the spec drives Chromium with the Playwright library, not the Playwright MCP server, which the plan treats as an optional external consumer |
| 8 | Dotfiles, excluded directories, traversal, symlink escapes and DNS rebinding are handled | done | `should_not_serve_git_config_when_requested`, `should_not_serve_env_when_requested`, `should_not_serve_node_modules_file_when_requested`, `should_refuse_traversal_when_dot_segments_are_percent_encoded`, `should_refuse_symlink_when_it_points_outside_root`, `should_refuse_path_when_symlink_is_in_the_middle`, `should_reject_with_421_when_host_names_another_site` (`crates/preview-cli/tests/http_security.rs`); `should_refuse_with_403_when_events_origin_is_foreign` (`injection_sse.rs`); `should_never_return_outside_bytes_when_directory_is_swapped_for_symlink` (`crates/preview-adapter-fs/tests/symlink_race.rs`); fuzz targets `decode_path` and `host_header`, results in [`security.md`](security.md#fuzzing); rules in [`security.md`](security.md); CI jobs `Integration`, `Security` |
| 9 | Watcher failure is visible, while basic HTTP stays usable | done | `should_report_root_removed_once_when_site_root_is_deleted` (`crates/preview-cli/tests/watcher.rs`: a `watcher` JSON event and one `warning[root-removed]`); `should_warn_and_degrade_when_reconcile_cannot_watch_a_directory` and `should_announce_state_change_once_when_watcher_degrades_and_recovers` (`crates/preview-composition/src/watch_pipeline.rs`); watcher-health model conformance (`watcher_conformance.rs`); [`runbook.md`](runbook.md#the-watcher-is-degraded-or-unavailable); CI jobs `Integration`, `Conformance (Quint)` |
| 10 | CI tests and smoke tests pass for macOS ARM64/x86_64 and Linux ARM64/x86_64 | done | CI job `Platform` is one matrix of four native legs: `Platform (ubuntu-24.04)`, `Platform (ubuntu-24.04-arm)`, `Platform (macos-15)`, `Platform (macos-15-intel)` running `cargo xtask test --only platform-tests` ([ADR 0074](adr/0074-native-platform-matrix.md)), part of `Required`; the archive smoke on each native runner is the `Build` and `Verify` matrices of `release.yml`. The result for the tagged commit is read on the run page (owner step 1 below) |
| 11 | v0.1.0 releases have working archives and SHA-256 checksums | pending | The workflow ([`release.md`](release.md)) builds the four archives, their SBOMs and `SHA256SUMS`, attests them, and its `Verify` jobs check sums, attestation and a smoke per archive; it runs only on the tag. Nothing is published yet. See the owner steps below |

Further pending items that belong to the release rather than to a section 16 line:

| Item | Status | Note |
|:--|:--|:--|
| Release workflow dry run | pending (owner) | [`release.md`](release.md#dry-run); not yet run |
| Tag `v0.1.0` | pending (owner) | pushed by the owner only; owner steps below |
| `THIRD-PARTY-NOTICES` completeness | pending | The notices in the archives currently lack the musl libc and Rust standard library notices. A fix is coming; item 1 and the release should wait for it, and this row is ticked when the fix has landed |

Summary: 10 done, 1 pending, 0 gaps among the 11 section 16 items; plus 3 pending release items
(dry run, tag, notices).

## Plan section 11, milestone M5 exit gate

"All four binary targets can preview the same fixture without Rust, Node or npm installed."
Proved only by a real release run: the four `Verify` jobs smoke each published archive on its own
OS, with a clean `PATH`, against the fixture site. Status: pending (owner steps 3 and 4).

## Supporting evidence

| Topic | Where |
|:--|:--|
| Performance targets of plan section 13 (start-up, reload latency, idle CPU, descriptors, soak, the 64-stream cap), first recorded results, measured on a loaded shared machine, not a quiet baseline | [`performance.md`](performance.md#first-recorded-results) |
| What each tool verifies and what it does not (Miri, Apalache, fuzzing, mutation, coverage) | [`verification-scope.md`](verification-scope.md) |
| Test levels and locations; the CI job for each gate stage | [`testing.md`](testing.md), [`required-checks.md`](required-checks.md) |
| Threat model, enforced security rules, fuzzing results | [`security.md`](security.md) |
| Known limitations as users read them | [`CHANGELOG.md`](../CHANGELOG.md), [`README.md`](../README.md#limitations) |
| Mutation baseline | [`mutation-baseline.md`](mutation-baseline.md) |

## Owner steps to cut v0.1.0

Agents never push tags, dispatch the workflow or publish a release. In order:

1. Confirm `Required` is green on the `main` commit to tag, including all four `Platform` legs.
   The workspace `version` in `Cargo.toml` is `0.1.0`.
2. Set the date: in `CHANGELOG.md` change `## [0.1.0] - Unreleased` to the tag date
   (`## [0.1.0] - YYYY-MM-DD`), delete the sentence under it that tells the owner to do so, and
   commit it to `main`. Update the README status line (the "pre-release" note under the title) in
   the same commit. Wait for `Required` again.
3. Wait until the `THIRD-PARTY-NOTICES` fix (musl libc and Rust standard library notices) is on
   `main`. Then the dry run of [`release.md`](release.md#dry-run) on a throwaway
   `dry-run-<name>` tag. It exercises the workflow end to end; delete its draft and tag afterwards. Treat it as required, not optional: nothing else has run `release.yml`.
4. Tag and push:

   ```sh
   git tag -a v0.1.0 -m "site-preview 0.1.0" <commit on main>
   git push origin v0.1.0
   ```

   The push starts `release.yml`; `prepare` fails unless the tag is `v` plus the workspace version
   and the commit is on `main`.
5. Wait for `prepare`, four `Build`, `Publish the draft release` and four `Verify` to be green, then
   check the draft as in [`release.md`](release.md#checking-a-release). When verifying the
   attestation, pin the workflow and the tag, as the README does: `gh attestation verify <archive>
   --repo pzmi/site-preview --signer-workflow pzmi/site-preview/.github/workflows/release.yml
   --source-ref refs/tags/v0.1.0`. With `--repo` alone, an attestation from any workflow or ref,
   including a dry run, would pass.
6. Publish by hand: `gh release edit v0.1.0 --draft=false`. Then try the README install command
   from a machine with no Rust, Node or npm, and tick item 11 above.
