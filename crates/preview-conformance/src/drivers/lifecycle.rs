//! The lifecycle driver (FORMAL-05, ADR 0056): replays `formal/lifecycle.qnt`'s `serverStep`
//! actions against the real composition [`Server`] over a site directory, port 0, the native
//! watcher and real loopback sockets.
//!
//! | Model action | Stimulus |
//! |:--|:--|
//! | `init` | drop every server, stream and socket; nothing has started |
//! | `startServer(bindOk, watcherOk)` | [`Server::start`]; `bindOk` false binds a port this driver already holds, `watcherOk` false is [`Server::start_with_unavailable_watcher`] |
//! | `refuseStart` | [`Server::start`] on a root that does not exist |
//! | `stopServer` | [`Server::stop`] on the last server, if one started; then each held event stream is read to its end |
//! | `request` | `GET /` over a new connection |
//! | `openStream` | `GET /__site_preview/events`, held open |
//!
//! `startServerBound` (`serverStepMostlyBound`'s sampling of `startServer`) is the same
//! stimulus. A restart builds a new [`Server`]: a stopped one has given up its runtime. The
//! server is started through a [`StartServer`] function, the real one unless a negative control
//! wraps it ([`LifecycleDriver::with_server`], FORMAL-10).
//!
//! The observation ([`LifecycleObs`]) is read from the real code after every step:
//!
//! - `state`: [`Server::state`]. A start that returns an error leaves no server to ask; the
//!   error's variant is the observation instead: [`StartupError::Bind`] is the machine's
//!   `BindFailed` (it ends `Failed`), and a refusal before `Start` (a missing root) leaves it
//!   `Stopped`. Any other error is a [`DriverError::Refused`].
//! - `listener_owned`: whether the last server's address still accepts a TCP connection.
//!   Only a bound listener completes a handshake on loopback; a closed one is refused.
//! - `tasks_alive`: [`Server::owns_tasks`], a task spawned and not yet joined.
//! - `streams_open`: how many event streams this driver opened that have not ended: after a
//!   stop, each held socket is read until the server closes it (end of file or a reset), so
//!   a stream the shutdown left open is counted.
//! - `stopped`: what [`Server::stop`] returned at a `stopServer` step, `None` at any other.
//!   The model has no drain: every stop it takes ends its streams at `CloseStreams`, before
//!   `JoinTasks`, so it is [`Stopped::Drained`]; a stop of no server drains nothing and is
//!   `Drained` too. A `DrainTimedOut` is a connection the shutdown did not end in time, which
//!   the model rules out.
//! - `response`: what the last client action saw: a `200` head is `Answered`; a refused,
//!   reset or other answer is `Unanswered`; a non-client action is `NoRequest`.
//!
//! The model's `res.listeners == 1`, `res.serveTasks + res.debounceTasks > 0`, `res.streams`,
//! `state`, `response` and the step's action project to the same type ([`project`]). The
//! watcher's own resources are below this observation (the model's header).

use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use preview_composition::{
    Ready, ServeConfig, Server, StartupError, StartupReporter, Stopped, WarningSink,
};
use preview_domain::{InFlight, LifecycleState, StartPhase};

use crate::itf::{State, Value};
use crate::replay::{Driver, DriverError, Params, ProjectionError};

/// How long a client waits to connect, for a response head that should come at once, or for
/// a stopped server to close a stream; only a bound on a failing run, never a timing oracle.
const REPLY_DEADLINE: Duration = Duration::from_secs(20);

/// What a client saw for the last action (the model's `Response`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    /// The last action was not a client's.
    NoRequest,
    /// The server answered `200`.
    Answered,
    /// Nothing answered, or the answer was a refusal (`503` for a stream over the cap).
    Unanswered,
}

