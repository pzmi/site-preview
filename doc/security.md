# Security

What Site Preview protects, what it does not, and which test pins each rule. To report a
vulnerability, see [`SECURITY.md`](../SECURITY.md). Every claim below names the code that
enforces it and the test that fails if it stops being true; a rule with no test is listed as
planned or as a limitation, never as a guarantee.

Test names are `should_*` functions; paths are relative to the repository root.

## Threat model

| Actor | Can do | Defended by |
|:--|:--|:--|
| A web page in the developer's browser (any origin) | Send requests to `127.0.0.1:<port>`, guess the port, rebind a DNS name to the loopback address | Host gate (421), Origin guard on the event stream, no CORS headers, no state-changing routes |
| A request (or a site file) naming a path outside the site | Traverse (`..`, encoded separators), follow or race a symlink, ask for dotfiles, editor backups, `node_modules` | One-decode path rules, the eligibility predicate, per-component confined opens |
| A machine on the LAN, only with `--allow-remote` | Read the site; hold event-stream slots | Nothing but the Host gate: there is no authentication (see Limitations) |
| The supply chain | A malicious or vulnerable dependency, a leaked secret, a mutable CI action | cargo-deny, Gitleaks, pinned actions, CodeQL, Dependabot, OpenSSF Scorecard |

Site Preview is a developer preview server for a directory of static files. It binds loopback
by default, writes nothing to the site, and makes no outbound network request.

## Non-goals

- **Not a sandbox.** The confinement below is HTTP-serving confinement: it stops a request
  from reaching outside the root. It does not stop another process of the same user from
  reading the same files (plan.md §5.3, ADR 0004).
- **No authentication.** `--allow-remote` serves the site to anyone who can reach the address.
- **No protection of the site's own behaviour.** Third-party scripts and assets in the
  previewed site may make external requests; Site Preview itself makes none (plan.md §15).
- **Not a browser security boundary.** It does not rewrite or add a Content-Security-Policy.

## Path rules

A request path is decoded and judged before anything touches the disk
(`crates/preview-domain/src/path.rs`, `eligibility.rs`; ADR 0012, ADR 0039).

| Rule | Enforced in | Pinned by |
|:--|:--|:--|
| Percent-decoding happens once; `%252e` stays literal text | `decode_request_path` | `should_decode_once_when_path_is_double_encoded` |
| `.` and `..`, literal or encoded, are traversal | `path.rs` | `should_refuse_when_path_contains_encoded_dot_dot`, `should_refuse_when_path_contains_literal_dot_segments`, property `should_refuse_as_traversal_when_a_segment_is_a_dot_segment_in_any_encoding` |
| Encoded or literal separators and backslashes, empty interior components, NUL, control characters, bad escapes and invalid UTF-8 are refused | `path.rs` | `should_refuse_when_path_contains_encoded_slash`, `..._encoded_backslash`, `..._literal_backslash`, `should_refuse_when_path_has_empty_interior_component`, `..._contains_nul`, `..._contains_control_character`, `should_refuse_when_percent_escape_is_malformed`, `should_refuse_when_decoded_bytes_are_invalid_utf8` |
| Every component is a single directory entry name | `Component` invariant | property `should_yield_only_single_entry_components_when_decoding_any_request_path` |
| Hidden components (leading `.`), including `.git`, `.env` and `.well-known`, are denied | `eligibility::component` | `should_deny_when_name_is_hidden`, `should_deny_when_any_component_is_hidden`; end to end `should_not_serve_git_config_when_requested`, `should_not_serve_env_when_requested` (`crates/preview-cli/tests/http_security.rs`) |
| Directories named exactly `node_modules`, `vendor`, `.venv`, `__pycache__` are denied; a file or a longer name is not | `eligibility.rs` | `should_deny_when_directory_is_excluded`, `should_allow_file_when_name_matches_excluded_directory`, `should_allow_component_when_name_only_contains_vendor_substring`; end to end `should_not_serve_vendor_file_when_requested`, `should_not_serve_node_modules_file_when_requested` |
| Editor artifacts are denied: `*~`, `.*.swp`, `.*.swo`, `#*#`, `*.tmp`, `*.bak`, `*.orig`, `*.rej`, `4913` | `is_editor_artifact` | `should_deny_when_file_ends_with_tilde`, `should_deny_when_name_is_editor_artifact`, `should_allow_when_name_only_resembles_editor_artifact` |
| Names are compared after a case fold (case-insensitive filesystems) | `fold` | `should_deny_when_excluded_directory_differs_only_in_case` |
| A name holding a code point HFS+ ignores when comparing names (zero-width characters, directional controls, U+FEFF; the CVE-2014-9390 class) is denied | `is_hfs_ignorable` | `should_deny_when_name_holds_hfs_ignorable_code_point`, `should_allow_when_name_holds_only_ordinary_non_ascii` |
| `__site_preview` in the root, in any case, is reserved for the internal endpoints; startup fails if the site holds one | `eligibility.rs`, `preview-adapter-fs/src/root.rs` | `should_deny_when_root_component_is_reserved`; `should_fail_with_reserved_conflict_when_name_differs_only_in_case` (`crates/preview-adapter-fs/tests/root.rs`); `should_not_touch_disk_when_reserved_name_is_percent_encoded` (`crates/preview-adapter-http/tests/router.rs`) |
| The query string never reaches path resolution | `files.rs` | `should_not_pass_query_to_resolution_when_path_has_query` |
| An ineligible path is never opened | `preview-application` resolve query | `should_not_open_when_path_is_ineligible` |
| Symlinks anywhere beneath the root are refused, even when they point inside it | `capability.rs` | see Confined opens |

