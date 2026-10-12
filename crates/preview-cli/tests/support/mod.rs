//! The black-box harness shared by the `preview-cli` integration tests (ADR 0016).
//!
//! [`Site`] is a temporary copy of the shared fixture site, made hostile at copy time with
//! synthetic secrets the server must never serve. [`PreviewProcess`] runs the real
//! `site-preview` binary on it and talks to it over raw TCP, so every request target and
//! header goes on the wire exactly as the test wrote it: no client normalises `%2e%2e`,
//! drops a forged `Host` or follows a redirect. [`tree_hash`] fingerprints a directory tree
//! so a test can show that serving changed nothing.
//!
//! The process announces its URL on the human `Local:` line; CLI-06 switches discovery to the
//! `--json` `ready` event.

use std::collections::hash_map::DefaultHasher;
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use preview_testkit::poll::wait_until;
use tempfile::TempDir;

/// How long a test waits for the subprocess to print its URL, or for a reply, before it fails.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// Synthetic content of `.git/config` in every [`Site`].
pub const GIT_CONFIG_CANARY: &str = "synthetic-canary-git-config";
/// Synthetic content of `.env` in every [`Site`].
pub const ENV_CANARY: &str = "synthetic-canary-dotenv";
/// Synthetic content of `vendor/x.js` in every [`Site`].
pub const VENDOR_CANARY: &str = "synthetic-canary-vendor";
/// Synthetic content of `node_modules/x.js` in every [`Site`].
pub const NODE_MODULES_CANARY: &str = "synthetic-canary-node-modules";
/// Synthetic content of `secret.txt` beside, never inside, every [`Site`]'s root.
pub const OUTSIDE_CANARY: &str = "synthetic-canary-outside-root";

/// Every canary a [`Site`] plants; none may ever appear in a response.
pub const CANARIES: [&str; 5] = [
    GIT_CONFIG_CANARY,
    ENV_CANARY,
    VENDOR_CANARY,
    NODE_MODULES_CANARY,
    OUTSIDE_CANARY,
];

/// The committed fixture site, `tests/fixtures/site` (ADR 0016).
pub fn fixture_site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/site")
}

/// The `site-preview` binary as an `assert_cmd` command, for runs that exit on their own.
pub fn site_preview() -> assert_cmd::Command {
    assert_cmd::Command::new(env!("CARGO_BIN_EXE_site-preview"))
}

/// A temporary copy of the fixture site.
///
/// The temporary directory holds two siblings: `site/`, the root a test serves, and
/// `outside/`, which holds `secret.txt` ([`OUTSIDE_CANARY`]) for traversal and symlink
/// escapes to aim at. The copy adds `.git/config`, `.env`, `vendor/x.js` and
/// `node_modules/x.js`, each holding its canary; they are created here rather than
/// committed, so the repository's own git never sees a nested `.git`.
pub struct Site {
    temp: TempDir,
    root: PathBuf,
    pristine: u64,
}

impl Site {
    /// Copies the fixture and plants the synthetic secrets.
    pub fn copy() -> Self {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let root = temp.path().join("site");
        copy_tree(&fixture_site(), &root);
        let pristine = tree_hash(&root);
        let planted = [
            (".git/config", GIT_CONFIG_CANARY),
            (".env", ENV_CANARY),
            ("vendor/x.js", VENDOR_CANARY),
            ("node_modules/x.js", NODE_MODULES_CANARY),
        ];
        let outside = (temp.path().join("outside/secret.txt"), OUTSIDE_CANARY);
        let inside = planted.map(|(relative, canary)| (root.join(relative), canary));
        for (path, canary) in inside.into_iter().chain([outside]) {
            fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
            fs::write(path, format!("{canary}\n")).expect("a planted file");
        }
        Self {
            temp,
            root,
            pristine,
        }
    }

    /// The root to serve.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The root as a command-line argument.
    pub fn arg(&self) -> &str {
        self.root.to_str().expect("a UTF-8 temporary path")
    }

    /// The directory beside the root that holds `secret.txt`.
    pub fn outside(&self) -> PathBuf {
        self.temp.path().join("outside")
    }

    /// The [`tree_hash`] of the copy before the secrets were planted: equal to the committed
    /// fixture's own hash while the fixture is unmodified.
    pub fn pristine_hash(&self) -> u64 {
        self.pristine
    }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("a directory copy");
    for entry in fs::read_dir(source).expect("a readable fixture directory") {
        let entry = entry.expect("a readable entry");
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_tree(&path, &destination);
        } else {
            fs::copy(&path, &destination).expect("a file copy");
        }
    }
}