/// The canonical lifecycle observation: what both the model and the real server project to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleObs {
    /// The lifecycle machine's state.
    pub state: LifecycleState,
    /// A bound listener is owned (the model's `res.listeners == 1`).
    pub listener_owned: bool,
    /// A serve or pipeline task is owned and not joined (`serveTasks + debounceTasks > 0`).
    pub tasks_alive: bool,
    /// Event streams opened and not ended (`res.streams`).
    pub streams_open: usize,
    /// What the stop at this step returned; `None` when the step was not a stop.
    pub stopped: Option<Stopped>,
    /// What the last client action saw.
    pub response: Response,
}

/// What the driver needs of a started server: [`Server`]'s queries and its stop. The seam a
/// negative control wraps to put a fault behind the real server (FORMAL-10).
pub trait ServerHandle {
    /// [`Server::state`].
    fn state(&self) -> LifecycleState;
    /// [`Server::owns_tasks`].
    fn owns_tasks(&self) -> bool;
    /// [`Server::local_addr`].
    fn local_addr(&self) -> SocketAddr;
    /// [`Server::stop`].
    fn stop(&mut self) -> Stopped;
}

impl ServerHandle for Server {
    fn state(&self) -> LifecycleState {
        Self::state(self)
    }

    fn owns_tasks(&self) -> bool {
        Self::owns_tasks(self)
    }

    fn local_addr(&self) -> SocketAddr {
        Self::local_addr(self)
    }

    fn stop(&mut self) -> Stopped {
        Self::stop(self)
    }
}

/// Starts a server for `config`: [`Server::start`] when `watcher_ok`, else
/// [`Server::start_with_unavailable_watcher`] ([`start_server`]), or a negative control's
/// wrapper around them.
pub type StartServer =
    fn(config: &ServeConfig, watcher_ok: bool) -> Result<Box<dyn ServerHandle>, StartupError>;

/// The real start: [`Server::start`], or [`Server::start_with_unavailable_watcher`].
///
/// # Errors
///
/// As [`Server::start`].
pub fn start_server(
    config: &ServeConfig,
    watcher_ok: bool,
) -> Result<Box<dyn ServerHandle>, StartupError> {
    let server = if watcher_ok {
        Server::start(config, Box::new(Quiet), Arc::new(Quiet))?
    } else {
        Server::start_with_unavailable_watcher(config, Box::new(Quiet), Arc::new(Quiet))?
    };
    Ok(Box::new(server))
}

/// The server the driver started last, or what stood in for it.
enum Current {
    /// Nothing started, or the last start was refused before `Start`.
    Absent,
    /// A server that started; it may have been stopped since.
    Started(Box<dyn ServerHandle>),
    /// The last start failed to bind.
    BindFailed,
}

/// Drives the real [`Server`] from the lifecycle model's `serverStep` actions.
pub struct LifecycleDriver {
    /// The site served; it holds an `index.html`.
    root: PathBuf,
    start_server: StartServer,
    current: Current,
    response: Response,
    stopped: Option<Stopped>,
    /// The event streams opened and not yet seen to end.
    streams: Vec<TcpStream>,
}

impl LifecycleDriver {
    /// A driver serving `root`, a directory with an `index.html`. Nothing is started until a
    /// trace's `init` and `startServer`.
    pub fn new(root: &Path) -> Self {
        Self::with_server(root, start_server)
    }

    /// A driver whose servers `start` starts.
    pub fn with_server(root: &Path, start: StartServer) -> Self {
        Self {
            root: root.to_path_buf(),
            start_server: start,
            current: Current::Absent,
            response: Response::NoRequest,
            stopped: None,
            streams: Vec::new(),
        }
    }

    /// Stops and forgets every server and stream: the model's `init`.
    fn reset(&mut self) {
        self.streams.clear();
        // Dropping a running server stops it.
        self.current = Current::Absent;
    }