`.well-known` is denied with every other hidden name. That is an intentional limitation, not an
oversight (plan.md §15).

## Confined opens

`crates/preview-adapter-fs/src/capability.rs` (ADR 0004). `RootDir::open_root` is the only
function that takes a filesystem path; after it, a request is resolved one component at a
time with `openat` relative to the previous directory descriptor. Never `canonicalize` plus
`starts_with`.

- Intermediate components open with `O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC |
  O_NONBLOCK`; the final component with `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK |
  O_NOCTTY`. `O_NONBLOCK` keeps a FIFO from hanging the open and is cleared again on a regular
  file; `O_NOCTTY` keeps a terminal from becoming the controlling terminal.
- Before the final open, `fstatat(AT_SYMLINK_NOFOLLOW)` refuses a device, FIFO, socket or link
  so no driver `open` routine runs. It is only a pre-filter: `fstat` on the descriptor that
  was opened is the authoritative check, and that same descriptor is what is served.
- Errno mapping: `ELOOP` and `ENOTDIR` are a refused symlink (`Forbidden`), `ENOENT` and
  `ENAMETOOLONG` are not found, `EACCES` and `EPERM` are permission denied, `ENXIO`, `ENODEV`
  and `EOPNOTSUPP` on the final open are not-regular. A non-directory in the middle of a path
  is not found, so `/index.html/x` does not reveal that `index.html` exists.

| Claim | Pinned by (`crates/preview-adapter-fs/tests/`) |
|:--|:--|
| Final symlink to outside, symlink in the middle, symlink to a directory, dangling symlink and a symlink that points inside the root are all refused | `capability.rs`: `should_refuse_when_final_component_is_symlink_to_outside`, `..._middle_component_is_symlink`, `..._symlink_points_inside_root`, `..._final_component_is_symlink_to_directory`, `..._symlink_is_dangling` |
| A FIFO or socket is refused and a FIFO is never opened | `should_refuse_when_target_is_fifo`, `should_refuse_when_target_is_socket`, `should_not_open_fifo_when_final_component_is_fifo` |
| A path through a file is not found, not a symlink refusal | `should_return_not_found_when_middle_component_is_regular_file`, `site_content.rs`: `should_return_not_found_when_path_continues_through_file`, `should_return_not_found_when_name_exceeds_name_max` |
| Replacing the root path by a symlink after startup does not move the served tree | `should_keep_resolving_under_original_root_when_root_path_is_replaced_by_symlink` |
| A root given through a symlink still works (only components below it are strict) | `root.rs`: `should_open_when_root_given_via_symlink`; `http_security.rs`: `should_serve_site_when_root_argument_is_a_symlink` |
| A directory swapped for a symlink in a concurrent loop never yields the outside bytes (FS-04); the same race against naive `std::fs` does, so the test is not vacuous | `symlink_race.rs`: `should_never_return_outside_bytes_when_directory_is_swapped_for_symlink`, `should_return_outside_bytes_when_path_is_opened_naively_during_swap` |
| A refusal carries no absolute path | `site_content.rs`: `should_not_leak_absolute_path_when_refused` |
| Any `SiteContent` implementation must map a refusal to a refusal, never to contents (TEST-04) | `site_content_contract.rs`: `should_pass_contract_when_site_is_root_dir_over_tempdir`, `should_fail_contract_when_adapter_maps_forbidden_to_file_contents` |
| The capability handle cannot be bypassed through its fields | `compile_fail.rs`: `should_reject_capability_shortcuts_when_adapter_fields_are_private` |
| End to end over HTTP | `crates/preview-cli/tests/http_security.rs`: `should_refuse_symlink_when_it_points_outside_root`, `should_refuse_path_when_symlink_is_in_the_middle`, `should_refuse_symlink_when_it_points_inside_root`, `should_refuse_traversal_when_dot_segments_are_percent_encoded`, `..._slash_is_percent_encoded`, `..._dot_segments_are_double_encoded`; `should_leave_tree_unchanged_when_suite_has_run` (the source tree is byte-identical afterwards) |

