//! A `site-preview` child process and a raw-TCP server-sent-events client.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long to wait for the server to print `ready`, or for an expected event.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// A running server, killed and reaped by PID when dropped, also when a benchmark failed.
pub struct Server {
    child: Child,
    authority: String,
    startup: Duration,
}

impl Server {
    /// Spawns `binary serve <root> --json` and waits for the `ready` event; the elapsed time
    /// from spawn to that line is [`Self::startup`].
    pub fn start(binary: &Path, root: &Path) -> Result<Self, String> {
        let begin = Instant::now();
        let mut child = Command::new(binary)
            .arg("serve")
            .arg(root)
            .arg("--json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
        let stdout = child.stdout.take().ok_or("stdout is not piped")?;
        let (sender, lines) = mpsc::channel();
        // The thread keeps draining stdout, so later watcher events never fill the pipe; it
        // ends when the child dies.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            authority: String::new(),
            startup: Duration::ZERO,
        };
        let line = lines
            .recv_timeout(DEADLINE)
            .map_err(|e| format!("no `ready` line within {DEADLINE:?}: {e}"))?;
        server.startup = begin.elapsed();
        let url = parse_ready_url(&line).ok_or_else(|| format!("not a `ready` event: {line}"))?;
        server.authority = authority_of(&url).ok_or_else(|| format!("no authority in {url}"))?;
        Ok(server)
    }

    /// The process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// `host:port` of the listener.
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// Time from spawn to the `ready` line.
    pub fn startup(&self) -> Duration {
        self.startup
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `url` of a `{"event":"ready","url":"..."}` line.
pub fn parse_ready_url(line: &str) -> Option<String> {
    if !line.contains("\"event\":\"ready\"") {
        return None;
    }
    let rest = &line[line.find("\"url\":\"")? + "\"url\":\"".len()..];
    Some(rest[..rest.find('"')?].to_owned())
}

/// `host:port` of an `http://host:port/` URL.
pub fn authority_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    (!authority.is_empty()).then(|| authority.to_owned())
}

/// The outcome of asking for the event stream.
pub enum Opened {
    /// A 200 answer: the stream is open.
    Stream(Box<SseClient>),
    /// Any other status, for example the 503 of a full server.
    Refused(u16),
}

/// One `/__site_preview/events` connection.
pub struct SseClient {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl SseClient {
    /// Connects and reads the response head.
    pub fn open(authority: &str) -> Result<Opened, String> {
        let mut stream = TcpStream::connect(authority).map_err(|e| format!("connect: {e}"))?;
        let request = format!(
            "GET /__site_preview/events HTTP/1.1\r\nHost: {authority}\r\n\
             Accept: text/event-stream\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("send: {e}"))?;
        let mut client = Self {
            stream,
            buffer: Vec::new(),
        };
        let deadline = Instant::now() + DEADLINE;
        while find(&client.buffer, b"\r\n\r\n").is_none() {
            client.fill(deadline)?;
        }
        let head_end = find(&client.buffer, b"\r\n\r\n").unwrap_or(0) + 4;
        let head = String::from_utf8_lossy(&client.buffer[..head_end]).into_owned();
        client.buffer.drain(..head_end);
        let status = parse_status(&head).ok_or_else(|| format!("bad response head: {head}"))?;
        Ok(if status == 200 {
            Opened::Stream(Box::new(client))
        } else {
            Opened::Refused(status)
        })
    }

    /// Blocks until an event named `name` has arrived (consuming it) or `DEADLINE` passes.
    pub fn wait_for(&mut self, name: &str) -> Result<(), String> {
        self.wait_for_within(name, DEADLINE)
    }

    /// Like [`Self::wait_for`] with a caller-chosen limit.
    pub fn wait_for_within(&mut self, name: &str, limit: Duration) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        loop {
            if take_event(&mut self.buffer, name) {
                return Ok(());
            }
            self.fill(deadline)
                .map_err(|e| format!("waiting for `{name}`: {e}"))?;
        }
    }

    fn fill(&mut self, deadline: Instant) -> Result<(), String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or("timed out")?;
        self.stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| format!("timeout: {e}"))?;
        let mut chunk = [0_u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err("the stream ended".to_owned()),
            Ok(n) => {
                self.buffer.extend_from_slice(&chunk[..n]);
                Ok(())
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Err("timed out".to_owned())
            }
            Err(e) => Err(format!("read: {e}")),
        }
    }
}

/// Removes everything up to and including the first `event: <name>` line of `buffer` and
/// returns true; leaves `buffer` alone and returns false when there is none (yet).
pub fn take_event(buffer: &mut Vec<u8>, name: &str) -> bool {
    let needle = format!("event: {name}\n");
    match find(buffer, needle.as_bytes()) {
        Some(at) => {
            buffer.drain(..at + needle.len());
            true
        }
        None => false,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_status(head: &str) -> Option<u16> {
    head.lines().next()?.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_read_url_when_line_is_the_ready_event() {
        // given
        let line = r#"{"event":"ready","url":"http://127.0.0.1:49152/","pid":1,"watching":true}"#;

        // when
        let url = parse_ready_url(line);

        // then
        assert_eq!(url.as_deref(), Some("http://127.0.0.1:49152/"));
    }

    #[test]
    fn should_return_none_when_line_is_another_event() {
        // given
        let line = r#"{"event":"watcher","state":"watching"}"#;

        // when
        let url = parse_ready_url(line);

        // then
        assert_eq!(url, None);
    }

    #[test]
    fn should_strip_scheme_and_path_when_authority_is_taken() {
        // given
        let url = "http://127.0.0.1:8080/";

        // when
        let authority = authority_of(url);

        // then
        assert_eq!(authority.as_deref(), Some("127.0.0.1:8080"));
        assert_eq!(authority_of("ftp://x"), None);
    }

    #[test]
    fn should_consume_through_event_when_it_is_in_the_buffer() {
        // given
        let mut buffer =
            b"3a\r\nevent: state\nid: 1\ndata: {}\n\n\r\nevent: reload\nid: 2\n".to_vec();

        // when
        let state = take_event(&mut buffer, "state");
        let reload = take_event(&mut buffer, "reload");
        let again = take_event(&mut buffer, "reload");

        // then
        assert!(state);
        assert!(reload);
        assert!(!again);
        assert_eq!(buffer, b"id: 2\n");
    }

    #[test]
    fn should_keep_buffer_when_event_has_not_arrived() {
        // given
        let mut buffer = b"event: sta".to_vec();

        // when
        let found = take_event(&mut buffer, "state");

        // then
        assert!(!found);
        assert_eq!(buffer, b"event: sta");
    }

    #[test]
    fn should_read_status_when_head_is_parsed() {
        // given
        let head = "HTTP/1.1 503 Service Unavailable\r\nretry-after: 2\r\n\r\n";

        // when
        let status = parse_status(head);

        // then
        assert_eq!(status, Some(503));
    }
}