    fn start(&mut self, bind_ok: bool, watcher_ok: bool) -> Result<(), DriverError> {
        // The model starts only from `Stopped`; a new start is a new `Server`.
        self.reset();
        let mut config = ServeConfig::new(self.root.clone());
        // A bind that must fail: the port is held, as by another program, for the whole start.
        let occupied = if bind_ok {
            None
        } else {
            let blocker = TcpListener::bind((config.host, 0))
                .map_err(|e| DriverError::Refused(format!("cannot occupy a port: {e}")))?;
            config.port = blocker
                .local_addr()
                .map_err(|e| DriverError::Refused(format!("cannot occupy a port: {e}")))?
                .port();
            Some(blocker)
        };
        let started = (self.start_server)(&config, watcher_ok);
        drop(occupied);
        self.current = match started {
            Ok(server) => Current::Started(server),
            Err(StartupError::Bind { .. }) => Current::BindFailed,
            Err(other) => return Err(DriverError::Refused(other.to_string())),
        };
        Ok(())
    }

    fn refuse_start(&mut self) -> Result<(), DriverError> {
        self.reset();
        let config = ServeConfig::new(self.root.join("missing-root"));
        self.current = match (self.start_server)(&config, true) {
            Ok(server) => Current::Started(server),
            Err(StartupError::InvalidRoot(_)) => Current::Absent,
            Err(other) => return Err(DriverError::Refused(other.to_string())),
        };
        Ok(())
    }

    /// Stops the last server, keeps what the stop returned, then reads each held stream until
    /// the server closes it; a stream still open after [`REPLY_DEADLINE`] stays counted.
    fn stop(&mut self) {
        self.stopped = Some(match &mut self.current {
            Current::Started(server) => server.stop(),
            Current::Absent | Current::BindFailed => Stopped::Drained,
        });
        self.streams.retain_mut(|stream| !ended(stream));
    }

    /// The address of the last server that started, stopped or not: where `listener_owned`
    /// is probed. A negative control (FORMAL-08) reads it to leak a listener there.
    pub fn address(&self) -> Option<SocketAddr> {
        match &self.current {
            Current::Started(server) => Some(server.local_addr()),
            Current::Absent | Current::BindFailed => None,
        }
    }

    fn request(&self) -> Response {
        let Some(addr) = self.address() else {
            return Response::Unanswered;
        };
        let head = format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        match send(addr, &head) {
            Ok((true, _)) => Response::Answered,
            Ok((false, _)) | Err(_) => Response::Unanswered,
        }
    }

    fn open_stream(&mut self) -> Response {
        let Some(addr) = self.address() else {
            return Response::Unanswered;
        };
        let head = format!("GET /__site_preview/events HTTP/1.1\r\nHost: {addr}\r\n\r\n");
        match send(addr, &head) {
            Ok((true, stream)) => {
                self.streams.push(stream);
                Response::Answered
            }
            Ok((false, _)) | Err(_) => Response::Unanswered,
        }
    }
}

impl Driver for LifecycleDriver {
    type Obs = LifecycleObs;

    fn apply(&mut self, action: &str, params: &Params) -> Result<(), DriverError> {
        self.response = Response::NoRequest;
        self.stopped = None;
        match action {
            "init" => self.reset(),
            "startServer" | "startServerBound" => {
                self.start(params.bool("bindOk")?, params.bool("watcherOk")?)?;
            }
            "refuseStart" => self.refuse_start()?,
            "stopServer" => self.stop(),
            "request" => self.response = self.request(),
            "openStream" => self.response = self.open_stream(),
            other => return Err(DriverError::UnknownAction(other.to_owned())),
        }
        Ok(())
    }

    fn observe(&self) -> LifecycleObs {
        let (state, tasks_alive) = match &self.current {
            Current::Absent => (LifecycleState::Stopped, false),
            Current::Started(server) => (server.state(), server.owns_tasks()),
            Current::BindFailed => (LifecycleState::Failed, false),
        };
        LifecycleObs {
            state,
            listener_owned: self.address().is_some_and(accepts),
            tasks_alive,
            streams_open: self.streams.len(),
            stopped: self.stopped,
            response: self.response,
        }
    }
}

/// Whether a listener at `addr` completes a handshake within [`REPLY_DEADLINE`].
fn accepts(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, REPLY_DEADLINE).is_ok()
}