Known limits of the capability: a hard link or a mount point inside the root is a real entry
and is served; a directory that can be searched but not read is refused.

## Request resolution: no existence leak

ADR 0039. The order is decode, eligibility, snapshot, open.

- A malformed path is 400, a traversal is 404, an ineligible path is 403 decided from the text
  alone (it reveals nothing about the disk).
- A name requested without a trailing slash that would be denied as a directory (`/vendor`)
  answers exactly like a missing entry, whatever is on disk, so a redirect cannot reveal it.
  Pinned by `should_return_not_found_when_excluded_directory_requested_without_slash`
  (`crates/preview-application/tests/resolve_request.rs`).
- Other outcomes: `should_return_bad_request_when_encoding_malformed`,
  `should_return_not_found_when_path_attempts_traversal`,
  `should_forbid_when_component_hidden`, `should_report_unavailable_when_open_fails_with_io_error`
  (a fixed 500 with no detail).
- A directory without an index is a 404, never a listing:
  `should_return_404_without_listing_when_index_missing`
  (`crates/preview-adapter-http/tests/directory.rs`); a directory redirect is built from
  validated components only, `should_build_location_from_components_when_request_spells_name_encoded`.

## HTTP surface

`crates/preview-adapter-http/src/router.rs` and `errors.rs`. Layers, outermost first: harden,
Host gate, method gate.

| Rule | Pinned by |
|:--|:--|
| Only `GET` and `HEAD`; anything else is 405 with `Allow: GET, HEAD`, internal paths included | `should_return_405_with_allow_when_method_is_post`, `should_return_405_when_method_is_post_on_internal_path` (`tests/router.rs`); `should_return_405_with_allow_when_method_is_not_get_or_head` (`http_security.rs`) |
| Every response, errors included, carries `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` | `should_send_no_store_and_nosniff_when_answering_any_status` (`http_security.rs`), `should_set_no_store_when_error_response` (`tests/router.rs`), `should_label_content_type_and_nosniff_when_serving_by_extension` (`tests/files.rs`) |
| Error bodies are fixed constants: nothing from the request or the filesystem reaches them | `should_not_echo_request_when_error_response` (`tests/router.rs`) |
| No CORS header is ever added | `should_not_emit_cors_headers_when_origin_sent` (`tests/host.rs`) |
| Internal routes are answered by handlers, never from disk | `should_not_touch_disk_when_internal_path_requested` (`tests/router.rs`) |
| A `Range` request gets a full 200 and no `Accept-Ranges` is advertised | `should_return_full_200_when_range_requested`, `should_omit_accept_ranges_when_serving` (`tests/files.rs`) |
| Source files are never modified | `should_leave_tree_unchanged_when_suite_has_run` (`http_security.rs`) |

### Streaming resource bounds

File bodies are streamed in 64 KiB chunks, one short `spawn_blocking` read per poll, started
only when the connection asks for more (`crates/preview-adapter-http/src/stream.rs`). A client
that stops reading holds one buffered chunk and no thread, so it cannot starve the blocking
pool that request resolution also uses (the HTTP-02 fix). Dropping the body stops the reading.
The promised `Content-Length` is the size at open time: a file that grew is cut off at that
length, one that shrank aborts the connection instead of sending a truncated body that looks
complete.

Pinned by `should_complete_other_request_when_client_stalls_body` and
`should_stop_reading_when_body_dropped` (`tests/stream.rs`), `should_not_exceed_length_when_file_grew`
and `should_abort_body_when_file_shorter_than_length` (`tests/files.rs`).

These bounds cover files streamed from disk. HTML that is checked and rewritten (at most
8 MiB) is held in memory instead, under a shared 128 MiB budget: each page reserves twice its
size plus 64 KiB before it is read and keeps it until its body is sent or dropped, and
never before the blocking read and rewrite have ended (a request abandoned mid-rewrite
cannot free its share early), sent in 64 KiB copies; a request that does not fit waits (ADR 0046). Clients that stop reading can
therefore delay other HTML responses until they disconnect, but cannot grow memory past the
budget. Pinned by `should_hold_budget_until_body_is_consumed_when_html_served`
(`tests/inject.rs`), `should_wait_for_release_when_budget_exhausted`,
`should_release_reservation_when_body_sent_or_dropped` and
`should_cap_reservation_when_charge_exceeds_budget` (`src/inject/budget.rs`),
`should_keep_reservation_until_blocking_work_ends_when_request_dropped`
(`src/inject/response.rs`).

### HTML injection

`crates/preview-adapter-http/src/inject/` (plan.md §6; ADR 0022, ADR 0043, ADR 0046). The
reload script is added to the response only; the file on disk never changes.

