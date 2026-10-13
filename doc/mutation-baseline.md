# Mutation baseline

The score of `cargo xtask mutation` on the TEST-08 commit, and the triage of every mutant the
tests did not catch (foundation.md section 5.3, ADR 0068). Re-measure after a change of scope
or a large change of the tests, and replace the numbers here; do not append history.

## Setup

- cargo-mutants 27.1.0 (`tools/versions.toml`), nextest as the test tool, Rust 1.97.0.
- Scope (`.cargo/mutants.toml`): `crates/preview-domain/src`, `crates/preview-application/src`
  and `crates/preview-adapter-http/src/inject`: 497 mutants.
- Tests selected: cargo-mutants runs the tests of the package that owns the mutated file, so
  the `preview-cli` black-box tests, `preview-conformance` and the Playwright suite never run.
  The nextest filter also drops the trybuild `compile_fail` binaries and the OS watcher tests.
  A survivor below therefore means "the unit, component and package integration tests do not
  see it"; no survivor is claimed eliminated by a suite that did not run.
- Run: `cargo xtask mutation --jobs 2` on a 4-core shared machine, about 52 minutes
  (unmutated baseline: 66 s build, 5 s test). Timeout: five times the unmutated test time,
  at least 60 s.
- Score: `caught / (caught + missed + timeout)`. Unviable mutants (did not compile) are left
  out; a timeout counts against the score.

## Result

| Run | Caught | Missed | Timeout | Unviable | Score |
|:--|--:|--:|--:|--:|--:|
| First run, tests as they were before TEST-08 | 341 | 46 | 6 | 104 | 86% |
| After the tests this task added (the baseline) | 380 | 7 | 6 | 104 | 96% |

The target is 90% on the critical domain algorithms after equivalent mutants are triaged
(foundation.md section 5.3). With the three equivalent survivors below left out the baseline
is 380 of 392 viable mutants, 96.9%.

## Survivors of the baseline (7)

| Mutant | Triage | Why |
|:--|:--|:--|
| `rel_path.rs:19` `RelPath::root` -> `Default::default()` | equivalent | `RelPath` derives `Default` as the empty component list, which is the root; the body is that call already. |
| `csp.rs:112` `\|\|` -> `&&` in `allows_same_origin` | equivalent | With `&&` an empty source list falls through to a loop over no sources, which returns `false`, the same as the early return. |
| `resolve_request.rs:172` `==` -> `!=` in `denied_as_directory` | equivalent | It swaps `Level::Root` and `Level::Nested`; the levels differ only for the reserved root name `__site_preview`, which `eligibility::path` has already refused for a file before `denied_as_directory` runs. |
| `rewrite.rs:332` `&&` -> `\|\|` in `refresh` | needs-test | The `refreshed` check (the page still has the same number of client tags, all with the new snapshot) cannot fail unless the HTML rewriter returns something other than what it was asked to write. It needs a seam that corrupts the rewriter output. |
| `rewrite.rs:333` `&&` -> `\|\|` in `refresh` | needs-test | Same guard, same missing seam. |
| `rewrite.rs:335` `&&` -> `\|\|` in `refresh` | needs-test | Same guard, same missing seam. |
| `rewrite.rs:342` `\|\|` -> `&&` in `refresh` | needs-test | `UnexpectedOutput` is returned when either check fails; the tests reach neither. Same seam. |

The four `refresh` survivors are the only open work. They guard against a bug in the
dependency, `is_single_insertion` and `outside` are tested directly, and the guards are
exercised end to end by `should_refresh_snapshot_attributes_when_client_tag_is_stale`. Adding
a rewriter fault seam only to kill them is not worth its weight today; the mutants stay on
this list until a task needs the seam.

## Timeouts (6)

All six turn a loop bound or a counter into one that never advances, so the test run hangs
until the limit; a hang is a detected change, but the run reports it as a timeout and the
score counts it against the tests.

