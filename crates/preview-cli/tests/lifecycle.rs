//! Black-box tests of how `serve` starts and stops (plan.md section 9, ADR 0045, ADR 0051): a
//! refused start exits 1 with what to do next, SIGINT and SIGTERM exit 0, a client that
//! stalls never holds the process past the drain bound, and a second SIGINT during the drain
//! exits 130 at once.

pub mod support;

use std::fs;
use std::io::{Read, Write};
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

/// A connection that sent half a request head and nothing more.
fn half_sent_head(server: &PreviewProcess) -> TcpStream {
    let mut stream = TcpStream::connect(server.authority()).expect("the server accepts");
    let head = format!("GET / HTTP/1.1\r\nHost: {}\r\nAccept:", server.authority());
    stream
        .write_all(head.as_bytes())
        .expect("half a head is sent");
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
