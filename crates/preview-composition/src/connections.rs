//! The accept loop: per-connection timeouts, the connection cap and the bounded shutdown
//! drain (plan.md section 9, ADR 0045).
//!
//! `axum::serve` was replaced because it sets no header-read or idle timeout, caps nothing and
//! waits for every connection without bound on shutdown: one half-sent request, or a client
//! that stops reading a large file, kept the process alive after Ctrl-C. Here each connection
//! is a hyper HTTP/1 connection with a header-read timer, holds one permit of a fixed pool,
//! and a shutdown waits at most [`ServerLimits::drain_timeout`] before it drops the rest.

use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::config::ServerLimits;

/// How long the accept loop pauses after a failed `accept`, so a process out of descriptors
/// (`EMFILE`) does not spin; the queued connection is retried after the pause.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// A request to stop, as the operating system delivered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSignal {
    /// SIGINT (Ctrl-C).
    Interrupt,
    /// SIGTERM.
    Terminate,
}

/// How a `serve` run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Every open connection finished within the drain bound.
    Drained,
    /// The drain bound passed; the connections still open were dropped. Still a clean stop:
    /// the server did what it was asked.
    DrainTimedOut,
    /// A second stop signal arrived during the drain: the remaining connections were dropped
    /// at once. The CLI exits with 128 plus the signal number, as a shell does.
    Forced(StopSignal),
}

/// A connection the loop serves: the router behind hyper's HTTP/1 state machine.
type Connection = http1::Connection<TokioIo<TcpStream>, TowerToHyperService<Router>>;

/// Serves `router` on `listener` until the first message on `stops`, then drains.
///
/// On the first stop the listener is closed (a new connection is refused; one still in the
/// kernel's accept queue is reset, never served), every open connection is asked to finish its
/// response and close, and `on_stop` is called with the number still open (composition ends
/// the event streams there). `on_waiting` is called once if a connection is still open when
/// `grace` has passed since then: the drain itself decides, on its own clock, whether it is
/// really waiting, so how late the caller looks never changes the answer. The run then ends
/// at the first of: every connection closed ([`Stopped::Drained`]), the drain bound
/// ([`Stopped::DrainTimedOut`]) or a second stop ([`Stopped::Forced`]). Pinned by
/// `should_report_waiting_only_when_a_connection_outlasts_the_grace`,
/// `should_drop_stalled_connection_when_drain_bound_passes`,
/// `should_stop_at_once_when_second_stop_arrives_during_drain`,
/// `should_close_connection_when_head_not_sent_within_timeout`,
/// `should_close_idle_keep_alive_connection_when_timeout_passes` and
/// `should_close_excess_connection_when_cap_reached`.
pub(crate) async fn serve_connections(
    listener: TcpListener,
    router: Router,
    limits: ServerLimits,
    stops: &mut mpsc::UnboundedReceiver<StopSignal>,
    on_stop: impl FnOnce(usize),
    grace: Duration,
    on_waiting: impl FnOnce(),
) -> Stopped {
    let mut http = http1::Builder::new();
    // hyper starts the header timer whenever it waits for a request head, so this also
    // closes an idle keep-alive connection; it never applies while a response streams.
    http.timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout);
    let permits = Arc::new(Semaphore::new(limits.max_connections));
    let (closing, closed) = watch::channel(false);
    let mut open = JoinSet::new();
    loop {
        match next_event(&listener, stops, &mut open).await {
            Event::Stop => break,
            Event::Accepted(stream) => {
                // At the cap the connection is closed at once rather than left queued: the
                // client learns now instead of hanging, and the descriptor is returned.
                if let Ok(permit) = Arc::clone(&permits).try_acquire_owned() {
                    let service = TowerToHyperService::new(router.clone());
                    let connection = http.serve_connection(TokioIo::new(stream), service);
                    open.spawn(serve_one(connection, closed.clone(), permit));
                }
            }
            Event::AcceptFailed => {
                if backoff(stops).await {
                    break;
                }
            }
        }
    }
    drop(listener);
    closing.send_replace(true);
    on_stop(open.len());
    drain(open, stops, limits.drain_timeout, grace, on_waiting).await
}