| Rule | Pinned by |
|:--|:--|
| Only a `GET` navigation is injected: `Sec-Fetch-Dest` missing, `document` or `iframe`; any other destination gets the file's bytes unchanged | `should_inject_snapshot_revision_when_document_get`, `should_inject_when_fetch_dest_missing_or_iframe`, `should_not_modify_when_sec_fetch_dest_empty` (`tests/inject.rs`) |
| `HEAD` sends the `GET`'s headers, including the rewritten `Content-Length`, and no body | `should_not_inject_when_head` (`tests/inject.rs`) |
| Error responses and non-HTML files are never injected | `should_not_inject_when_response_is_error` (`tests/inject.rs`), `should_serve_stylesheet_bytes_unchanged_when_requested` (`crates/preview-cli/tests/serve_slice.rs`) |
| The tag carries the site state read before the file was opened, attribute values escaped | `should_inject_snapshot_revision_when_document_get` (`tests/inject.rs`), `should_escape_attribute_value_when_value_holds_markup` (`src/inject/mod.rs`) |
| `Content-Length` is the rewritten length; `ETag` and `Last-Modified` are not sent | `should_set_content_length_of_rewritten_body_when_injected` (`tests/inject.rs`) |
| One insertion, before the real `</body>` (not one in a comment, script, attribute or `<template>`), else at the end; never a second copy; a rewriter failure serves the original bytes | `should_insert_before_real_body_end_when_script_contains_fake_body_tag`, `should_insert_before_body_end_when_comment_contains_fake_body_tag`, `should_skip_template_body_end_when_template_holds_body_tags`, `should_insert_once_before_first_body_end_when_body_end_repeats`, `should_not_duplicate_when_client_already_present`, `should_accept_only_single_insertion_when_output_compared` (`src/inject/rewrite.rs`), `should_serve_original_and_warn_once_when_rewrite_fails` (`tests/inject.rs`) |
| A page that already loads the client (a saved served page) gets no second tag; its tag's `data-instance` and `data-revision` are set to the current snapshot, every other byte kept, so a stale copy cannot loop the browser | `should_refresh_snapshot_attributes_when_client_tag_is_stale` (`src/inject/rewrite.rs`), `should_refresh_stale_client_tag_when_page_already_loads_client` (`tests/inject.rs`), `should_serve_current_snapshot_when_page_holds_stale_client_tag` (`crates/preview-cli/tests/injection_sse.rs`) |
| A page is labelled `charset=utf-8` and injected only when its bytes are UTF-8 and either it starts with a UTF-8 BOM (which wins, as in a browser) or its first `<meta>` charset declaration agrees; otherwise it is served unchanged as `text/html` | `should_not_label_utf8_when_meta_names_shift_jis`, `should_serve_unchanged_and_warn_when_html_is_latin1`, `should_label_utf8_and_inject_when_bom_overrides_meta_charset` (`tests/inject.rs`), `should_classify_encoding_when_meta_varies`, `should_classify_as_utf8_when_bom_precedes_other_meta` (`src/inject/charset.rs`) |
| A page over 8 MiB is streamed unchanged, never read whole | `should_serve_unchanged_and_warn_once_when_html_over_limit` (`tests/inject.rs`) |
| A page not injected is warned on stderr once per code and request path (`html-too-large`, `html-not-utf8`, `html-rewrite-failed`, `csp-blocks-live-reload`), the path re-encoded from validated parts so no query string, absolute path or control character is printed; never in an HTTP response | `should_report_once_per_code_and_path_when_repeated`, `should_stop_reporting_when_memo_is_full` (`src/warnings.rs`), `should_show_encoded_path_when_warning_names_page` (`src/inject/response.rs`), `should_warn_on_stderr_once_when_html_is_not_utf8` (`crates/preview-cli/tests/serve_slice.rs`) |
| A page whose `<meta http-equiv="Content-Security-Policy">` (in the head, any number, comma-separated policies included) would block the client script (`script-src-elem`, else `script-src`, else `default-src`) or the reload connection (`connect-src`, else `default-src`) is still injected and warned once per path as `csp-blocks-live-reload`; the policy is never changed and no header is added. `'self'`, `*` and `http:` allow; `https:` alone blocks (this server speaks plain `http`). A host-source may match this server, so it is not reported; other violations show only in the browser console | `should_warn_when_script_src_excludes_self`, `should_warn_when_default_src_none_and_no_connect_src`, `should_not_warn_when_self_allowed`, `should_warn_when_any_of_several_policies_blocks`, `should_warn_when_only_https_scheme_allowed` (`src/inject/csp.rs`), `should_collect_head_csp_metas_when_page_has_them` (`src/inject/rewrite.rs`), `should_preserve_meta_csp_bytes_when_injecting`, `should_warn_once_per_path_when_meta_csp_blocks_reload` (`tests/inject.rs`) |
| Source files are never modified by injection | `should_leave_source_bytes_unchanged_when_injected` (`tests/inject.rs`), `should_leave_source_bytes_unchanged_when_served` (`crates/preview-cli/tests/serve_slice.rs`) |

