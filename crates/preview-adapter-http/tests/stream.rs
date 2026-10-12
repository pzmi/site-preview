//! Streaming-body resource tests: a client that stops reading holds no blocking thread, and a
//! dropped body stops the reading.

use std::io::{self, Cursor, Read};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use preview_adapter_http::{
    AppState, DEFAULT_KEEP_ALIVE, DEFAULT_MAX_STREAMS, Injection, Shutdown, StreamLimit,
    build_router,
};
use preview_application::{ContentFile, ContentRefusal, Opened, ReloadState, SiteContent};
use preview_domain::{BoundAddr, HostPolicy, SitePath};
use preview_testkit::poll::wait_until;
use preview_testkit::{FixedInstanceIds, InMemorySite};
use tower::ServiceExt;

/// Generous: on a loaded CI machine the second request may take a while, never this long.
const DEADLINE: Duration = Duration::from_secs(30);

fn app(content: Arc<dyn SiteContent>) -> Router {
    let mut ids = FixedInstanceIds::new(1, 1);
    build_router(AppState {
        content,
        reload: ReloadState::new(ids.next_id()),
        host: HostPolicy::new(BoundAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80), false),
        shutdown: Shutdown::new().signal(),
        keep_alive: DEFAULT_KEEP_ALIVE,
        streams: StreamLimit::new(DEFAULT_MAX_STREAMS),
        injection: Injection::default(),
    })
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .header("host", "127.0.0.1")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

#[test]
fn should_complete_other_request_when_client_stalls_body() {
    // given: one blocking thread, shared by request resolution and file reads, and a client
    // that took the first chunk of a large file and then stopped reading
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .expect("a runtime");
    let site = InMemorySite::new()
        .with_file("big.bin", &vec![7_u8; 4 * 1024 * 1024])
        .with_file("small.txt", b"small");
    let app = app(Arc::new(site));
    let stalled_app = app.clone();
    let stalled = runtime.block_on(async move {
        let response = stalled_app
            .oneshot(get("/big.bin"))
            .await
            .expect("a response");
        let mut body = response.into_body();
        let first = body.frame().await.expect("a first frame").expect("a chunk");
        assert!(first.is_data());
        body
    });

    // when: another client asks for a file while the first one still reads nothing
    let (tx, rx) = mpsc::channel();
    let handle = runtime.handle().clone();
    thread::spawn(move || {
        let body = handle.block_on(async move {
            let response = app.oneshot(get("/small.txt")).await.expect("a response");
            assert_eq!(response.status(), StatusCode::OK);
            response
                .into_body()
                .collect()
                .await
                .expect("a body")
                .to_bytes()
        });
        let _ = tx.send(body);
    });
    let mut second = None;
    // Polled, not slept: the second request runs on another OS thread, and only the
    // scheduler decides when it finishes.
    let waited = wait_until(
        DEADLINE,
        "the second response while the first stalls",
        || {
            second = second.take().or_else(|| rx.try_recv().ok());
            second.is_some()
        },
    );
    drop(stalled);
    runtime.shutdown_background();

    // then
    assert!(waited.is_ok(), "the stalled body starved the blocking pool");
    assert_eq!(second.expect("a second body").as_ref(), b"small");
}

/// A file that counts its reads.
struct Counted {
    data: Vec<u8>,
    reads: Arc<AtomicUsize>,
}

struct CountedFile {
    cursor: Cursor<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}

impl Read for CountedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.cursor.read(buf)
    }
}

impl ContentFile for CountedFile {
    fn len(&self) -> u64 {
        self.cursor.get_ref().len() as u64
    }
}

impl SiteContent for Counted {
    fn open(&self, _path: &SitePath) -> Result<Opened, ContentRefusal> {
        Ok(Opened::File(Box::new(CountedFile {
            cursor: Cursor::new(self.data.clone()),
            reads: Arc::clone(&self.reads),
        })))
    }
}

#[tokio::test]
async fn should_stop_reading_when_body_dropped() {
    // given: a large file whose first chunk the client took
    let reads = Arc::new(AtomicUsize::new(0));
    let app = app(Arc::new(Counted {
        data: vec![1_u8; 4 * 1024 * 1024],
        reads: Arc::clone(&reads),
    }));
    let response = app.clone().oneshot(get("/x.bin")).await.unwrap();
    let mut body = response.into_body();
    body.frame().await.expect("a first frame").expect("a chunk");
    let after_first = reads.load(Ordering::SeqCst);

    // when: the client goes away, and the server goes on to answer another request
    drop(body);
    let other = app.oneshot(get("/y.bin")).await.unwrap();
    let other_body = other.into_body().collect().await.unwrap().to_bytes();

    // then: one read per chunk taken, none ahead of the client and none after the drop
    assert_eq!(after_first, 1);
    assert_eq!(other_body.len(), 4 * 1024 * 1024);
    let total = reads.load(Ordering::SeqCst);
    assert_eq!(total, 1 + 4 * 1024 * 1024 / (64 * 1024));
}
