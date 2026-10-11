//! Ending the open event streams when the server shuts down (plan.md sections 7.4 and 9).
//!
//! A graceful HTTP shutdown waits for every in-flight response to finish, and an event stream
//! never finishes on its own. [`Shutdown`] is the switch composition flips when it stops: every
//! stream holding a [`ShutdownSignal`] ends at its next poll, so the server's own shutdown is
//! never held open by a browser tab.
//!
//! A tokio `watch` rather than `tokio_util`'s `CancellationToken`: the adapter already uses
//! `watch`, and a flag that is read, never queued, needs no further dependency (ADR 0026).

use tokio::sync::watch;

/// The owner's side: fires once, for every [`ShutdownSignal`] made from it.
///
/// Dropping it counts as firing, so a handle lost on an error path cannot leave a stream
/// running (`should_end_stream_when_shutdown_handle_dropped` in `tests/sse.rs`).
#[derive(Debug)]
pub struct Shutdown {
    tx: watch::Sender<bool>,
}

impl Shutdown {
    /// A switch that has not fired.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    /// A signal for [`crate::AppState`]; it sees a firing that happened before it was made.
    #[must_use]
    pub fn signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            rx: self.tx.subscribe(),
        }
    }

    /// Fires: every open stream ends (`should_end_stream_when_shutdown_token_cancelled`).
    /// Idempotent.
    pub fn trigger(&self) {
        self.tx.send_replace(true);
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

/// The streams' side of a [`Shutdown`]. Clones observe the same switch.
#[derive(Debug, Clone)]
pub struct ShutdownSignal {
    rx: watch::Receiver<bool>,
}

impl ShutdownSignal {
    /// Resolves once the switch has fired or its [`Shutdown`] is gone. Cancel safe: it reads
    /// the current flag first, so a firing is never missed by a dropped and re-made future.
    pub(crate) async fn fired(&mut self) {
        // An error means the `Shutdown` was dropped, which counts as firing.
        self.rx.wait_for(|fired| *fired).await.ok();
    }
}
