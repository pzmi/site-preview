//! Black-box tests of how `serve` starts and stops (plan.md section 9, ADR 0045, ADR 0051): a
//! refused start exits 1 with what to do next, SIGINT and SIGTERM exit 0, an open event
//! stream is ended rather than waited for, the listener refuses connections once shutdown
//! starts, a client that stalls never holds the process past the drain bound, and a second
//! SIGINT during the drain exits 130 at once.

pub mod support;

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use predicates::prelude::*;
use preview_testkit::poll::wait_until;
use support::{DEADLINE, PreviewProcess, Site, site_preview};

/// The CLI's drain bound (`ServerLimits::DEFAULT`, ADR 0045).
const DRAIN: Duration = Duration::from_secs(3);
/// How long past the drain bound a stopping process may take: well under the 10 s header
/// timeout, so a pass shows the drain bound ended the run, not a connection timing out.
const MARGIN: Duration = Duration::from_secs(5);
/// Far larger than the loopback socket buffers: a client that stops reading it leaves the
/// server blocked mid-write.
const BIG: usize = 64 << 20;

/// A connection that sent half a request head and nothing more, once the server has accepted
/// it (so the drain holds it rather than the closing listener resetting it).
fn half_sent_head(server: &PreviewProcess) -> TcpStream {
    let mut stream = TcpStream::connect(server.authority()).expect("the server accepts");
    let head = format!("GET / HTTP/1.1\r\nHost: {}\r\nAccept:", server.authority());
    stream
        .write_all(head.as_bytes())
        .expect("half a head is sent");
    server.await_accepted();
    stream
}

/// A connection that asked for `/big.bin`, read the status line's start (so the server is
/// writing the body) and then stopped reading.
fn stalled_reader(server: &PreviewProcess) -> TcpStream {
    let mut stream = TcpStream::connect(server.authority()).expect("the server accepts");
    stream
        .set_read_timeout(Some(DEADLINE))
        .expect("a read timeout");
    let head = format!(
        "GET /big.bin HTTP/1.1\r\nHost: {}\r\n\r\n",
        server.authority()
    );
    stream
        .write_all(head.as_bytes())
        .expect("the request is sent");
    let mut start = [0_u8; 12];
    stream.read_exact(&mut start).expect("the reply starts");
    assert_eq!(&start, b"HTTP/1.1 200");
    stream
}

fn site_with_big_file() -> Site {
    let site = Site::copy();
    fs::write(site.path().join("big.bin"), vec![b'x'; BIG]).expect("a big file");
    site
}

/// Waits for the drain notice on stderr: from then on the server is draining.
fn await_drain_notice(server: &PreviewProcess) {
    let mut seen = Vec::new();
    // Polled: the notice comes from the subprocess once it has handled the signal.
    wait_until(DEADLINE, "the drain notice on stderr", || {
        seen.extend(server.stderr().try_iter());
        seen.iter()
            .any(|line| line.starts_with("site-preview: shutting down: "))
    })
    .unwrap_or_else(|timeout| panic!("{timeout}; stderr: {seen:?}"));
}

#[test]
fn should_exit_0_within_drain_bound_when_clients_stall() {
    // given: one client stuck in its request head, one that stopped reading a large body
    let site = site_with_big_file();
    let server = PreviewProcess::serve(&site);
    let _head = half_sent_head(&server);
    let _reader = stalled_reader(&server);

    // when
    server.signal("INT");

    // then
    await_drain_notice(&server);
    let status = server.wait_exit(DRAIN + MARGIN);
    assert_eq!(status.code(), Some(0), "{status}");
}

#[test]
fn should_exit_130_when_second_sigint_arrives_during_drain() {
    // given: a stalled reader keeps the drain going for the whole bound
    let site = site_with_big_file();
    let server = PreviewProcess::serve(&site);
    let _reader = stalled_reader(&server);
    server.signal("INT");
    await_drain_notice(&server);

    // when
    server.signal("INT");

    // then
    let status = server.wait_exit(DEADLINE);
    assert_eq!(status.code(), Some(130), "{status}");
}

#[test]
fn should_not_announce_drain_when_only_a_tab_is_open() {
    // given: a browser tab's event stream and an idle keep-alive connection, which a
    // shutdown ends at once
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let mut events = server.events(&[]).expect("the stream opens");
    events.baseline();
    let mut idle = TcpStream::connect(server.authority()).expect("the server accepts");
    idle.set_read_timeout(Some(DEADLINE))
        .expect("a read timeout");
    let head = format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", server.authority());
    idle.write_all(head.as_bytes())
        .expect("the request is sent");
    let mut start = [0_u8; 12];
    idle.read_exact(&mut start).expect("the reply starts");

    // when
    server.signal("INT");

    // then: the exit is immediate, so nothing claims the server is waiting
    let finished = server.wait_exit_with_output(DEADLINE);
    assert_eq!(finished.status.code(), Some(0), "{:?}", finished.status);
    assert!(
        !finished
            .stderr
            .iter()
            .any(|line| line.contains("shutting down")),
        "{:?}",
        finished.stderr
    );
}

#[test]
fn should_exit_0_when_sigterm_received() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    assert_eq!(server.get("/").status, 200);

    // when
    server.signal("TERM");

    // then
    let status = server.wait_exit(DEADLINE);
    assert_eq!(status.code(), Some(0), "{status}");
}

#[test]
fn should_exit_0_when_sigint_received_with_open_sse_connection() {
    // given: a browser tab's event stream, past its `state` baseline
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let mut events = server.events(&[]).expect("the stream opens");
    events.baseline();

    // when
    server.signal("INT");

    // then: the server ended the stream itself, and the process exited cleanly
    assert!(
        events.ended_cleanly(),
        "the stream was dropped, not ended by the shutdown"
    );
    let status = server.wait_exit(DEADLINE);
    assert_eq!(status.code(), Some(0), "{status}");
}

#[test]
fn should_close_listener_when_shut_down() {
    // given: a client stuck in its request head keeps the drain going for its whole bound
    let site = Site::copy();
    let server = PreviewProcess::serve(&site);
    let authority = server.authority().to_owned();
    let _head = half_sent_head(&server);

    // when
    server.signal("TERM");

    // then: refused while the process still drains, and after it has exited
    await_drain_notice(&server);
    let during = TcpStream::connect(&authority).map_err(|error| error.kind());
    let status = server.wait_exit(DRAIN + MARGIN);
    let after = TcpStream::connect(&authority).map_err(|error| error.kind());
    assert_eq!(
        during.err(),
        Some(ErrorKind::ConnectionRefused),
        "while draining"
    );
    assert_eq!(status.code(), Some(0), "{status}");
    assert_eq!(
        after.err(),
        Some(ErrorKind::ConnectionRefused),
        "after exit"
    );
}

#[test]
fn should_exit_1_with_message_when_reserved_path_exists() {
    // given: the root holds the reserved name in another letter case
    let site = Site::copy();
    fs::create_dir(site.path().join("__SITE_PREVIEW")).expect("a reserved directory");
    let mut command = site_preview();

    // when
    let assert = command.args(["serve", site.arg()]).assert();

    // then
    assert
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(
            "site-preview: error: cannot serve ",
        ))
        .stderr(predicate::str::contains("`__site_preview`"))
        .stderr(predicate::str::contains("rename or remove it"));
}
