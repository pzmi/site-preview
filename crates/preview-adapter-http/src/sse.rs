//! The live-reload event stream, `GET /__site_preview/events` (plan.md section 7.2).
//!
//! One connection is one [`StateSubscription`]: taken before the initial value is read, so a
//! publish racing with the connect arrives exactly once, as the baseline or as the next
//! `reload` (ADR 0013; pinned in the application by
//! `should_observe_publish_after_subscribe_when_published_between_subscribe_and_read`). The
//! subscription only ever holds the newest value, so a slow or stalled browser costs one value,
//! never a queue (plan.md section 7.4).
//!
//! A `reload` event carries what changed (ADR 0030, ADR 0031) through [`ReloadPayload`], the
//! adapter's own wire shape: the domain types carry no serde (foundation.md §2.2). Because
//! coalescing drops the change sets of skipped revisions, each connection remembers the last
//! state it sent and never offers a hot update across a gap.

use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Extension;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use preview_application::{Published, StateSubscription};
use preview_domain::{AssetKind, Authority, SiteState};
use tokio::sync::OwnedSemaphorePermit;
use tokio_stream::Stream;

use crate::shutdown::ShutdownSignal;
use crate::sse_guard::{admit, busy, forbidden};
use crate::state::AppState;

/// How long a stream may stay silent before a keepalive comment (`:`) is sent (plan.md
/// section 7.2). The timer restarts after every event.
pub const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Answers `GET` and `HEAD /__site_preview/events`.
///
/// - Admission first, before anything is subscribed: the Origin check against the authority
///   the Host gate accepted (`403`, `should_reject_with_403_when_origin_cross_site`), then a
///   permit from [`AppState::streams`] (`503`, `should_return_503_when_limit_reached`). The
///   permit lives as long as the stream: dropping the body, the end of the stream on
///   shutdown and the reload state closing all return it
///   (`should_release_permit_when_client_disconnects`,
///   `should_release_permit_when_stream_ends_on_shutdown`).
/// - `HEAD` passes the Origin check, then answers the stream's headers with no body,
///   without subscribing or taking a permit: no stream ever follows a `HEAD`
///   (`should_answer_head_without_taking_permit_when_limit_reached`).
/// - First a `state` event with the value current at subscribe time, `id:` its revision and
///   data `{"instance_id":…,"revision":…}` only: it is a baseline, so the changes that
///   produced that value are not sent (`should_keep_state_event_shape_when_connected`); `Last-Event-ID` is not read: every
///   connection starts from the authoritative current state
///   (`should_ignore_last_event_id_when_reconnecting`).
/// - Then one `reload` event per value the subscription yields, its data a
///   [`ReloadPayload`]; publishes that happen between two polls collapse into the newest
///   (`should_send_reload_with_new_revision_when_published`,
///   `should_send_full_when_revisions_coalesced`).
/// - A keepalive comment after [`AppState::keep_alive`] of silence
///   (`should_send_keepalive_when_interval_elapses`).
/// - The stream ends when the [`ShutdownSignal`] fires or the reload state is dropped
///   (`should_end_stream_when_shutdown_token_cancelled`,
///   `should_end_stream_when_reload_state_closed`).
///
/// `Content-Type: text/event-stream`, `X-Accel-Buffering: no` so a buffering proxy passes
/// events through; `no-store` and `nosniff` come from the outer `harden` layer
/// (`should_set_stream_headers_when_connected`).
pub(crate) async fn events(
    State(state): State<AppState>,
    Extension(host): Extension<Authority>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    if admit(&headers, &host).is_err() {
        return forbidden();
    }
    if method == Method::HEAD {
        let mut response = Response::new(Body::empty());
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        return unbuffered(response);
    }
    let Some(permit) = state.streams.acquire() else {
        return busy();
    };
    let subscription = state.reload.subscribe();
    let baseline = subscription.initial().state.clone();
    let first = state_event(&baseline);
    let feed = Feed {
        subscription,
        last_sent: baseline,
        shutdown: state.shutdown,
        _permit: permit,
    };
    let stream = EventStream {
        first: Some(first),
        next: Some(Box::pin(feed.next())),
    };
    unbuffered(
        Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(state.keep_alive))
            .into_response(),
    )
}