## Host and Origin rules

Policy lives in the pure domain (`crates/preview-domain/src/request_policy.rs`, ADR 0012,
ADR 0037); the adapter passes raw header values (`crates/preview-adapter-http/src/host.rs`,
`sse_guard.rs`). The Host gate runs on every route, internal ones included, and before the
method gate.

**Host (421 `Host not allowed`)**, ADR 0021 and ADR 0037:

- Allowed: the bound numeric IP with the bound port, and `localhost:<port>` (any ASCII case)
  when the listener is loopback. With `--allow-remote`, numeric IPs only (the bound one; any
  numeric IP on a wildcard listener). Never a DNS name, which closes DNS rebinding.
- Rejected: wrong port, missing Host, userinfo, trailing dot, non-canonical IPv4 (`127.1`,
  padded, octal, hex), IPv6 zone ids, `0.0.0.0` and `::` as a destination, an IPv4-mapped IPv6
  address standing in for IPv4, a wildcard listener without `--allow-remote` (fails closed).
- The adapter also rejects a duplicated Host header, a Host that is not UTF-8, an
  absolute-form request target whose authority disagrees with Host, and one whose scheme is
  not `http` (ADR 0044).

Pinned by `crates/preview-domain/src/request_policy.rs` (`should_reject_when_host_is_dns_name`,
`should_reject_when_port_mismatches`, `should_reject_userinfo_when_host_carries_credentials`,
`should_reject_non_canonical_ipv4_when_shorthand_or_padded`,
`should_reject_ipv6_when_zone_id_present`, `should_reject_unspecified_host_when_wildcard_and_allow_remote`,
`should_reject_everything_when_wildcard_without_allow_remote`,
`should_reject_dns_name_when_wildcard_and_allow_remote`, and the allow cases) and by
`crates/preview-adapter-http/tests/host.rs` (`should_reject_with_421_when_host_is_rebinding_name`,
`..._host_port_differs`, `..._host_missing`, `..._host_header_is_duplicated`,
`..._host_is_not_utf8`, `..._absolute_target_disagrees_with_host`,
`..._absolute_target_scheme_is_not_http`, `..._internal_route_has_bad_host`,
`should_reject_with_421_before_405_when_post_has_bad_host`). End to end:
`should_reject_with_421_when_host_names_another_site` (`http_security.rs`).

**Event stream (`/__site_preview/events`)**, ADR 0044. `GET` and `HEAD` run the Origin check
first, before anything is subscribed:

- `Sec-Fetch-Site: cross-site` is 403. An `Origin` must be `http://` plus the exact authority
  the Host gate accepted (port included; `localhost` and `127.0.0.1` differ); `null`, other
  schemes and anything with a path, query or fragment are 403. A repeated `Origin` or
  `Sec-Fetch-Site`, or a value that is not visible ASCII, is 403 without consulting the policy.
  Neither header present is allowed (agents, `curl`), because native `EventSource` cannot add
  headers.
- At most 64 streams per server (`DEFAULT_MAX_STREAMS`); the 65th gets 503 with
  `Retry-After: 2`. The permit is owned by the stream, so a disconnect, a shutdown and a closed
  reload state all return it. `HEAD` takes no permit. A browser's `EventSource` never retries a
  non-200 answer, so the reload client re-creates it after a backoff of 1 s doubling to 30 s,
  without reloading the page (ADR 0044, amendment).

Pinned by `crates/preview-domain/src/request_policy.rs` (`should_reject_when_origin_is_cross_site`,
`should_reject_when_origin_conflicts_with_host`, `should_reject_when_origin_is_null`,
`should_reject_when_origin_scheme_is_not_http`, `should_reject_when_origin_is_malformed`,
`should_allow_sse_when_no_browser_headers`) and `crates/preview-adapter-http/tests/sse.rs`
(`should_reject_with_403_when_origin_cross_site`, `..._origin_header_is_ambiguous`,
`..._origin_differs_from_accepted_host`, `should_accept_when_no_origin_headers`,
`should_return_503_when_limit_reached`, `should_return_503_when_default_limit_of_64_streams_open`,
`should_release_permit_when_client_disconnects`, `should_release_permit_when_stream_ends_on_shutdown`,
`should_answer_head_without_taking_permit_when_limit_reached`,
`should_reject_with_403_when_head_is_cross_site`).

## Binding and `--allow-remote`

The default bind is `127.0.0.1`. A non-loopback host (`0.0.0.0` and `::` included) is a usage
error (exit 2, `refusing to bind <host> without --allow-remote`) unless `--allow-remote` is
given. With it, the CLI prints this warning on stderr at startup, and only for a non-loopback
bind:

```
site-preview: warning[remote-binding]: listening on <host> with no authentication: anyone who can reach this address can read the site; do not use it on an untrusted network
```

