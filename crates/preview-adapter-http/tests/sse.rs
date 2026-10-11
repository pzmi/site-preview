//! The live-reload event stream, `GET /__site_preview/events` (plan.md section 7.2).
//!
//! Every test runs on a paused clock: a poll that would wait lets tokio jump to the next
//! timer, so a keepalive arrives after exactly the configured interval and "nothing else was
//! queued" shows up as the next frame being a keepalive rather than an event. No sleeps.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{AppState, Shutdown, build_router};
use preview_application::ReloadState;
use preview_domain::{BoundAddr, ChangeSet, HostPolicy};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tokio::time::Instant;
use tower::ServiceExt;

/// Not the 15 s default, so the tests show the interval is the injected one.
const KEEP_ALIVE: Duration = Duration::from_secs(3);
const KEEP_ALIVE_FRAME: &str = ":\n\n";

struct Server {
    reload: ReloadState,
    shutdown: Shutdown,
    instance: String,
}

impl Server {
    fn new() -> Self {
        let instance = FixedInstanceIds::new(7, 1).next_id();
        Self {
            instance: instance.as_str().to_owned(),
            reload: ReloadState::new(instance),
            shutdown: Shutdown::new(),
        }
    }

    async fn connect(&self, headers: &[(&str, &str)]) -> (axum::http::response::Parts, Body) {
        open(build_router(state_of(self)), headers).await
    }

    fn publish(&self) {
        self.reload
            .publish_next(ChangeSet::new())
            .expect("not exhausted");
    }

    fn event(&self, name: &str, revision: u64) -> String {
        format!(
            "event: {name}\nid: {revision}\ndata: {{\"instance_id\":\"{}\",\"revision\":{revision}}}\n\n",
            self.instance
        )
    }
}

async fn open(app: axum::Router, headers: &[(&str, &str)]) -> (axum::http::response::Parts, Body) {
    let mut request = Request::builder()
        .uri("/__site_preview/events")
        .header("host", "127.0.0.1");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Body::empty()).expect("request");
    let response = app.oneshot(request).await.expect("response");
    response.into_parts()
}

fn state_of(server: &Server) -> AppState {
    AppState {
        content: Arc::new(InMemorySite::new()),
        reload: server.reload.clone(),
        host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
        shutdown: server.shutdown.signal(),
        keep_alive: KEEP_ALIVE,
    }
}

/// The next frame of the body as text, or `None` once the stream has ended.
async fn next_frame(body: &mut Body) -> Option<String> {
    let frame = body.frame().await?.expect("an infallible stream");
    let data = frame.into_data().expect("a data frame");
    Some(String::from_utf8(data.to_vec()).expect("utf-8"))
}

#[tokio::test(start_paused = true)]
async fn should_send_state_baseline_when_connected() {
    // given
    let server = Server::new();
    server.publish();
    server.publish();

    // when
    let (_, mut body) = server.connect(&[]).await;

    // then
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 2)));
}

#[tokio::test(start_paused = true)]
async fn should_set_stream_headers_when_connected() {
    // given
    let server = Server::new();

    // when
    let (parts, _body) = server.connect(&[]).await;

    // then
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok());
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(header("content-type"), Some("text/event-stream"));
    assert_eq!(header("cache-control"), Some("no-store"));
    assert_eq!(header("x-content-type-options"), Some("nosniff"));
    assert_eq!(header("x-accel-buffering"), Some("no"));
}

#[tokio::test(start_paused = true)]
async fn should_send_reload_with_new_revision_when_published() {
    // given
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));

    // when
    server.publish();

    // then: exactly one reload for one publish, then only keepalives
    assert_eq!(next_frame(&mut body).await, Some(server.event("reload", 1)));
    assert_eq!(
        next_frame(&mut body).await.as_deref(),
        Some(KEEP_ALIVE_FRAME)
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_only_latest_when_several_published_before_poll() {
    // given
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));

    // when
    server.publish();
    server.publish();
    server.publish();

    // then
    assert_eq!(next_frame(&mut body).await, Some(server.event("reload", 3)));
    assert_eq!(
        next_frame(&mut body).await.as_deref(),
        Some(KEEP_ALIVE_FRAME)
    );
}

#[tokio::test(start_paused = true)]
async fn should_deliver_publish_once_when_published_between_connect_and_first_poll() {
    // given: the subscription is taken when the response is built, before any body poll
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;

    // when
    server.publish();

    // then
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));
    assert_eq!(next_frame(&mut body).await, Some(server.event("reload", 1)));
    assert_eq!(
        next_frame(&mut body).await.as_deref(),
        Some(KEEP_ALIVE_FRAME)
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_keepalive_when_interval_elapses() {
    // given
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));
    let start = Instant::now();

    // when
    let frame = next_frame(&mut body).await;

    // then
    assert_eq!(frame.as_deref(), Some(KEEP_ALIVE_FRAME));
    assert_eq!(start.elapsed(), KEEP_ALIVE);
}

#[tokio::test(start_paused = true)]
async fn should_restart_keepalive_timer_when_event_sent() {
    // given
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));
    tokio::time::advance(Duration::from_secs(2)).await;
    server.publish();
    assert_eq!(next_frame(&mut body).await, Some(server.event("reload", 1)));
    let sent = Instant::now();

    // when
    let frame = next_frame(&mut body).await;

    // then
    assert_eq!(frame.as_deref(), Some(KEEP_ALIVE_FRAME));
    assert_eq!(sent.elapsed(), KEEP_ALIVE);
}

#[tokio::test(start_paused = true)]
async fn should_end_stream_when_shutdown_token_cancelled() {
    // given
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));

    // when
    server.shutdown.trigger();
    server.publish();

    // then: no reload after shutdown, and no keepalive either
    assert_eq!(next_frame(&mut body).await, None);
}

#[tokio::test(start_paused = true)]
async fn should_end_stream_when_shutdown_handle_dropped() {
    // given
    let server = Server::new();
    let owner = Shutdown::new();
    let app = build_router(AppState {
        shutdown: owner.signal(),
        ..state_of(&server)
    });
    let (_, mut body) = open(app, &[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));

    // when
    drop(owner);

    // then
    assert_eq!(next_frame(&mut body).await, None);
}

#[tokio::test(start_paused = true)]
async fn should_end_stream_when_reload_state_closed() {
    // given: the router and the test hold the only reload handles
    let server = Server::new();
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));

    // when
    let Server { reload, .. } = server;
    drop(reload);

    // then
    assert_eq!(next_frame(&mut body).await, None);
}

#[tokio::test(start_paused = true)]
async fn should_ignore_last_event_id_when_reconnecting() {
    // given
    let server = Server::new();
    server.publish();

    // when
    let (_, mut body) = server.connect(&[("last-event-id", "41")]).await;

    // then
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 1)));
    assert_eq!(
        next_frame(&mut body).await.as_deref(),
        Some(KEEP_ALIVE_FRAME)
    );
}
