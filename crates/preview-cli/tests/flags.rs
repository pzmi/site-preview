//! Black-box tests of the `serve` flags (plan.md section 3): argument order, `--debounce`
//! bounds, the remote-bind refusal and warning, an explicit port in use, and two instances
//! side by side. Each runs the real binary on a temporary copy of the shared fixture site.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use preview_testkit::poll::wait_until;
use tempfile::TempDir;

/// How long a test waits for the subprocess to print its URL before it fails.
const STARTUP_DEADLINE: Duration = Duration::from_secs(30);

/// A running `site-preview serve`; dropping it kills and reaps the child.
struct Server {
    child: Child,
    url: String,
    stderr: Receiver<String>,
}

impl Server {
    fn start(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_site-preview"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("site-preview starts");
        let stdout = lines(child.stdout.take().expect("stdout is piped"));
        let stderr = lines(child.stderr.take().expect("stderr is piped"));
        let mut server = Self {
            child,
            url: String::new(),
            stderr,
        };
        let mut seen = Vec::new();
        // Polled, not slept: the subprocess prints its URL once the listener is bound, and only
        // the OS scheduler decides when that is.
        let waited = wait_until(STARTUP_DEADLINE, "the `Local:` line on stdout", || {
            seen.extend(stdout.try_iter());
            seen.iter().any(|line| line.starts_with("Local:"))
        });
        if let Err(timeout) = waited {
            panic!("{timeout}; stdout so far: {seen:?}");
        }
        server.url = seen
            .iter()
            .find_map(|line| line.strip_prefix("Local:"))
            .map(|url| url.trim().to_owned())
            .expect("a Local: line");
        server
    }

    /// The `host:port` of the printed URL.
    fn authority(&self) -> &str {
        self.url.trim_start_matches("http://").trim_end_matches('/')
    }

    /// GETs `/` over a fresh connection, with the printed authority as Host, and returns the
    /// status code.
    fn get_root(&self) -> u16 {
        let mut stream = TcpStream::connect(self.authority()).expect("the server accepts");
        stream
            .set_read_timeout(Some(STARTUP_DEADLINE))
            .expect("a read timeout");
        let head = format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.authority()
        );
        stream
            .write_all(head.as_bytes())
            .expect("the request is sent");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("a complete reply");
        reply
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("a status line")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Already exited is fine; reaping is what matters, so no zombie outlives the test.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn lines(stream: impl Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn copy_of_fixture() -> TempDir {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/site");
    let target = tempfile::tempdir().expect("a temporary directory");
    let mut pending = vec![source.clone()];
    while let Some(current) = pending.pop() {
        let relative = current.strip_prefix(&source).expect("beneath the fixture");
        fs::create_dir_all(target.path().join(relative)).expect("a directory copy");
        for entry in fs::read_dir(&current).expect("a readable fixture directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(&source).expect("beneath the fixture");
                fs::copy(&path, target.path().join(relative)).expect("a file copy");
            }
        }
    }
    target
}

fn path_arg(dir: &TempDir) -> &str {
    dir.path().to_str().expect("a UTF-8 temporary path")
}

#[test]
fn should_accept_directory_when_given_after_flags() {
    // given
    let site = copy_of_fixture();

    // when
    let server = Server::start(&[
        "serve",
        "--port",
        "0",
        "--debounce",
        "150ms",
        path_arg(&site),
    ]);

    // then
    assert_eq!(server.get_root(), 200);
}

#[test]
fn should_accept_directory_when_given_before_flags() {
    // given
    let site = copy_of_fixture();

    // when
    let server = Server::start(&["serve", path_arg(&site), "--port", "0", "--debounce", "1s"]);

    // then
    assert_eq!(server.get_root(), 200);
}

#[test]
fn should_exit_2_when_debounce_is_zero() {
    for zero in ["0ms", "0s"] {
        // given
        let site = copy_of_fixture();
        let mut command = AssertCommand::new(env!("CARGO_BIN_EXE_site-preview"));

        // when
        let assert = command
            .args(["serve", "--debounce", zero, path_arg(&site)])
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
        let site = copy_of_fixture();
        let mut command = AssertCommand::new(env!("CARGO_BIN_EXE_site-preview"));

        // when
        let assert = command
            .args(["serve", "--host", host, path_arg(&site)])
            .assert();

        // then
        assert.code(2).stdout("").stderr(format!(
            "site-preview: error: refusing to bind {host} without --allow-remote\n"
        ));
    }
}

#[test]
fn should_warn_and_serve_when_host_is_wildcard_with_allow_remote() {
    // given
    let site = copy_of_fixture();

    // when
    let server = Server::start(&[
        "serve",
        "--host",
        "0.0.0.0",
        "--allow-remote",
        path_arg(&site),
    ]);

    // then: the printed URL is one the Host policy accepts, never the wildcard itself
    assert!(
        server.url.starts_with("http://127.0.0.1:"),
        "{}",
        server.url
    );
    assert_eq!(server.get_root(), 200);
    let warning = server
        .stderr
        .recv_timeout(STARTUP_DEADLINE)
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
    let site = copy_of_fixture();
    let server = Server::start(&["serve", "--allow-remote", path_arg(&site)]);

    // when
    let status = server.get_root();

    // then
    assert_eq!(status, 200);
    assert!(server.stderr.try_recv().is_err(), "no warning on loopback");
}

#[test]
fn should_exit_1_when_explicit_port_in_use() {
    // given: a test listener holds an OS-assigned port
    let site = copy_of_fixture();
    let occupied = TcpListener::bind("127.0.0.1:0").expect("a test listener");
    let port = occupied
        .local_addr()
        .expect("a bound port")
        .port()
        .to_string();
    let mut command = AssertCommand::new(env!("CARGO_BIN_EXE_site-preview"));

    // when
    let assert = command
        .args(["serve", "--port", &port, path_arg(&site)])
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
    let first_site = copy_of_fixture();
    let second_site = copy_of_fixture();

    // when
    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(|| Server::start(&["serve", path_arg(&first_site)]));
        let second = scope.spawn(|| Server::start(&["serve", path_arg(&second_site)]));
        (
            first.join().expect("the first instance starts"),
            second.join().expect("the second instance starts"),
        )
    });

    // then
    assert_ne!(first.url, second.url);
    assert_eq!(first.get_root(), 200);
    assert_eq!(second.get_root(), 200);
}