Pinned by `should_exit_2_when_host_is_wildcard_without_allow_remote`,
`should_warn_and_serve_when_host_is_wildcard_with_allow_remote` and
`should_not_warn_when_loopback_with_allow_remote` (`crates/preview-cli/tests/flags.rs`), and
`should_refuse_non_loopback_host_when_allow_remote_absent` (`crates/preview-composition/src/config.rs`).
The Host policy is built from the address actually bound, so the printed URL is one the gate
accepts, never the wildcard. There is no authentication; the Host gate is not an access
control for a LAN.

## Connection limits and shutdown drain

A stalled or slow client cannot hold the server open (ADR 0045): a request head not read within
10 s closes the connection, at most 128 connections are served at once (one accepted beyond the
cap is closed), and the first SIGINT or SIGTERM ends the event streams and drains for at most
3 s before exiting 0. A second signal during the drain exits at once with 130 or 143. There is
no write timeout: a client that stops reading holds its connection until it goes or the server
stops. Pinned by `should_exit_0_within_drain_bound_when_clients_stall`,
`should_exit_130_when_second_sigint_arrives_during_drain` and `should_exit_0_when_sigterm_received`
(`crates/preview-cli/tests/lifecycle.rs`), and `should_close_connection_when_head_not_sent_within_timeout`,
`should_close_idle_keep_alive_connection_when_timeout_passes`, `should_close_excess_connection_when_cap_reached`,
`should_drop_stalled_connection_when_drain_bound_passes` and `should_stop_at_once_when_second_stop_arrives_during_drain`
(`crates/preview-composition/src/connections.rs`).

## No telemetry, and logging

Site Preview makes no outbound request, collects no analytics and has no remote logging
(plan.md §2, "Logging"). It writes concise diagnostics to stderr (the startup banner,
warnings such as `warning[remote-binding]` and the HTML injection warnings, errors) and keeps no per-request log: no request
bodies, query strings or cookies are recorded. Error responses carry fixed text (see HTTP
surface), and refusals from the filesystem adapter carry a category, never a path or an OS
error text. The per-instance id used by the reload protocol identifies a server process, not a
user or browser (plan.md §7.1).

## Supply chain

| Control | Where | Pinned by |
|:--|:--|:--|
| `cargo-deny`: vulnerabilities and yanked crates fail at any depth; an unmaintained direct dependency fails; licences limited to MIT, Apache-2.0, BSD-2/3-Clause, ISC, Unicode-3.0, Zlib; unknown registries and git sources are denied; wildcard requirements are denied | `deny.toml`, `cargo xtask security` (stage `deny`), ADR 0041 | `should_fail_sources_check_when_a_crate_depends_on_a_git_source`, `should_pass_sources_check_when_a_crate_has_no_dependencies` (`tools/xtask/tests/security_fixtures.rs`) |
| The fuzz crate's own lockfile, `fuzz/Cargo.lock`, which the root run does not see: the same policy plus NCSA for `libfuzzer-sys` only (fuzz-only, never in the product) | `fuzz/deny.toml`, `cargo xtask security` (stage `deny-fuzz`), ADR 0065 | `should_match_root_policy_when_fuzz_exception_is_removed` (`tools/xtask/src/security.rs`) |
| MPL-2.0 is allowed for exactly four crates (`cssparser`, `cssparser-macros`, `dtoa-short`, `selectors`), pulled in by the HTML rewriter; any other MPL-2.0 crate fails | `deny.toml` `[[licenses.exceptions]]`, ADR 0043 | the `deny` stage |
| Gitleaks over the full history; refuses to run on a shallow or empty clone | `.gitleaks.toml`, `cargo xtask security`, ADR 0041 | `should_fail_secret_scan_when_history_holds_an_aws_style_key`, `should_fail_secret_scan_when_key_was_removed_but_stays_in_history`, `should_pass_secret_scan_when_history_holds_no_secret` |
| `actionlint` over the workflows | `cargo xtask security` | `should_fail_workflow_lint_when_expression_is_invalid` |
| Every GitHub Action and Docker image is pinned by full commit SHA or digest; a YAML-parsed walk of workflows and local actions that fails closed (flow mappings, quoted keys, reusable workflows, merge keys) | `tools/xtask/src/action_pins.rs`, ADR 0041 | the `should_*_when_*` tests in that file, e.g. `should_report_reference_when_action_uses_a_tag`, `should_report_reference_when_step_is_a_flow_mapping`, `should_pass_when_repository_workflows_are_pinned` |
| CodeQL (Rust) with `security-events: write` on that job only | `.github/workflows/codeql.yml` | the pin check and `actionlint` |
| Dependabot weekly for `cargo`, `github-actions` and the two npm directories, minor and patch grouped | `.github/dependabot.yml` | review |
| The `security` workflow runs the same `cargo xtask security` stage as local development | `.github/workflows/security.yml` | CI |
| OpenSSF Scorecard on each push to `main` and weekly, results in code scanning and the README badge; `security-events: write` and `id-token: write` on that job only (ADR 0082) | `.github/workflows/scorecard.yml` | the pin check and `actionlint` |
| The Codecov upload holds `id-token: write` in a `workflow_run` job that checks out the reported commit but runs no repository code; no Codecov token is stored (ADR 0082, ADR 0086) | `.github/workflows/coverage-report.yml` | the pin check and `actionlint` |