fn unbuffered(mut response: Response) -> Response {
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

/// `event: state`, `id: <revision>`, `data: {"instance_id":…,"revision":…}` (plan.md
/// section 7.2).
///
/// The data is built by `serde_json`, so the instance id is escaped as a JSON string even
/// though its alphabet (`[A-Za-z0-9-]`) needs no escaping today.
fn state_event(state: &SiteState) -> Event {
    let revision = state.revision().get();
    let data = serde_json::json!({
        "instance_id": state.instance_id().as_str(),
        "revision": revision,
    });
    Event::default()
        .event("state")
        .id(revision.to_string())
        .data(data.to_string())
}

/// The data of a `reload` event (ADR 0030, ADR 0031):
/// `{"instance_id":…,"revision":…,"changes":[{"path":…,"kind":…}],"full":…}`.
///
/// - `path` is [`preview_domain::ChangedPath::url_path`] as the domain gives it, already percent-encoded per
///   component (`should_send_encoded_path_when_name_has_space`); `kind` is `css`, `js`,
///   `html`, `asset` or `other` (`should_name_each_kind_when_published`).
/// - `full` is the change set's own verdict; a full set still lists the paths it kept, for
///   information only (`should_send_full_when_change_set_full`).
/// - Across a gap, when the revision is not the one after the last this connection sent or
///   the instance differs (`should_send_full_when_instance_differs`), the skipped revisions' changes are gone: `full` is `true` and
///   `changes` is empty (`should_send_full_when_revisions_coalesced`), and the next
///   contiguous revision is hot again (`should_send_changes_again_when_gap_followed_by_next_revision`).
#[derive(Debug)]
struct ReloadPayload<'a> {
    instance_id: &'a str,
    revision: u64,
    changes: Vec<ChangeEntry<'a>>,
    full: bool,
}

/// One entry of [`ReloadPayload::changes`].
#[derive(Debug)]
struct ChangeEntry<'a> {
    path: &'a str,
    kind: &'static str,
}

impl<'a> ReloadPayload<'a> {
    /// The payload for `published`, sent on a connection whose previous event announced
    /// `last_sent`.
    fn after(last_sent: &SiteState, published: &'a Published) -> Self {
        let state = &published.state;
        let contiguous = last_sent.instance_id() == state.instance_id()
            && last_sent.revision().get().checked_add(1) == Some(state.revision().get());
        let changes = if contiguous {
            published
                .changes
                .changes()
                .iter()
                .map(|change| ChangeEntry {
                    path: change.url_path(),
                    kind: kind_code(change.kind()),
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            instance_id: state.instance_id().as_str(),
            revision: state.revision().get(),
            changes,
            full: !contiguous || published.changes.is_full(),
        }
    }

    /// The event, its data's keys in the documented order (`instance_id`, `revision`,
    /// `changes`, `full`; `path`, `kind`). `serde_json` without `preserve_order` sorts object
    /// keys, so the object is assembled here and only the strings go through `serde_json`,
    /// which escapes them (`should_send_changes_when_single_css_revision_published`).
    fn event(&self) -> Event {
        let changes: Vec<String> = self
            .changes
            .iter()
            .map(|change| {
                format!(
                    r#"{{"path":{},"kind":{}}}"#,
                    json_string(change.path),
                    json_string(change.kind)
                )
            })
            .collect();
        let data = format!(
            r#"{{"instance_id":{},"revision":{},"changes":[{}],"full":{}}}"#,
            json_string(self.instance_id),
            self.revision,
            changes.join(","),
            self.full
        );
        Event::default()
            .event("reload")
            .id(self.revision.to_string())
            .data(data)
    }
}

/// `value` as a JSON string literal, quoted and escaped.
fn json_string(value: &str) -> String {
    serde_json::Value::from(value).to_string()
}

/// The wire name of a kind; part of the protocol, so spelled out rather than derived.
const fn kind_code(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Css => "css",
        AssetKind::Js => "js",
        AssetKind::Html => "html",
        AssetKind::Asset => "asset",
        AssetKind::Other => "other",
    }
}

/// What a connection waits on between events, and its permit: dropped with the feed, so
/// with the body or at the end of the stream.
struct Feed {
    subscription: StateSubscription,
    /// The state the last event on this connection announced: the gap rule compares with it.
    last_sent: SiteState,
    shutdown: ShutdownSignal,
    _permit: OwnedSemaphorePermit,
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
        let event = ReloadPayload::after(&self.last_sent, &published).event();
        self.last_sent = published.state;
        Some((event, self))
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

#[cfg(test)]
mod tests {
    use super::*;
    use preview_domain::{ChangeEvent, ChangeKind, ChangeSet, Component, InstanceId, RelPath};

    fn state(instance: &str, revision: u64) -> SiteState {
        let mut state = SiteState::initial(InstanceId::parse(instance).expect("valid id"));
        while state.revision().get() != revision {
            state = state.advanced().expect("not exhausted");
        }
        state
    }

    #[test]
    fn should_send_full_when_instance_differs() {
        // given: the next revision number, but announced by another instance
        let mut changes = ChangeSet::new();
        changes.accumulate(&ChangeEvent {
            kind: ChangeKind::Modify,
            path: RelPath::new(vec![Component::new("site.css").expect("valid")]),
        });
        let published = Published {
            state: state("other-2", 1),
            changes,
        };

        // when
        let payload = ReloadPayload::after(&state("first-1", 0), &published);

        // then
        assert!(payload.full);
        assert!(payload.changes.is_empty());
    }
}
