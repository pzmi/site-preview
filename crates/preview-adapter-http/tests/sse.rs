//! The live-reload event stream, `GET /__site_preview/events` (plan.md section 7.2).
//!
//! Every test runs on a paused clock: a poll that would wait lets tokio jump to the next
//! timer, so a keepalive arrives after exactly the configured interval and "nothing else was
//! queued" shows up as the next frame being a keepalive rather than an event. No sleeps.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{AppState, DEFAULT_MAX_STREAMS, Shutdown, StreamLimit, build_router};
use preview_application::ReloadState;
use preview_domain::{
    BoundAddr, ChangeEvent, ChangeKind, ChangeSet, Component, HostPolicy, RelPath,
};
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tokio::time::Instant;
use tower::ServiceExt;

/// Not the 15 s default, so the tests show the interval is the injected one.
const KEEP_ALIVE: Duration = Duration::from_secs(3);
const KEEP_ALIVE_FRAME: &str = ":\n\n";

struct Server {
    reload: ReloadState,
    shutdown: Shutdown,
    streams: StreamLimit,
    instance: String,
}

impl Server {
    fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_STREAMS)
    }

    fn with_limit(max_streams: usize) -> Self {
        let instance = FixedInstanceIds::new(7, 1).next_id();
        Self {
            instance: instance.as_str().to_owned(),
            reload: ReloadState::new(instance),
            shutdown: Shutdown::new(),
            streams: StreamLimit::new(max_streams),
        }
    }

    async fn connect(&self, headers: &[(&str, &str)]) -> (axum::http::response::Parts, Body) {
        open(build_router(state_of(self)), headers).await
    }

    async fn send(&self, request: Request<Body>) -> (axum::http::response::Parts, Body) {
        let response = build_router(state_of(self))
            .oneshot(request)
            .await
            .expect("response");
        response.into_parts()
    }

    fn publish(&self) {
        self.reload
            .publish_next(ChangeSet::new())
            .expect("not exhausted");
    }

    /// A `reload` frame with `changes` as `(path, kind)` pairs.
    fn reload(&self, revision: u64, changes: &[(&str, &str)], full: bool) -> String {
        let changes: Vec<String> = changes
            .iter()
            .map(|(path, kind)| format!("{{\"path\":\"{path}\",\"kind\":\"{kind}\"}}"))
            .collect();
        format!(
            "event: reload\nid: {revision}\ndata: {{\"instance_id\":\"{}\",\"revision\":{revision},\"changes\":[{}],\"full\":{full}}}\n\n",
            self.instance,
            changes.join(",")
        )
    }

    fn publish_events(&self, events: &[ChangeEvent]) {
        let mut changes = ChangeSet::new();
        for event in events {
            changes.accumulate(event);
        }
        self.reload.publish_next(changes).expect("not exhausted");
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
        streams: server.streams.clone(),
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
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[], false))
    );
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
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(3, &[], true))
    );
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
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[], false))
    );
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
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[], false))
    );
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

fn request(method: Method, host: &str, headers: &[(&str, HeaderValue)]) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri("/__site_preview/events")
        .header("host", host);
    for (name, value) in headers {
        request = request.header(*name, value.clone());
    }
    request.body(Body::empty()).expect("request")
}

fn text(value: &'static str) -> HeaderValue {
    HeaderValue::from_static(value)
}

fn header<'a>(parts: &'a axum::http::response::Parts, name: &str) -> Option<&'a str> {
    parts.headers.get(name).and_then(|v| v.to_str().ok())
}