`Cargo.lock` is committed and builds use `--locked`. A passing advisory scan is not a
guarantee that dependencies are safe (foundation.md §8).

## Architecture rules that protect these properties

Enforced by `cargo xtask architecture`; the catalog, with each rule's negative fixture, is
[`architecture-rules.md`](architecture-rules.md).

- `ARCH-domain-purity`: the domain, which holds the path, eligibility, Host and Origin
  policies, does no I/O and reads no clock, environment or network (ADR 0012).
- `ARCH-fs-authority`: only `preview-adapter-fs` and the composition bootstrap name filesystem
  authority, so no request handler can open a path by name.
- `ARCH-layer-isolation`: adapters never name a sibling adapter or composition.
- `ARCH-unsafe-code` and the `unsafe_code = "forbid"` workspace lint: no `unsafe` in owned code.
- `ARCH-lint-suppression`, `ARCH-ignored-test`, `ARCH-test-sleep`, `ARCH-unfinished-code`:
  no silent `#[allow]`, `#[ignore]`, sleeps in tests, `todo!` or `dbg!`.
- `ARCH-unlisted-external`, `ARCH-external-source`: every direct third-party dependency is
  listed per crate, and a non-crates.io source needs an ADR.

## Known limitations

These are choices, not bugs (plan.md §15); a report that only restates one is out of scope.

- **`--allow-remote` has no authentication.** It is unsafe on an untrusted network.
- **Fetch Metadata on a LAN address.** Browsers send `Sec-Fetch-*` only to potentially
  trustworthy origins. A page opened at `http://<LAN address>` has none, so HTML injection
  classifies requests on its fallback rules there, and a cross-site `no-cors` request to the
  event stream carries neither `Origin` nor `Sec-Fetch-Site`, passes the checks and can hold
  stream slots until the 64-stream limit is reached. It can deny live reload; it cannot read
  the stream (ADR 0037, amendment). The limit bounds the cost, it does not prevent it.
- **Hidden paths are unreachable**, `.well-known` included.
- **Symlinks are refused** everywhere under the root, even those pointing inside it.
- **Served HTML is rewritten** (the reload script is added to the response; the file on disk
  never changes).
- **A restrictive Content-Security-Policy** in a page may stop live reload from working. A
  policy in a `<meta>` tag is checked and warned about (`csp-blocks-live-reload`); a policy from
  elsewhere, or a violation only the browser can see (a host-source, a nonce on other scripts),
  shows only in the browser console.
- **Some pages get no live reload without a warning.** A `</body>` inside SVG `<style>`
  text or an unclosed `<template>` can receive the script where it is inert; a frameset
  document has no body to run it in; a page loaded as a frame (`Sec-Fetch-Dest: frame`, a
  `<frame>` of a frameset) is never injected. The page itself is served intact.
- **Not a sandbox** against other processes of the same user; hard links and mount points
  inside the root are served.
- **Network-mounted directories** may not deliver native change events.
- **Range requests** get a full 200 response.

## Planned

Not yet enforced; do not rely on them until the task lands. Nothing is planned at the moment.

## Fuzzing

The `fuzz/` crate (ADR 0063, ADR 0065) is outside the workspace and builds only with the pinned
nightly. Each target calls the real code and asserts properties, so a crash is either a panic
in the code under test or a broken property.

