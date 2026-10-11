//! Black-box tests of `site-preview serve`: the real binary on a temporary copy of the shared
//! fixture site (ADR 0016), a real listener on an OS-assigned port, and a plain HTTP client.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
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

/// A running `site-preview serve`; dropping it kills and reaps the child, also on a failed
/// assertion.
struct Server {
    child: Child,
    url: String,
}

impl Server {
    fn start(directory: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_site-preview"))
            .arg("serve")
            .arg(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("site-preview starts");
        let lines = read_lines(child.stdout.take().expect("stdout is piped"));
        let mut server = Self {
            child,
            url: String::new(),
        };
        let mut seen = Vec::new();
        // Polled, not slept: the subprocess prints its URL once the listener is bound, and only
        // the OS scheduler decides when that is.
        let waited = wait_until(STARTUP_DEADLINE, "the `Local:` line on stdout", || {
            seen.extend(lines.try_iter());
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

    fn get(&self, path: &str) -> (u16, Vec<u8>) {
        let reply = self.fetch(path);
        (reply.status, reply.body)
    }

    /// GETs `path` (which may carry a query) without following redirects.
    fn fetch(&self, path: &str) -> Reply {
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .new_agent();
        let url = format!("{}{}", self.url, path.trim_start_matches('/'));
        let mut response = agent.get(&url).call().expect("the server answers");
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let content_type = header("content-type");
        let location = header("location");
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_vec().expect("a readable body");
        Reply {
            status,
            content_type,
            location,
            body,
        }
    }
}

/// One response, as the tests inspect it.
struct Reply {
    status: u16,
    content_type: Option<String>,
    location: Option<String>,
    body: Vec<u8>,
}

impl Drop for Server {
    fn drop(&mut self) {
        // Already exited is fine; reaping is what matters, so no zombie outlives the test.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lines(stdout: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn fixture_site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/site")
}

/// Every file under `dir`, by relative path, with its bytes: two equal snapshots mean an
/// unchanged tree.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("a readable fixture directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(dir)
                    .expect("below the root")
                    .to_path_buf();
                out.insert(relative, fs::read(&path).expect("a readable fixture file"));
            }
        }
    }
    out
}

fn copy_of_fixture() -> TempDir {
    let temp = tempfile::tempdir().expect("a temporary directory");
    for (relative, bytes) in snapshot(&fixture_site()) {
        let target = temp.path().join(relative);
        fs::create_dir_all(target.parent().expect("a parent")).expect("a directory");
        fs::write(target, bytes).expect("a copied file");
    }
    temp
}

#[test]
fn should_serve_index_when_started_on_fixture() {
    // given
    let site = copy_of_fixture();
    let server = Server::start(site.path());

    // when
    let (status, body) = server.get("/");

    // then
    // The page is compared by its own content: the reload client is injected into served
    // HTML, so the response is not byte-equal to the file once injection exists.
    assert_eq!(status, 200);
    let page = String::from_utf8(body).expect("a UTF-8 page");
    assert!(page.contains("<h1>Route planner</h1>"), "{page}");
    assert!(
        server.url.starts_with("http://127.0.0.1:"),
        "not loopback: {}",
        server.url
    );
}

#[test]
fn should_serve_stylesheet_bytes_unchanged_when_requested() {
    // given
    let site = copy_of_fixture();
    let before = snapshot(site.path());
    let server = Server::start(site.path());

    // when
    let (status, body) = server.get("/styles.css");
    drop(server);

    // then
    assert_eq!(status, 200);
    assert_eq!(
        body,
        fs::read(fixture_site().join("styles.css")).expect("the fixture stylesheet")
    );
    assert_eq!(
        snapshot(site.path()),
        before,
        "serving changed the site tree"
    );
    assert_eq!(
        snapshot(&fixture_site()),
        before,
        "the shared fixture differs from its copy"
    );
}

#[test]
fn should_label_content_type_when_serving_fixture_files() {
    // given
    let site = copy_of_fixture();
    let server = Server::start(site.path());
    let expected = [
        ("/", "text/html; charset=utf-8"),
        ("/guide/", "text/html; charset=utf-8"),
        ("/styles.css", "text/css; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/data.json", "application/json"),
        ("/routes.gpx", "application/gpx+xml"),
        ("/images/sample.svg", "image/svg+xml"),
    ];

    // when
    let replies: Vec<_> = expected
        .iter()
        .map(|(path, _)| server.fetch(path))
        .collect();

    // then
    for ((path, content_type), reply) in expected.iter().zip(&replies) {
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(reply.content_type.as_deref(), Some(*content_type), "{path}");
    }
}

#[test]
fn should_redirect_to_slash_with_query_when_directory_requested() {
    // given
    let site = copy_of_fixture();
    let server = Server::start(site.path());

    // when
    let redirect = server.fetch("/guide?from=home");
    let target = server.fetch(redirect.location.as_deref().unwrap_or_default());

    // then
    assert_eq!(redirect.status, 307);
    assert_eq!(redirect.location.as_deref(), Some("/guide/?from=home"));
    assert_eq!(target.status, 200);
    // Compared by content, not bytes: served HTML gains the reload client once injection exists.
    let page = String::from_utf8(target.body).expect("a UTF-8 page");
    assert!(page.contains("<h1>Guide</h1>"), "{page}");
}

#[test]
fn should_refuse_dotfile_when_requested() {
    // given
    let site = copy_of_fixture();
    fs::write(site.path().join(".env"), "SYNTHETIC=1\n").expect("a dotfile");
    let server = Server::start(site.path());

    // when
    let (status, body) = server.get("/.env");

    // then
    assert_eq!(status, 403);
    assert!(
        !String::from_utf8_lossy(&body).contains("SYNTHETIC"),
        "the dotfile leaked"
    );
}

#[test]
fn should_exit_1_when_directory_does_not_exist() {
    // given
    let parent = tempfile::tempdir().expect("a temporary directory");
    let missing = parent.path().join("missing");
    let mut command = AssertCommand::new(env!("CARGO_BIN_EXE_site-preview"));

    // when
    let assert = command.arg("serve").arg(&missing).assert();

    // then
    assert
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(
            "site-preview: error: cannot serve ",
        ))
        .stderr(predicate::str::contains("does not exist"));
}