async fn body_text(body: Body) -> String {
    let bytes = body.collect().await.expect("body").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

/// A fixed 403 with no CORS header and nothing from the request; and nothing subscribed or
/// held: the server's only permit is still free afterwards.
async fn assert_forbidden(
    server: &Server,
    (parts, body): (axum::http::response::Parts, Body),
    case: &str,
) {
    assert_eq!(parts.status, StatusCode::FORBIDDEN, "{case}");
    assert_eq!(header(&parts, "cache-control"), Some("no-store"), "{case}");
    assert!(
        !parts
            .headers
            .keys()
            .any(|k| k.as_str().starts_with("access-control-")),
        "{case}"
    );
    assert_eq!(body_text(body).await, "403 Forbidden", "{case}");
    let (after, _body) = server.connect(&[]).await;
    assert_eq!(after.status, StatusCode::OK, "{case}: permit still free");
}

#[tokio::test(start_paused = true)]
async fn should_reject_with_403_when_origin_cross_site() {
    for (case, headers) in [
        (
            "cross-site fetch",
            vec![("sec-fetch-site", text("cross-site"))],
        ),
        (
            "cross-site fetch with a matching origin",
            vec![
                ("origin", text("http://127.0.0.1")),
                ("sec-fetch-site", text("cross-site")),
            ],
        ),
        (
            "foreign origin",
            vec![("origin", text("http://evil.example"))],
        ),
        ("opaque origin", vec![("origin", text("null"))]),
        ("other scheme", vec![("origin", text("https://127.0.0.1"))]),
        ("other port", vec![("origin", text("http://127.0.0.1:81"))]),
        (
            "localhost is another origin",
            vec![("origin", text("http://localhost"))],
        ),
        (
            "origin with a path",
            vec![("origin", text("http://127.0.0.1/x"))],
        ),
    ] {
        // given
        let server = Server::with_limit(1);
        let refused = request(Method::GET, "127.0.0.1", &headers);

        // when
        let outcome = server.send(refused).await;

        // then
        assert_forbidden(&server, outcome, case).await;
    }
}

#[tokio::test(start_paused = true)]
async fn should_reject_with_403_when_origin_header_is_ambiguous() {
    let non_utf8 = HeaderValue::from_bytes(b"http://127.0.0.1\xff").expect("obs-text is valid");
    for (case, headers) in [
        (
            "repeated origin",
            vec![
                ("origin", text("http://127.0.0.1")),
                ("origin", text("http://127.0.0.1")),
            ],
        ),
        ("non-UTF-8 origin", vec![("origin", non_utf8.clone())]),
        (
            "repeated sec-fetch-site",
            vec![
                ("sec-fetch-site", text("same-origin")),
                ("sec-fetch-site", text("same-origin")),
            ],
        ),
    ] {
        // given
        let server = Server::with_limit(1);
        let refused = request(Method::GET, "127.0.0.1", &headers);

        // when
        let outcome = server.send(refused).await;

        // then
        assert_forbidden(&server, outcome, case).await;
    }
}

#[tokio::test(start_paused = true)]
async fn should_reject_with_403_when_origin_differs_from_accepted_host() {
    // given: both authorities are ones the Host gate accepts, but not the same one
    let server = Server::with_limit(1);
    let refused = request(
        Method::GET,
        "localhost",
        &[("origin", text("http://127.0.0.1"))],
    );

    // when
    let outcome = server.send(refused).await;

    // then
    assert_forbidden(&server, outcome, "127.0.0.1 origin on a localhost host").await;
}

#[tokio::test(start_paused = true)]
async fn should_accept_when_no_origin_headers() {
    for (host, headers) in [
        ("127.0.0.1", vec![]),
        (
            "127.0.0.1",
            vec![
                ("origin", text("http://127.0.0.1")),
                ("sec-fetch-site", text("same-origin")),
            ],
        ),
        ("localhost", vec![("origin", text("http://localhost:80"))]),
        ("127.0.0.1", vec![("sec-fetch-site", text("same-site"))]),
    ] {
        // given
        let server = Server::new();

        // when
        let (parts, mut body) = server.send(request(Method::GET, host, &headers)).await;

        // then
        assert_eq!(parts.status, StatusCode::OK, "{host} {headers:?}");
        assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));
    }
}

#[tokio::test(start_paused = true)]
async fn should_return_503_when_limit_reached() {
    // given
    let server = Server::with_limit(2);
    let (first, _a) = server.connect(&[]).await;
    let (second, _b) = server.connect(&[]).await;

    // when
    let (third, body) = server.connect(&[]).await;

    // then
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(third.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header(&third, "retry-after"), Some("2"));
    assert_eq!(header(&third, "cache-control"), Some("no-store"));
    assert_eq!(
        header(&third, "content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(body_text(body).await, "503 Service Unavailable");
}

#[tokio::test(start_paused = true)]
async fn should_return_503_when_default_limit_of_64_streams_open() {
    // given
    let server = Server::new();
    let mut open = Vec::new();
    for _ in 0..64 {
        let (parts, body) = server.connect(&[]).await;
        assert_eq!(parts.status, StatusCode::OK);
        open.push(body);
    }

    // when
    let (over, _) = server.connect(&[]).await;

    // then
    assert_eq!(DEFAULT_MAX_STREAMS, 64);
    assert_eq!(over.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header(&over, "cache-control"), Some("no-store"));
}

#[tokio::test(start_paused = true)]
async fn should_release_permit_when_client_disconnects() {
    // given
    let server = Server::with_limit(1);
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));
    let (refused, _) = server.connect(&[]).await;
    assert_eq!(refused.status, StatusCode::SERVICE_UNAVAILABLE);

    // when
    drop(body);

    // then
    let (parts, mut again) = server.connect(&[]).await;
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(next_frame(&mut again).await, Some(server.event("state", 0)));
}

