//! The live-reload event stream, `GET /__site_preview/events` (plan.md section 7.2).
//!
//! One connection is one [`StateSubscription`]: taken before the initial value is read, so a
//! publish racing with the connect arrives exactly once, as the baseline or as the next
//! `reload` (ADR 0013; pinned in the application by
//! `should_observe_publish_after_subscribe_when_published_between_subscribe_and_read`). The
//! subscription only ever holds the newest value, so a slow or stalled browser costs one value,
//! never a queue (plan.md section 7.4).

use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderName, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use preview_application::StateSubscription;
use preview_domain::SiteState;
use tokio_stream::Stream;

use crate::shutdown::ShutdownSignal;
use crate::state::AppState;

/// How long a stream may stay silent before a keepalive comment (`:`) is sent (plan.md
/// section 7.2). The timer restarts after every event.
pub const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Answers `GET /__site_preview/events`.
///
/// - First a `state` event with the value current at subscribe time, `id:` its revision
///   (`should_send_state_baseline_when_connected`); `Last-Event-ID` is not read: every
///   connection starts from the authoritative current state
///   (`should_ignore_last_event_id_when_reconnecting`).
/// - Then one `reload` event per value the subscription yields; publishes that happen
///   between two polls collapse into the newest
///   (`should_send_reload_with_new_revision_when_published`,
///   `should_send_only_latest_when_several_published_before_poll`).
/// - A keepalive comment after [`AppState::keep_alive`] of silence
///   (`should_send_keepalive_when_interval_elapses`).
/// - The stream ends when the [`ShutdownSignal`] fires or the reload state is dropped
///   (`should_end_stream_when_shutdown_token_cancelled`,
///   `should_end_stream_when_reload_state_closed`).
///
/// `Content-Type: text/event-stream`, `X-Accel-Buffering: no` so a buffering proxy passes
/// events through; `no-store` and `nosniff` come from the outer `harden` layer
/// (`should_set_stream_headers_when_connected`).
pub(crate) async fn events(State(state): State<AppState>) -> Response {
    let subscription = state.reload.subscribe();
    let first = state_event("state", &subscription.initial().state);
    let feed = Feed {
        subscription,
        shutdown: state.shutdown,
    };
    let stream = EventStream {
        first: Some(first),
        next: Some(Box::pin(feed.next())),
    };
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(state.keep_alive))
        .into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

/// `event: <name>`, `id: <revision>`, `data: {"instance_id":…,"revision":…}`.
///
/// The data is built by `serde_json`, so the instance id is escaped as a JSON string even
/// though its alphabet (`[A-Za-z0-9-]`) needs no escaping today.
fn state_event(name: &'static str, state: &SiteState) -> Event {
    let revision = state.revision().get();
    let data = serde_json::json!({
        "instance_id": state.instance_id().as_str(),
        "revision": revision,
    });
    Event::default()
        .event(name)
        .id(revision.to_string())
        .data(data.to_string())
}

/// What a connection waits on between events.
struct Feed {
    subscription: StateSubscription,
    shutdown: ShutdownSignal,
}

impl Feed {
    /// The next `reload` event and the feed to continue from, or `None` when the stream ends.
    async fn next(mut self) -> Option<(Event, Self)> {
        let published = {
            let mut fired = pin!(self.shutdown.fired());
            let mut changed = pin!(self.subscription.changed());
            // Shutdown is polled first: once it has fired, no further event is sent, even if
            // a publish is ready at the same moment.
            poll_fn(|cx| {
                if fired.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
                changed.as_mut().poll(cx).map(Result::ok)
            })
            .await?
        };
        Some((state_event("reload", &published.state), self))
    }
}

type Step = Pin<Box<dyn Future<Output = Option<(Event, Feed)>> + Send>>;

/// The baseline event, then whatever the feed yields, until it ends.
struct EventStream {
    first: Option<Event>,
    next: Option<Step>,
}

impl Stream for EventStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(first) = this.first.take() {
            return Poll::Ready(Some(Ok(first)));
        }
        let Some(step) = this.next.as_mut() else {
            return Poll::Ready(None);
        };
        match step.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                this.next = None;
                Poll::Ready(None)
            }
            Poll::Ready(Some((event, feed))) => {
                this.next = Some(Box::pin(feed.next()));
                Poll::Ready(Some(Ok(event)))
            }
        }
    }
}