enum Event {
    Stop,
    Accepted(TcpStream),
    AcceptFailed,
}

/// The next thing the loop reacts to; finished connection tasks are reaped on the way, so
/// the set never grows with closed connections.
async fn next_event(
    listener: &TcpListener,
    stops: &mut mpsc::UnboundedReceiver<StopSignal>,
    open: &mut JoinSet<()>,
) -> Event {
    poll_fn(|cx| {
        // A closed channel (its sender gone) also stops: nothing could stop the loop later.
        if stops.poll_recv(cx).is_ready() {
            return Poll::Ready(Event::Stop);
        }
        while let Poll::Ready(Some(_)) = open.poll_join_next(cx) {}
        listener.poll_accept(cx).map(|accepted| match accepted {
            Ok((stream, _)) => Event::Accepted(stream),
            Err(_) => Event::AcceptFailed,
        })
    })
    .await
}

/// Waits [`ACCEPT_BACKOFF`]; true if a stop arrived meanwhile.
async fn backoff(stops: &mut mpsc::UnboundedReceiver<StopSignal>) -> bool {
    let mut pause = pin!(sleep(ACCEPT_BACKOFF));
    poll_fn(|cx| {
        if stops.poll_recv(cx).is_ready() {
            return Poll::Ready(true);
        }
        pause.as_mut().poll(cx).map(|()| false)
    })
    .await
}

/// Serves one connection; once `closing` flips, asks it to finish the response in flight and
/// close. The permit is held until the connection is gone.
async fn serve_one(
    connection: Connection,
    mut closing: watch::Receiver<bool>,
    _permit: OwnedSemaphorePermit,
) {
    let mut connection = pin!(connection);
    let stopping = async move {
        loop {
            let stop = *closing.borrow_and_update();
            if stop || closing.changed().await.is_err() {
                return;
            }
        }
    };
    let mut stopping = pin!(stopping);
    // A failed connection (a reset, a malformed request, the header timeout) is the client's
    // business: hyper has already answered or closed it.
    let finished = poll_fn(|cx| {
        if connection.as_mut().poll(cx).is_ready() {
            return Poll::Ready(true);
        }
        stopping.as_mut().poll(cx).map(|()| false)
    })
    .await;
    if !finished {
        connection.as_mut().graceful_shutdown();
        let _closed = connection.await;
    }
}

enum Drain {
    Done,
    TimedOut,
    Forced(StopSignal),
}