/// A fingerprint of everything under `dir`: each entry's relative path and kind, a file's
/// bytes and a symlink's target (never followed). Two equal hashes, taken in one test
/// process, mean an unchanged tree; the value is not stable across builds and is never
/// stored.
pub fn tree_hash(dir: &Path) -> u64 {
    let mut entries = Vec::new();
    collect(dir, dir, &mut entries);
    entries.sort();
    let mut hasher = DefaultHasher::new();
    entries.hash(&mut hasher);
    hasher.finish()
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u8, Vec<u8>)>) {
    for entry in fs::read_dir(dir).expect("a readable directory") {
        let path = entry.expect("a readable entry").path();
        let relative = path
            .strip_prefix(root)
            .expect("below the root")
            .to_path_buf();
        let kind = fs::symlink_metadata(&path)
            .expect("readable metadata")
            .file_type();
        if kind.is_symlink() {
            let target = fs::read_link(&path).expect("a readable link");
            out.push((relative, b'l', target.into_os_string().into_encoded_bytes()));
        } else if kind.is_dir() {
            out.push((relative, b'd', Vec::new()));
            collect(root, &path, out);
        } else {
            out.push((relative, b'f', fs::read(&path).expect("a readable file")));
        }
    }
}

/// A running `site-preview`, started with the given arguments.
///
/// Dropping it fails the test if the process had already exited (it must stay up until the
/// test ends, unless the test waited for its exit with [`Self::wait_exit`]), then kills and
/// reaps it, so no process outlives the test, also when an assertion failed.
pub struct PreviewProcess {
    child: Child,
    url: String,
    stderr: Receiver<String>,
    stopped: bool,
}

impl PreviewProcess {
    /// `site-preview serve <site>`.
    pub fn serve(site: &Site) -> Self {
        Self::start([OsStr::new("serve"), site.path().as_os_str()])
    }

    /// Spawns the binary with `args` and waits for its `Local:` line.
    pub fn start<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = Command::new(env!("CARGO_BIN_EXE_site-preview"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("site-preview starts");
        let stdout = lines(child.stdout.take().expect("stdout is piped"));
        let stderr = lines(child.stderr.take().expect("stderr is piped"));
        let mut process = Self {
            child,
            url: String::new(),
            stderr,
            stopped: false,
        };
        let mut seen = Vec::new();
        // Polled, not slept: the subprocess prints its URL once the listener is bound, and only
        // the OS scheduler decides when that is.
        let waited = wait_until(DEADLINE, "the `Local:` line on stdout", || {
            seen.extend(stdout.try_iter());
            seen.iter().any(|line| line.starts_with("Local:"))
        });
        if let Err(timeout) = waited {
            let errors: Vec<_> = process.stderr.try_iter().collect();
            panic!("{timeout}; stdout so far: {seen:?}; stderr: {errors:?}");
        }
        process.url = seen
            .iter()
            .find_map(|line| line.strip_prefix("Local:"))
            .map(|url| url.trim().to_owned())
            .expect("a Local: line");
        process
    }

    /// Stops the process and returns every line it wrote to stderr, in order. The reader
    /// thread ends when the pipe closes with the process, so nothing written before the stop
    /// can be missed (unlike a `try_recv` right after a request).
    pub fn stop_and_stderr(mut self) -> Vec<String> {
        self.stopped = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stderr.iter().collect()
    }

    /// The printed URL, e.g. `http://127.0.0.1:49152/`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The `host:port` of the printed URL, e.g. `127.0.0.1:49152`.
    pub fn authority(&self) -> &str {
        self.url.trim_start_matches("http://").trim_end_matches('/')
    }

    /// The port of the printed URL.
    pub fn port(&self) -> u16 {
        self.authority()
            .rsplit(':')
            .next()
            .and_then(|port| port.parse().ok())
            .expect("a port in the printed URL")
    }

    /// The lines the process wrote to stderr, as they arrive.
    pub fn stderr(&self) -> &Receiver<String> {
        &self.stderr
    }

    /// `GET target` with the printed authority as Host.
    pub fn get(&self, target: &str) -> Reply {
        self.request("GET", target, &[])
    }

    /// `HEAD target` with the printed authority as Host.
    pub fn head(&self, target: &str) -> Reply {
        self.request("HEAD", target, &[])
    }

    /// `method target` with the printed authority as Host plus `headers`. The target is sent
    /// verbatim: percent-encoding and dot segments reach the server as written.
    pub fn request(&self, method: &str, target: &str, headers: &[(&str, &str)]) -> Reply {
        let mut head = format!(
            "{method} {target} HTTP/1.1\r\nHost: {}\r\n",
            self.authority()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        self.raw(head.as_bytes())
    }

    /// Sends `head` (request line and headers, without the final blank line) verbatim over a
    /// fresh connection, adds `Connection: close`, and reads the reply to the end. Raw bytes,
    /// because an HTTP client sets Host from the URL and would not send a forged, duplicated
    /// or absolute-form one.
    pub fn raw(&self, head: &[u8]) -> Reply {
        let mut stream = TcpStream::connect(self.authority()).expect("the server accepts");
        stream
            .set_read_timeout(Some(DEADLINE))
            .expect("a read timeout");
        stream.write_all(head).expect("the request is sent");
        stream
            .write_all(b"Connection: close\r\n\r\n")
            .expect("the request is sent");
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).expect("a complete reply");
        Reply::parse(&reply)
    }
}

impl PreviewProcess {
    /// Sends the signal `name` (`INT`, `TERM`) to the process with the system `kill`, whose
    /// arguments are passed as they are, never through a shell.
    pub fn signal(&self, name: &str) {
        let status = Command::new("kill")
            .args(["-s", name, &self.child.id().to_string()])
            .status()
            .expect("kill runs");
        assert!(status.success(), "kill -s {name} failed: {status}");
    }

