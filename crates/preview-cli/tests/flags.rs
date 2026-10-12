//! Black-box tests of the `serve` flags (plan.md section 3): argument order, `--debounce`
//! bounds, the remote-bind refusal and warning, an explicit port in use, and two instances
//! side by side. Each runs the real binary on a temporary copy of the shared fixture site.

pub mod support;

use std::net::TcpListener;
use std::thread;

use predicates::prelude::*;
use support::{DEADLINE, PreviewProcess, Site, site_preview};

/// GETs `/` and returns the status code.
fn get_root(server: &PreviewProcess) -> u16 {
    server.get("/").status
}

#[test]
fn should_accept_directory_when_given_after_flags() {
    // given
    let site = Site::copy();

    // when
    let server = PreviewProcess::start(["serve", "--port", "0", "--debounce", "150ms", site.arg()]);

    // then
    assert_eq!(get_root(&server), 200);
}

#[test]
fn should_accept_directory_when_given_before_flags() {
    // given
    let site = Site::copy();

    // when
    let server = PreviewProcess::start(["serve", site.arg(), "--port", "0", "--debounce", "1s"]);

    // then
    assert_eq!(get_root(&server), 200);
}

#[test]
fn should_exit_2_when_debounce_is_zero() {
    for zero in ["0ms", "0s"] {
        // given
        let site = Site::copy();
        let mut command = site_preview();

        // when
        let assert = command
            .args(["serve", "--debounce", zero, site.arg()])
            .assert();

        // then
        assert
            .code(2)
            .stdout("")
            .stderr(predicate::str::contains("--debounce"))
            .stderr(predicate::str::contains("10ms to 10s"));
    }
}

#[test]
fn should_exit_2_when_host_is_wildcard_without_allow_remote() {
    for host in ["0.0.0.0", "::", "192.0.2.1"] {
        // given
        let site = Site::copy();
        let mut command = site_preview();

        // when
        let assert = command.args(["serve", "--host", host, site.arg()]).assert();

        // then
        assert.code(2).stdout("").stderr(format!(
            "site-preview: error: refusing to bind {host} without --allow-remote\n"
        ));
    }
}

#[test]
fn should_warn_and_serve_when_host_is_wildcard_with_allow_remote() {
    // given
    let site = Site::copy();

    // when
    let server =
        PreviewProcess::start(["serve", "--host", "0.0.0.0", "--allow-remote", site.arg()]);

    // then: the printed URL is one the Host policy accepts, never the wildcard itself
    assert!(
        server.url().starts_with("http://127.0.0.1:"),
        "{}",
        server.url()
    );
    assert_eq!(get_root(&server), 200);
    let warning = server
        .stderr()
        .recv_timeout(DEADLINE)
        .expect("a warning line on stderr");
    assert!(
        warning.starts_with("site-preview: warning[remote-binding]: listening on 0.0.0.0 "),
        "{warning}"
    );
    assert!(warning.contains("no authentication"), "{warning}");
}

#[test]
fn should_not_warn_when_loopback_with_allow_remote() {
    // given
    let site = Site::copy();
    let server = PreviewProcess::start(["serve", "--allow-remote", site.arg()]);

    // when
    let status = get_root(&server);

    // then
    assert_eq!(status, 200);
    let stderr = server.stop_and_stderr();
    assert!(stderr.is_empty(), "no warning on loopback: {stderr:?}");
}

#[test]
fn should_exit_1_when_explicit_port_in_use() {
    // given: a test listener holds an OS-assigned port
    let site = Site::copy();
    let occupied = TcpListener::bind("127.0.0.1:0").expect("a test listener");
    let port = occupied
        .local_addr()
        .expect("a bound port")
        .port()
        .to_string();
    let mut command = site_preview();

    // when
    let assert = command
        .args(["serve", "--port", &port, site.arg()])
        .assert();

    // then
    assert
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(format!(
            "site-preview: error: cannot listen on 127.0.0.1:{port}: "
        )));
    drop(occupied);
}

#[test]
fn should_serve_two_instances_when_started_concurrently() {
    // given
    let first_site = Site::copy();
    let second_site = Site::copy();

    // when
    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(|| PreviewProcess::start(["serve", first_site.arg()]));
        let second = scope.spawn(|| PreviewProcess::start(["serve", second_site.arg()]));
        (
            first.join().expect("the first instance starts"),
            second.join().expect("the second instance starts"),
        )
    });

    // then
    assert_ne!(first.url(), second.url());
    assert_eq!(get_root(&first), 200);
    assert_eq!(get_root(&second), 200);
}