#[tokio::test(start_paused = true)]
async fn should_release_permit_when_stream_ends_on_shutdown() {
    // given
    let server = Server::with_limit(1);
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));

    // when: the stream ends while the client still holds the body
    server.shutdown.trigger();
    assert_eq!(next_frame(&mut body).await, None);

    // then
    let (parts, _again) = server.connect(&[]).await;
    assert_eq!(parts.status, StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn should_answer_head_without_taking_permit_when_limit_reached() {
    // given: the only permit is held
    let server = Server::with_limit(1);
    let (_, _held) = server.connect(&[]).await;

    // when
    let (parts, body) = server.send(request(Method::HEAD, "127.0.0.1", &[])).await;

    // then
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(header(&parts, "content-type"), Some("text/event-stream"));
    assert_eq!(header(&parts, "cache-control"), Some("no-store"));
    assert_eq!(body_text(body).await, "");
}

#[tokio::test(start_paused = true)]
async fn should_reject_with_403_when_head_is_cross_site() {
    // given
    let server = Server::with_limit(1);
    let refused = request(
        Method::HEAD,
        "127.0.0.1",
        &[("sec-fetch-site", text("cross-site"))],
    );

    // when
    let (parts, _) = server.send(refused).await;

    // then
    assert_eq!(parts.status, StatusCode::FORBIDDEN);
}

fn change(kind: ChangeKind, path: &str) -> ChangeEvent {
    ChangeEvent {
        kind,
        path: RelPath::new(
            path.split('/')
                .map(|part| Component::new(part).expect("valid test component"))
                .collect(),
        ),
    }
}

async fn connected(server: &Server) -> Body {
    let (_, mut body) = server.connect(&[]).await;
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 0)));
    body
}

#[tokio::test(start_paused = true)]
async fn should_send_changes_when_single_css_revision_published() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;

    // when
    server.publish_events(&[change(ChangeKind::Modify, "css/site.css")]);

    // then
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[("/css/site.css", "css")], false))
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_full_when_revisions_coalesced() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;

    // when: two hot revisions before the stream is polled
    server.publish_events(&[change(ChangeKind::Modify, "a.css")]);
    server.publish_events(&[change(ChangeKind::Modify, "b.css")]);

    // then: one event, full, with no changes: those of revision 1 are gone
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(2, &[], true))
    );
    assert_eq!(
        next_frame(&mut body).await.as_deref(),
        Some(KEEP_ALIVE_FRAME)
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_changes_again_when_gap_followed_by_next_revision() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;
    server.publish_events(&[change(ChangeKind::Modify, "a.css")]);
    server.publish_events(&[change(ChangeKind::Modify, "b.css")]);
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(2, &[], true))
    );

    // when
    server.publish_events(&[change(ChangeKind::Modify, "c.css")]);

    // then
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(3, &[("/c.css", "css")], false))
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_full_when_change_set_full() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;

    // when: an html change makes the set full; its paths stay, for information
    server.publish_events(&[
        change(ChangeKind::Modify, "index.html"),
        change(ChangeKind::Modify, "site.css"),
    ]);

    // then
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[("/index.html", "html"), ("/site.css", "css")], true))
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_full_without_changes_when_directory_changed() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;

    // when
    server.publish_events(&[change(ChangeKind::DirectoryChanged, "img")]);

    // then
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[], true))
    );
}

#[tokio::test(start_paused = true)]
async fn should_name_each_kind_when_published() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;

    // when
    server.publish_events(&[
        change(ChangeKind::Modify, "app.js"),
        change(ChangeKind::Modify, "data.json"),
        change(ChangeKind::Modify, "logo.png"),
        change(ChangeKind::Modify, "mod.mjs"),
        change(ChangeKind::Modify, "page.htm"),
        change(ChangeKind::Modify, "site.css"),
    ]);

    // then
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(
            1,
            &[
                ("/app.js", "js"),
                ("/data.json", "other"),
                ("/logo.png", "asset"),
                ("/mod.mjs", "js"),
                ("/page.htm", "html"),
                ("/site.css", "css"),
            ],
            true
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn should_send_encoded_path_when_name_has_space() {
    // given
    let server = Server::new();
    let mut body = connected(&server).await;

    // when
    server.publish_events(&[change(ChangeKind::Modify, "css/my site\"q.css")]);

    // then: percent-encoded by the domain, sent as given
    assert_eq!(
        next_frame(&mut body).await,
        Some(server.reload(1, &[("/css/my%20site%22q.css", "css")], false))
    );
}

#[tokio::test(start_paused = true)]
async fn should_keep_state_event_shape_when_connected() {
    // given: the current value was produced by a hot change set
    let server = Server::new();
    server.publish_events(&[change(ChangeKind::Modify, "site.css")]);

    // when
    let (_, mut body) = server.connect(&[]).await;

    // then: the baseline names the state only, never the changes behind it
    assert_eq!(next_frame(&mut body).await, Some(server.event("state", 1)));
}