    /// Waits up to `within` for the process to exit on its own and returns its status; fails
    /// the test if it is still running then.
    pub fn wait_exit(mut self, within: Duration) -> ExitStatus {
        let mut status = None;
        // Polled: only the OS scheduler decides when the process has finished its shutdown.
        let waited = wait_until(within, "site-preview to exit", || {
            status = self.child.try_wait().expect("the child can be polled");
            status.is_some()
        });
        if let Err(timeout) = waited {
            let errors: Vec<_> = self.stderr.try_iter().collect();
            panic!("{timeout}; stderr: {errors:?}");
        }
        // Reaped: nothing is left for `Drop` to kill, and this exit was expected.
        self.stopped = true;
        status.expect("an exit status")
    }
}

impl Drop for PreviewProcess {
    fn drop(&mut self) {
        if self.stopped {
            return;
        }
        let early = self.child.try_wait();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Ok(Some(status)) = early
            && !thread::panicking()
        {
            let errors: Vec<_> = self.stderr.try_iter().collect();
            panic!("site-preview exited before the test ended: {status}; stderr: {errors:?}");
        }
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

/// One HTTP/1.1 response, as read off the wire.
#[derive(Debug)]
pub struct Reply {
    /// The status code.
    pub status: u16,
    /// Every header, name lowercased, in wire order.
    pub headers: Vec<(String, String)>,
    /// Every byte after the header block, de-chunked if the reply was chunked. For HEAD it
    /// holds whatever the server wrongly sent after the headers, so it must be empty.
    pub body: Vec<u8>,
}

impl Reply {
    fn parse(bytes: &[u8]) -> Self {
        let split = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a complete header block");
        let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
        let rest = &bytes[split + 4..];
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .expect("a status line");
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let chunked = headers
            .iter()
            .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"));
        let body = if chunked {
            dechunk(rest)
        } else {
            rest.to_vec()
        };
        Self {
            status,
            headers,
            body,
        }
    }

    /// The value of the first header named `name` (any case).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    /// `Content-Type`.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// `Location`.
    pub fn location(&self) -> Option<&str> {
        self.header("location")
    }

    /// The body as text, lossily.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Panics if the body holds any [`CANARIES`] entry.
    pub fn assert_leaks_nothing(&self, case: &str) {
        let text = self.text();
        for canary in CANARIES {
            assert!(!text.contains(canary), "{case}: leaked {canary}: {text}");
        }
    }
}

fn dechunk(mut rest: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("a chunk size line");
        let size_line = String::from_utf8_lossy(&rest[..end]);
        let size_text = size_line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).expect("a hexadecimal chunk size");
        rest = &rest[end + 2..];
        if size == 0 {
            return body;
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}
