//! Stop signals and the binary's graceful shutdown (plan.md section 9, section 7.4 bullet 5,
//! ADR 0028, ADR 0045, ADR 0051).
//!
//! [`stop_signals`] turns every SIGINT and SIGTERM, in the order the operating system delivers
//! them, into a [`StopSignal`] on the serve task's stop channel. The first one closes the
//! listener; [`Server::serve_until_stopped`] then runs the lifecycle's shutdown in plan.md's
//! order:
//!
//! 1. **listener**: closed by the accept loop at the first signal, so a new connection is
//!    refused from then on (`should_close_listener_when_shut_down`);
//! 2. **event streams**: the HTTP-10 [`Shutdown`](preview_adapter_http::Shutdown) switch ends
//!    every open stream, so a browser tab never holds the process
//!    (`should_exit_0_when_sigint_received_with_open_sse_connection`,
//!    `should_drain_without_timeout_when_stopped_with_open_event_stream`);
//! 3. **watcher**: the pipeline task is cancelled; it closes the native watcher and ends;
//! 4. **tasks**: the serve task's drain and the pipeline are joined within the drain bound,
//!    then the runtime is shut down without waiting for anything else.
//!
//! A drained or timed-out drain is a clean stop (exit 0, `should_exit_0_when_sigterm_received`);
//! a second signal during the drain stops at once ([`Stopped::Forced`], exit 130 or 143).
//!
//! The handlers are installed when the lifecycle binds the listener, before readiness is
//! reported: a signal sent as soon as the URL is known is queued on the channel, never the
//! default action that would end the process uncleanly.

use core::future::poll_fn;
use core::task::Poll;
use std::io;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::connections::{StopSignal, Stopped};
use crate::lifecycle::Server;

/// Installs the SIGINT and SIGTERM handlers and forwards every delivery, in order, to `tx`.
/// Must run inside the runtime.
///
/// # Errors
///
/// The operating system refused to install a handler.
pub(crate) fn stop_signals(tx: mpsc::UnboundedSender<StopSignal>) -> io::Result<()> {
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::spawn(async move {
        loop {
            let next = poll_fn(|cx| {
                if let Poll::Ready(delivered) = interrupt.poll_recv(cx) {
                    return Poll::Ready(delivered.map(|()| StopSignal::Interrupt));
                }
                terminate
                    .poll_recv(cx)
                    .map(|delivered| delivered.map(|()| StopSignal::Terminate))
            })
            .await;
            // `None`: the signal driver is gone with the runtime; nothing more can arrive.
            let Some(stop) = next else { return };
            if tx.send(stop).is_err() {
                return;
            }
        }
    });
    Ok(())
}

impl Server {
    /// Serves until the first stop signal closes the listener, then performs the rest of the
    /// shutdown (the binary's run). Signals that arrived during start-up were queued and stop
    /// the server at once.
    pub(crate) fn serve_until_stopped(&mut self) -> Stopped {
        self.await_listener_closed();
        self.stop()
    }
}
