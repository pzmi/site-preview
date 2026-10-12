//! Black-box tests of the output modes (plan.md section 3, ADR 0020): under `--json` stdout
//! carries the `ready` event once and nothing else, flushed as soon as the server accepts;
//! warnings and the shutdown notice stay text on stderr; without it the human block is
//! printed. Every other black-box test discovers its URL through the same `ready` event
//! (`support`), so a regression there fails them all.
//!
//! The `watcher` events are pinned below the binary, because an unavailable or failing
//! native watcher cannot be provoked unprivileged and without touching the shared machine
//! (`watcher.rs`): `should_emit_watcher_degraded_event_when_watcher_fails_at_runtime`
//! (`lifecycle.rs` in composition) and the JSON rendering in `output.rs`.

pub mod support;

use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

use preview_testkit::poll::wait_until;
use serde_json::Value;
use support::{DEADLINE, PreviewProcess, Site};

fn parse(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|error| panic!("not JSON ({error}): {line:?}"))
}

#[test]
fn should_emit_single_ready_line_when_json() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let pid = server.pid();
    let url = server.url().to_owned();
    assert_eq!(server.get("/").status, 200);

    // when
    server.signal("INT");
    let finished = server.wait_exit_with_output(DEADLINE);

    // then: stdout is exactly one JSON line, the ready event
    assert_eq!(finished.status.code(), Some(0), "{:?}", finished.status);
    assert_eq!(finished.stdout.len(), 1, "{:?}", finished.stdout);
    let ready = parse(&finished.stdout[0]);
    assert_eq!(ready["event"], "ready");
    assert_eq!(ready["url"], url.as_str());
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(url.ends_with('/'), "{url}");
    assert_eq!(ready["pid"], pid);
    assert_eq!(ready["watching"], true);
    assert_eq!(
        ready.as_object().map(|fields| fields.len()),
        Some(4),
        "{ready}"
    );
}

#[test]
fn should_flush_ready_when_stdout_is_pipe() {
    // given
    let site = Site::copy();

    // when: the harness reads the event from a pipe while the process keeps running and
    // writes nothing more, so the line was flushed rather than held in a buffer
    let server = PreviewProcess::serve(&site);

    // then
    assert_eq!(server.startup_stdout().len(), 1);
    assert_eq!(parse(&server.startup_stdout()[0])["event"], "ready");
    assert_eq!(server.get("/").status, 200, "the process is still serving");
    assert!(
        server.stdout().try_recv().is_err(),
        "nothing else was written"
    );
}

#[test]
fn should_keep_stdout_json_only_when_warnings_occur() {
    // given: two warnings, a remote bind and a browser that cannot be opened
    let site = Site::copy();
    let server = PreviewProcess::start_without_launcher([
        "serve",
        "--host",
        "0.0.0.0",
        "--allow-remote",
        "--open",
        site.arg(),
    ]);

    // when
    server.signal("INT");
    let finished = server.wait_exit_with_output(DEADLINE);

    // then: the warnings are text on stderr, stdout is the ready event alone
    assert_eq!(finished.stdout.len(), 1, "{:?}", finished.stdout);
    assert_eq!(parse(&finished.stdout[0])["event"], "ready");
    let codes: Vec<_> = finished
        .stderr
        .iter()
        .filter_map(|line| line.strip_prefix("site-preview: warning["))
        .filter_map(|rest| rest.split_once("]: ").map(|(code, _)| code))
        .collect();
    assert!(codes.contains(&"remote-binding"), "{:?}", finished.stderr);
    assert!(codes.contains(&"open-failed"), "{:?}", finished.stderr);
}

#[test]
fn should_keep_drain_notice_on_stderr_when_json_and_a_client_stalls() {
    // given: a client stuck in its request head holds the drain open
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let mut stalled = TcpStream::connect(server.authority()).expect("the server accepts");
    stalled
        .write_all(b"GET / HTTP/1.1\r\nHost:")
        .expect("half a head is sent");
    server.await_accepted();

    // when
    server.signal("INT");
    let finished = server.wait_exit_with_output(Duration::from_secs(8));

    // then
    assert_eq!(finished.status.code(), Some(0), "{:?}", finished.status);
    assert!(
        finished
            .stderr
            .iter()
            .any(|line| line.starts_with("site-preview: shutting down: waiting up to 3s")),
        "{:?}",
        finished.stderr
    );
    assert_eq!(finished.stdout.len(), 1, "{:?}", finished.stdout);
}

#[test]
fn should_print_final_human_output_when_json_is_absent() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::start_human(["serve", site.arg()]);

    // when: the block is one write; its last line may arrive after `Local:`
    let mut block = server.startup_stdout().to_vec();
    // Polled: the lines are read off a pipe by another thread.
    wait_until(DEADLINE, "the `Ready.` line", || {
        block.extend(server.stdout().try_iter());
        block.iter().any(|line| line == "Ready.")
    })
    .unwrap_or_else(|timeout| panic!("{timeout}; stdout: {block:?}"));

    // then
    assert_eq!(block.len(), 5, "{block:?}");
    assert_eq!(block[0], "Site Preview");
    assert!(block[1].starts_with("Serving:  "), "{block:?}");
    assert!(block[1].ends_with("site"), "{block:?}");
    assert_eq!(block[2], format!("Local:    {}", server.url()));
    assert!(
        block[3].starts_with("Watching: ") && block[3].ends_with(" eligible directories"),
        "{block:?}"
    );
    assert_eq!(block[4], "Ready.");
}