- `budget.rs:30` `*` -> `/`: `CHUNK` becomes 64 / 1024 = 0, so the body stream never advances.
- `budget.rs:122` `+=` -> `*=` in `Buffered::poll_next`: the offset stays 0.
- `charset.rs:114`, `charset.rs:116` in `charset_from_content`: the search start moves
  backwards or stays, so the scan for `charset` never ends.
- `path.rs:166`, `path.rs:169` in `has_well_formed_escapes`: the index multiplies instead of
  advancing and stays at zero.

## What the first run missed, and how it was closed

Of the 46 first-run survivors, 39 were needs-test and were killed by tests added in TEST-08.
The other seven are the survivors of the baseline above: three equivalent, four `refresh`
guards that need a fault seam.

| Group | Mutants | Test added |
|:--|--:|:--|
| `HtmlBudget::default` size and the 64 KiB body chunk (`budget.rs:20`, `:30`) | 5 | `should_hold_128_mib_when_budget_is_the_default`, `should_send_64_kib_chunks_when_page_is_larger` |
| `charset_from_content` value ends at `;` (`charset.rs:126`) | 1 | `should_stop_the_value_at_a_semicolon_when_content_has_more_parameters` |
| Display of `Found`, `RewriteFailure`, `ReloadClosed`, `ContentRefusal`, `InstanceIdError`, `RevisionExhausted`, `PathRefusal` | 7 | one text-pinning test each |
| Exactly 8 MiB is still injected (`response.rs:132`) | 1 | `should_inject_when_html_is_exactly_the_limit` |
| Warning names the meta charset only for another declared encoding (`response.rs:219`) | 1 | `should_name_the_meta_charset_in_the_warning_only_when_it_declares_another_encoding` |
| A `<script src>` that is not the client (`rewrite.rs:167`) | 1 | `should_not_take_another_script_for_the_client_when_page_loads_a_script` |
| `is_single_insertion` checks the tag, `outside` splits and refuses (`rewrite.rs:263`, `:351`) | 5 | `should_reject_an_insertion_elsewhere_when_output_has_the_right_length`, `should_return_the_parts_between_ranges_when_outside_is_taken` |
| `ContentFile::is_empty` (`site_content.rs:69`) | 1 | `should_be_empty_only_when_file_length_is_zero` |
| `ChangeSet::is_empty`, `WatchDiff::is_empty`, `WatchSet::len` and `is_empty` | 8 | `should_not_be_empty_when_only_the_full_reason_is_set`, `should_report_emptiness_when_only_one_side_of_a_diff_is_set`, `should_count_directories_when_set_is_built` |
| `Revision::get`, `Tick::as_millis`, `SitePath::is_root`, `Component::as_ref` | 6 | one test each |
| `parse_authority` bracketed host without a port, `parse_port("0")` (`request_policy.rs:348`, `:375`) | 2 | `should_use_the_default_port_when_bracketed_ipv6_host_has_no_port`, `should_accept_port_zero_and_reject_leading_zeros_when_port_is_parsed` |
| `.swp` alone is reported hidden, not an editor artifact (`eligibility.rs:155`) | 1 | `should_report_hidden_when_name_is_only_the_swap_suffix` |

## Running it

- `cargo xtask mutation` runs the whole scope; `--package <crate>` narrows it and `--jobs N`
  sets the parallelism (default: half the cores). Point `TMPDIR` at a disk with room: each job
  copies the tree and builds in it.
- `cargo xtask mutation --in-diff <rev>` tries only the mutants on lines changed since the
  merge-base of `<rev>` and `HEAD`, working tree included. A diff whose changed lines hold no
  mutant (only tests, docs or comments) is a clean run that tries nothing.
- The report is `target/mutants/mutants.out/` and `target/mutants/summary.md`. The exit code
  is 0 when the run completed, whatever it found; `mutation.yml` runs it weekly and uploads both.