/// Waits for the open connections, at most `bound`, and gives up at once on a second stop;
/// calls `on_waiting` once if a connection is still open when `grace` has passed.
async fn drain(
    mut open: JoinSet<()>,
    stops: &mut mpsc::UnboundedReceiver<StopSignal>,
    bound: Duration,
    grace: Duration,
    on_waiting: impl FnOnce(),
) -> Stopped {
    let mut deadline = pin!(sleep(bound));
    let mut grace = pin!(sleep(grace));
    let mut on_waiting = Some(on_waiting);
    let outcome = poll_fn(|cx| {
        // A closed channel cannot deliver a second stop; it is not one.
        if let Poll::Ready(Some(signal)) = stops.poll_recv(cx) {
            return Poll::Ready(Drain::Forced(signal));
        }
        loop {
            match open.poll_join_next(cx) {
                Poll::Ready(Some(_)) => {}
                Poll::Ready(None) => return Poll::Ready(Drain::Done),
                Poll::Pending => break,
            }
        }
        // Reached only while a connection is still open.
        if on_waiting.is_some()
            && grace.as_mut().poll(cx).is_ready()
            && let Some(waiting) = on_waiting.take()
        {
            waiting();
        }
        deadline.as_mut().poll(cx).map(|()| Drain::TimedOut)
    })
    .await;
    // Dropping the set aborts what is left; each aborted task drops its socket.
    open.abort_all();
    match outcome {
        Drain::Done => Stopped::Drained,
        Drain::TimedOut => Stopped::DrainTimedOut,
        Drain::Forced(signal) => Stopped::Forced(signal),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Read, Write};
    use std::net::{SocketAddr, TcpStream as StdStream};
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::routing::get;
    use preview_testkit::poll::wait_until;
    use tokio::runtime::Runtime;
    use tokio::task::JoinHandle;

    use super::*;

    /// How long a test waits for something that should happen well within it.
    const DEADLINE: Duration = Duration::from_secs(20);
    /// A bound a test never reaches when the behaviour under test works.
    const NEVER: Duration = Duration::from_secs(3_600);
    /// A response far larger than the loopback socket buffers, so a client that does not
    /// read it leaves the server blocked mid-write.
    const BIG: usize = 64 << 20;
    /// How long the drain runs before it reports that it is waiting.
    const GRACE: Duration = Duration::from_millis(100);

    struct Server {
        runtime: Runtime,
        addr: SocketAddr,
        stops: mpsc::UnboundedSender<StopSignal>,
        run: JoinHandle<Stopped>,
        /// Whether the drain reported that it was waiting.
        waiting: Arc<AtomicBool>,
    }

    fn start(limits: ServerLimits) -> Server {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let listener = runtime
            .block_on(TcpListener::bind("127.0.0.1:0"))
            .expect("a listener");
        let addr = listener.local_addr().expect("a bound address");
        let (stops, mut received) = mpsc::unbounded_channel();
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/big", get(|| async { vec![b'x'; BIG] }));
        let waiting = Arc::new(AtomicBool::new(false));
        let waited = Arc::clone(&waiting);
        let run = runtime.spawn(async move {
            let report = move || waited.store(true, Ordering::SeqCst);
            serve_connections(
                listener,
                router,
                limits,
                &mut received,
                |_| {},
                GRACE,
                report,
            )
            .await
        });
        Server {
            runtime,
            addr,
            stops,
            run,
            waiting,
        }
    }

    impl Server {
        fn connect(&self) -> StdStream {
            let stream = StdStream::connect(self.addr).expect("the server accepts");
            stream
                .set_read_timeout(Some(DEADLINE))
                .expect("a read timeout");
            stream
        }

        fn stop(&self, signal: StopSignal) {
            self.stops.send(signal).expect("the loop is running");
        }

        /// A connection that asked for the big response, read its first bytes (so the
        /// server is writing it) and then stopped reading.
        fn stalled_reader(&self) -> StdStream {
            let mut stream = self.connect();
            stream
                .write_all(b"GET /big HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("the request is sent");
            let mut start = [0_u8; 12];
            stream.read_exact(&mut start).expect("the reply starts");
            assert_eq!(&start, b"HTTP/1.1 200");
            stream
        }

        fn finish(self) -> Stopped {
            let Self { runtime, run, .. } = self;
            // Polled: the run ends on the runtime's threads; the deadline only bounds a hang.
            wait_until(DEADLINE, "the serve loop to end", || run.is_finished())
                .expect("the run ends within the deadline");
            runtime.block_on(run).expect("the run does not panic")
        }
    }

    fn limits(header_read_timeout: Duration, max_connections: usize) -> ServerLimits {
        ServerLimits {
            header_read_timeout,
            max_connections,
            drain_timeout: NEVER,
        }
    }

    /// True once the server closed the connection: end of stream or a reset, never the
    /// client's own read timeout.
    fn closed_by_server(stream: &mut StdStream) -> bool {
        let mut rest = Vec::new();
        match stream.read_to_end(&mut rest) {
            Ok(_) => true,
            Err(e) => e.kind() == ErrorKind::ConnectionReset,
        }
    }

    fn get_root(stream: &mut StdStream) -> String {
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("the request is sent");
        let mut reply = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !String::from_utf8_lossy(&reply).ends_with("ok") {
            let read = stream.read(&mut chunk).expect("a reply");
            assert_ne!(read, 0, "closed before the reply ended");
            reply.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[test]
    fn should_close_connection_when_head_not_sent_within_timeout() {
        // given
        let server = start(limits(Duration::from_millis(200), 8));
        let mut stream = server.connect();

        // when
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: loc")
            .expect("a partial head is sent");

        // then
        assert!(
            closed_by_server(&mut stream),
            "the half-sent head is dropped"
        );
        server.stop(StopSignal::Interrupt);
        assert_eq!(server.finish(), Stopped::Drained);
    }

    #[test]
    fn should_close_idle_keep_alive_connection_when_timeout_passes() {
        // given
        let server = start(limits(Duration::from_millis(200), 8));
        let mut stream = server.connect();

        // when
        let reply = get_root(&mut stream);

        // then
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(
            closed_by_server(&mut stream),
            "the idle connection is dropped"
        );
        server.stop(StopSignal::Interrupt);
        assert_eq!(server.finish(), Stopped::Drained);
    }

    #[test]
    fn should_close_excess_connection_when_cap_reached() {
        // given: one permit, held by a connection that is still sending its head
        let server = start(limits(NEVER, 1));
        let mut holder = server.connect();
        holder
            .write_all(b"GET / HTTP/1.1\r\n")
            .expect("a partial head is sent");

        // when
        let mut excess = server.connect();

        // then
        assert!(
            closed_by_server(&mut excess),
            "the excess connection is closed"
        );
        drop(holder);
        // Polled: the permit returns once the server has seen the holder go, which only the
        // scheduler decides; until then a new connection is still turned away.
        wait_until(
            DEADLINE,
            "a connection served after the holder left",
            || {
                let mut next = server.connect();
                next.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .is_ok()
                    && {
                        let mut reply = Vec::new();
                        next.read_to_end(&mut reply).is_ok() && reply.starts_with(b"HTTP/1.1 200")
                    }
            },
        )
        .expect("the permit is returned");
        server.stop(StopSignal::Interrupt);
        assert_eq!(server.finish(), Stopped::Drained);
    }

    #[test]
    fn should_drop_stalled_connection_when_drain_bound_passes() {
        // given: a client that stopped reading a large response
        let mut limits = limits(NEVER, 8);
        limits.drain_timeout = Duration::from_millis(200);
        let server = start(limits);
        let mut stalled = server.stalled_reader();

        // when
        server.stop(StopSignal::Interrupt);

        // then
        assert_eq!(server.finish(), Stopped::DrainTimedOut);
        assert!(
            closed_by_server(&mut stalled),
            "the stalled connection is dropped"
        );
    }

    #[test]
    fn should_report_waiting_only_when_a_connection_outlasts_the_grace() {
        for stalls in [false, true] {
            // given: a client that stopped reading a large response, or no client at all; the
            // bound is longer than the grace, so a stalled drain is still running when it ends
            let mut limits = limits(NEVER, 8);
            limits.drain_timeout = GRACE * 2;
            let server = start(limits);
            let _stalled = stalls.then(|| server.stalled_reader());

            // when
            server.stop(StopSignal::Interrupt);

            // then: decided on the drain's clock, after the run, however late anyone looks
            let waiting = Arc::clone(&server.waiting);
            let expected = if stalls {
                Stopped::DrainTimedOut
            } else {
                Stopped::Drained
            };
            assert_eq!(server.finish(), expected, "stalls: {stalls}");
            assert_eq!(waiting.load(Ordering::SeqCst), stalls, "stalls: {stalls}");
        }
    }

    #[test]
    fn should_stop_at_once_when_second_stop_arrives_during_drain() {
        // given
        let server = start(limits(NEVER, 8));
        let mut stalled = server.stalled_reader();

        // when
        server.stop(StopSignal::Interrupt);
        server.stop(StopSignal::Terminate);

        // then
        assert_eq!(server.finish(), Stopped::Forced(StopSignal::Terminate));
        assert!(
            closed_by_server(&mut stalled),
            "the stalled connection is dropped"
        );
    }
}