| Target | Code under test | Properties |
|:--|:--|:--|
| `decode_path` | `decode_request_path`, `eligibility::path` (domain) | No panic. Every accepted path has only single-entry components (no `.`, `..`, empty, `/`, `\` or control character); `eligibility::path` returns exactly the verdict and reason of an oracle written in the target from the documented rules (HFS-ignorable code points, the reserved root name, excluded directories, editor artifacts, hidden names, compared after its own copy of the case fold: ASCII, `ſ` as `s`, `İ` and `ı` as `i`), in both directions: eligible exactly when the oracle denies nothing; decoding is deterministic and a re-encoded path decodes to itself. Seeds cover the confusable spellings. |
| `rewrite_html` | `inject::rewrite` (`preview-adapter-http`) over arbitrary bytes | No panic. On `Injected`, the output is the input with the tag inserted at one offset (removing it gives back the input bytes) and a separate strict `lol_html` parse sees exactly one client `<script>`. On `AlreadyPresent`, every client tag carries the snapshot attributes. |
| `host_header` | `HostPolicy::check`, `OriginPolicy::check` (domain), over eight listener addresses, both `--allow-remote` settings and any port | No panic. Any `Allow` has the bound port; `localhost` only on a loopback listener; an allowed address is never unspecified and is the bound one, or any one on a wildcard listener with `--allow-remote`; the decision is deterministic; the Origin a browser sends for an accepted authority is allowed. |
| `change_sequence` | `ChangeCoordinator` (`preview-application`) with its real debouncer and `ReloadState`, fed change batches, overflows, failures, readiness, reconciles, clock steps and settles | No panic. The revision changes only on a publish, by exactly one; nothing is published unless a non-ignored event (decided by the target's own table of which of its names each deny rule refuses, not by the production `classify`), an overflow or a recovery came since the last publish; a published change set has at most 64 entries, a non-full one only css, js or asset entries, and a rescan is always full. |

Each target's input layout is documented at the top of its file, and its committed seeds are in
`fuzz/seeds/<target>/` (a xtask unit test fails when a target has no seeds, no `[[bin]]` entry
or no place in the `fuzz.yml` matrix). Run one locally with
`cargo xtask fuzz <target> --time 60`; it needs `cargo-fuzz` and the pinned nightly
(`cargo xtask install-tools cargo-fuzz nightly`). The generated corpus goes to the ignored
`fuzz/corpus/<target>/`.

`.github/workflows/fuzz.yml` runs every target nightly and on demand (`workflow_dispatch`, with
a time per target from 1 to 3600 seconds, 600 by default), resumes from the corpus the previous
run cached, and uploads a crash input as the artifact `fuzz-crash-<target>-<run>-<attempt>`.
The fuzz crate's own lockfile is audited by the `deny-fuzz` stage (see Supply chain).

Fuzz targets are formatted like the workspace (`fuzz-fmt` in `quick` and `check`), carry the
workspace's lint table in `fuzz/Cargo.toml`, and are linted with Clippy on the pinned nightly
in `fuzz.yml` (ADR 0069).

### Results

Local runs of `cargo xtask fuzz <target> --time 60` (one core, `nightly-2026-10-01`, from the
committed seeds and the local corpus). `cov` is libFuzzer's count of covered edges at the end
of the run.

| Target | Date | Runs | `cov` | Crashes |
|:--|:--|--:|--:|--:|
| `host_header` | 2026-10-13 (SEC-05) | 6,367,011 | 545 | 0 |
| `rewrite_html` | 2026-10-13 (SEC-05) | 105,086 | 6778 | 0 |
| `change_sequence` | 2026-10-13 (SEC-05) | 221,846 | 788 | 0 |
| `decode_path` | 2026-10-13 (FORMAL-10 review, the two-way oracle) | 382,049 | 533 | 0 |
| `change_sequence` | 2026-10-13 (FORMAL-10 review, the explicit ignored-name table) | 273,984 | 813 | 0 |

A changed property that the fuzzer then fails to break still needs a control: with the oracle's
`ſ` mapping removed, the committed seed `confusable-long-s-raw` fails `decode_path` at once
(`eligibility::path disagrees with the oracle for ["node_moduleſ", "x"]`).

### Triage

A fresh fuzz run must leave no untriaged crash (foundation.md section 12, phase 5).

1. **Get the input.** Download the workflow artifact (or take the file libFuzzer named in
   `fuzz/artifacts/<target>/` locally): `crash-*` is a panic or failed assertion, `oom-*` an
   allocation over the RSS limit, `timeout-*` an input that ran too long.
2. **Reproduce.** `cargo xtask fuzz <target> --reproduce <file>` runs that one input and prints
   the panic message, which names the broken property. A crash that does not reproduce is
   still a finding: note the nightly and the run, and rerun the target with the corpus.
3. **Minimise.** `cargo +<nightly> fuzz tmin <target> <file>` (the nightly from
   `tools/versions.toml`, run in `fuzz/`) shrinks it; read the layout comment of the target to
   decode the bytes.
4. **Decide whose bug it is.** A panic in the code under test, or a property the code breaks,
   is a product bug. A property the documented behaviour does not promise is a target bug: fix
   the target and say why in the commit. A security consequence (a path, Host or injection
   rule that does not hold) is reported privately first, as `SECURITY.md` says.
5. **Fix with a regression test.** Add a unit test with the minimised input to the crate that
   owns the code (`should_<outcome>_when_<condition>`), see it fail, then fix. A lifecycle,
   watcher or reload behaviour change updates the Quint model, its scenarios and the
   conformance driver first (foundation.md section 6.2). Add the minimised input to
   `fuzz/seeds/<target>/` so every later run starts from it.
6. **Never** silence a crash by loosening a property, catching the panic in the target, or
   dropping a seed.