/// Reads `stream` until the server ends it: end of file or a reset is an end; a stream not
/// ended within [`REPLY_DEADLINE`] (keepalive comments do not extend it) is still open.
fn ended(stream: &mut TcpStream) -> bool {
    let deadline = Instant::now() + REPLY_DEADLINE;
    let mut buffer = [0_u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return false;
        }
        match stream.read(&mut buffer) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return !matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
        }
    }
}

/// Sends `head` on a new connection to `addr` and reads the response head: whether it is a
/// `200`, and the connection (an event stream stays open on it).
fn send(addr: SocketAddr, head: &str) -> io::Result<(bool, TcpStream)> {
    let mut stream = TcpStream::connect_timeout(&addr, REPLY_DEADLINE)?;
    stream.set_read_timeout(Some(REPLY_DEADLINE))?;
    stream.write_all(head.as_bytes())?;
    let mut seen = Vec::new();
    let mut byte = [0_u8; 1];
    while !seen.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            break;
        }
        seen.push(byte[0]);
    }
    Ok((seen.starts_with(b"HTTP/1.1 200 "), stream))
}

/// Projects a state of `formal/lifecycle.qnt` (or of a module importing it, such as
/// `lifecycle_traces.qnt`) into the driver's observation.
///
/// # Errors
///
/// [`ProjectionError`] when a variable is absent or has a shape the model does not give it.
pub fn project(state: &State) -> Result<LifecycleObs, ProjectionError> {
    let var = |name: &str| {
        state
            .var(name)
            .ok_or_else(|| ProjectionError::missing(name))
    };
    let count = |field: &str| {
        var("res")?
            .field(field)
            .and_then(Value::as_int)
            .ok_or_else(|| ProjectionError::missing(&format!("res.{field}")))
    };
    Ok(LifecycleObs {
        state: lifecycle_state(var("state")?)?,
        listener_owned: count("listeners")? == 1,
        tasks_alive: count("serveTasks")? + count("debounceTasks")? > 0,
        streams_open: usize::try_from(count("streams")?)
            .map_err(|_| ProjectionError::missing("res.streams"))?,
        // The model has no drain: each of its stops is a drained one (the module's header).
        stopped: (state.action_taken() == Some("stopServer")).then_some(Stopped::Drained),
        response: match var("response")?.tag() {
            Some("NoRequest") => Response::NoRequest,
            Some("Answered") => Response::Answered,
            Some("Unanswered") => Response::Unanswered,
            _ => return Err(ProjectionError::missing("response")),
        },
    })
}

/// The model's `LifecycleState` as the domain's.
fn lifecycle_state(value: &Value) -> Result<LifecycleState, ProjectionError> {
    let unknown = || ProjectionError(format!("state {value} is not a lifecycle state"));
    let (tag, payload) = value.as_variant().ok_or_else(unknown)?;
    let state = match (tag, payload.tag()) {
        ("Stopped", _) => LifecycleState::Stopped,
        ("Failed", _) => LifecycleState::Failed,
        ("Starting", Some("Binding")) => LifecycleState::Starting(StartPhase::Binding),
        ("Starting", Some("StartingWatcher")) => {
            LifecycleState::Starting(StartPhase::StartingWatcher)
        }
        ("Serving", None) => LifecycleState::Serving {
            watcher: payload.as_bool().ok_or_else(unknown)?,
        },
        ("Stopping", Some("NoStep")) => LifecycleState::Stopping { awaiting: None },
        ("Stopping", Some("AwaitBind")) => LifecycleState::Stopping {
            awaiting: Some(InFlight::Bind),
        },
        ("Stopping", Some("AwaitWatcher")) => LifecycleState::Stopping {
            awaiting: Some(InFlight::Watcher),
        },
        _ => return Err(unknown()),
    };
    Ok(state)
}

/// Reports nothing: the driver observes the server, not its messages.
struct Quiet;

impl StartupReporter for Quiet {
    fn ready(&mut self, _ready: &Ready<'_>) {}

    fn draining(&mut self, _bound: Duration) {}
}

impl WarningSink for Quiet {
    fn warn(&self, _code: &str, _message: &str) {}
}
