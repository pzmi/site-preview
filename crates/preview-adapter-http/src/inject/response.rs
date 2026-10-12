//! Serving an HTML file: the injection decision, the rewrite and the response headers
//! (plan.md §6 steps 1 to 3, 6 and 7; §7.3 "Snapshot ordering"; ADR 0022, ADR 0046).

use std::io::{self, ErrorKind, Read};
use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use preview_application::ContentFile;
use preview_domain::injection::{
    FetchDest, InjectionDecision, InjectionInput, Media, RequestMethod, decide,
};
use preview_domain::{
    ChangedPath, MAX_INJECTION_INPUT_BYTES, RelPath, SiteState, decode_request_path,
};

use super::budget::{HtmlBudget, buffered_body, charge};
use super::charset::{Encoding, classify};
use super::csp::{CSP_BLOCKS_LIVE_RELOAD, detect};
use super::{Rewrite, ScriptTag, rewrite_with_csp};
use crate::errors::plain;
use crate::mime::html_content_type;
use crate::stream::stream_body;
use crate::warnings::{OncePerPath, SilentWarnings, WarningSink};

/// The warning code for a page the rewriter refused (plan.md §6 step 6).
const REWRITE_FAILED: &str = "html-rewrite-failed";

/// What HTML serving needs beyond the content port: where warnings go and the memory budget.
///
/// Clones share both. [`Injection::default`] drops warnings and has the default budget; the
/// composition root passes the CLI's stderr sink with [`Injection::new`].
#[derive(Clone)]
pub struct Injection {
    warnings: Arc<OncePerPath>,
    budget: HtmlBudget,
}

impl Injection {
    /// Reports warnings to `sink`, once per warning code and path, within the default
    /// [`HtmlBudget`].
    #[must_use]
    pub fn new(sink: Arc<dyn WarningSink>) -> Self {
        Self {
            warnings: Arc::new(OncePerPath::new(sink)),
            budget: HtmlBudget::default(),
        }
    }

    /// The same sink, within `budget` instead.
    #[must_use]
    pub fn with_budget(mut self, budget: HtmlBudget) -> Self {
        self.budget = budget;
        self
    }
}

impl Default for Injection {
    fn default() -> Self {
        Self::new(Arc::new(SilentWarnings))
    }
}

/// The request facts injection depends on.
pub(crate) struct HtmlRequest<'a> {
    pub(crate) method: &'a Method,
    pub(crate) headers: &'a HeaderMap,
    /// The raw request path, used only to name the page in a warning.
    pub(crate) raw_path: &'a str,
}

/// The response for an HTML file opened with `snapshot` read just before the open.
///
/// - **GET** asks the domain's [`decide`] with the request's `Sec-Fetch-Dest`. On `Inject` the
///   page (at most 8 MiB) is read and rewritten on a blocking thread with a [`ScriptTag`] of
///   `snapshot`, never a fresh state; `Content-Length` is the rewritten length and `ETag` and
///   `Last-Modified` are absent. Any other outcome sends the original bytes.
/// - **HEAD** answers exactly the headers the same GET would (so its `Content-Length` is the
///   rewritten length when a GET would be injected) with no body: it does the GET's work and
///   discards the body. It reports no warning.
/// - **Over 8 MiB**: streamed unchanged as plain `text/html` (its encoding is unchecked) and
///   warned once per path (`html-too-large`) when a navigation GET would have been injected.
/// - **Not UTF-8**, by its bytes or its `<meta>` declaration: sent unchanged as plain
///   `text/html` and warned once per path (`html-not-utf8`).
/// - **Client already present** (a saved copy of a served page): no second tag; the existing
///   tag's `data-instance` and `data-revision` are set to `snapshot`, so a stale copy cannot
///   make the client reload on every load.
/// - **CSP**: an injected page whose `<meta http-equiv="Content-Security-Policy">` would block
///   the client script or the reload connection is still injected, byte for byte as before, and
///   warned once per path (`csp-blocks-live-reload`); see [`super::csp`].
/// - **Rewrite refused**: the original bytes, labeled UTF-8, warned once per path
///   (`html-rewrite-failed`).
///
/// A page held in memory is charged to the [`HtmlBudget`] until its body is sent or
/// dropped, and never released before the blocking read and rewrite have ended, even when
/// this future is dropped first (pinned by
/// `should_keep_reservation_until_blocking_work_ends_when_request_dropped`); a request that
/// does not fit waits. The file is only ever read. A file that ends
/// before its open-time length answers 500, since no byte has been sent yet.
///
/// Pinned by the tests in `tests/inject.rs`, among them
/// `should_inject_snapshot_revision_when_document_get`,
/// `should_not_modify_when_sec_fetch_dest_empty`,
/// `should_set_content_length_of_rewritten_body_when_injected`,
/// `should_serve_unchanged_and_warn_once_when_html_over_limit`,
/// `should_not_inject_when_head` and `should_not_label_utf8_when_meta_names_shift_jis`.
pub(crate) async fn html_response(
    injection: &Injection,
    request: &HtmlRequest<'_>,
    file: Box<dyn ContentFile>,
    snapshot: SiteState,
) -> Response {
    let len = file.len();
    let get = *request.method == Method::GET;
    let fetch_dest = request
        .headers
        .get("sec-fetch-dest")
        .map(|value| value.to_str().map_or(FetchDest::Other, FetchDest::parse));
    // HEAD is asked as the GET it describes, so both carry the same headers.
    let input = InjectionInput {
        method: RequestMethod::Get,
        status_ok: true,
        media: Media::from_essence(html_content_type(false)),
        fetch_dest,
        size: len,
        utf8_ok: true,
    };
    let path = get.then(|| shown_path(request.raw_path)).flatten();

    if len > MAX_INJECTION_INPUT_BYTES {
        if let (InjectionDecision::Skip(reason), Some(path)) = (decide(&input), &path)
            && let Some(warning) = reason.warning()
        {
            injection
                .warnings
                .warn(warning.code(), path, &warning.to_string());
        }
        let body = if get {
            stream_body(file, len)
        } else {
            Body::empty()
        };
        return html_with_headers(body, false, len);
    }

    let Some(permit) = injection.budget.reserve(charge(len)).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let warnings = get.then(|| Arc::clone(&injection.warnings));
    // The permit moves into the blocking task and comes back with the page: if this future is
    // dropped (client gone, connection cap, drain) while the page is read or rewritten, the
    // blocking task still holds both copies, so the reservation must stay with it until it
    // ends (ADR 0046).
    let prepared = tokio::task::spawn_blocking(move || {
        let page = prepare(file, len, input, &snapshot)?;
        if let (Some(warnings), Some(path), Some((code, message))) = (warnings, path, &page.warning)
        {
            warnings.warn(code, &path, message);
        }
        Ok::<_, io::Error>((page, permit))
    })
    .await;
    let Ok(Ok((page, permit))) = prepared else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let sent = page.bytes.len() as u64;
    let body = if get {
        buffered_body(page.bytes, permit)
    } else {
        Body::empty()
    };
    html_with_headers(body, page.utf8, sent)
}

/// A page ready to send.
struct Prepared {
    bytes: Vec<u8>,
    utf8: bool,
    warning: Option<(&'static str, String)>,
}

/// Reads the page, checks its encoding, decides and rewrites. Blocking.
fn prepare(
    file: Box<dyn ContentFile>,
    len: u64,
    input: InjectionInput,
    snapshot: &SiteState,
) -> io::Result<Prepared> {
    let bytes = read_exactly(file, len)?;
    let encoding = classify(&bytes);
    let input = InjectionInput {
        utf8_ok: encoding.is_utf8(),
        ..input
    };
    let utf8 = input.utf8_ok;
    let (bytes, warning) = match decide(&input) {
        InjectionDecision::Inject => {
            match rewrite_with_csp(&bytes, &ScriptTag::from_state(snapshot)) {
                (Rewrite::Injected(rewritten), policies) => {
                    (rewritten, detect(&policies).map(csp_warning))
                }
                (Rewrite::AlreadyPresent(refreshed), policies) => {
                    (refreshed, detect(&policies).map(csp_warning))
                }
                (Rewrite::Failed(failure), _) => (
                    bytes,
                    Some((
                        REWRITE_FAILED,
                        format!("{failure}; served unchanged without live reload"),
                    )),
                ),
            }
        }
        InjectionDecision::Skip(reason) => {
            let warning = reason.warning().map(|warning| {
                let mut message = warning.to_string();
                if encoding == Encoding::DeclaredOther {
                    message.push_str(" (its <meta> charset names another encoding)");
                }
                (warning.code(), message)
            });
            (bytes, warning)
        }
    };
    Ok(Prepared {
        bytes,
        utf8,
        warning,
    })
}

fn csp_warning(warning: super::CspWarning) -> (&'static str, String) {
    (CSP_BLOCKS_LIVE_RELOAD, warning.to_string())
}

/// Exactly `len` bytes of `file`: never more if it grew, `UnexpectedEof` if it shrank.
fn read_exactly(file: Box<dyn ContentFile>, len: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or_default());
    file.take(len).read_to_end(&mut bytes)?;
    if (bytes.len() as u64) < len {
        return Err(ErrorKind::UnexpectedEof.into());
    }
    Ok(bytes)
}

/// The request path as warnings show it: decoded once, then re-encoded component by
/// component like a redirect `Location`, so no control character or query string reaches
/// stderr. `None` for a path that does not decode (never the case for a served file).
fn shown_path(raw_path: &str) -> Option<String> {
    let path = decode_request_path(raw_path).ok()?;
    let encoded = ChangedPath::new(&RelPath::new(path.components().to_vec()));
    let mut shown = encoded.url_path().to_owned();
    if path.has_trailing_slash() && !shown.ends_with('/') {
        shown.push('/');
    }
    Some(shown)
}

fn html_with_headers(body: Body, utf8: bool, len: u64) -> Response {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(html_content_type(utf8)),
    );
    headers.insert(CONTENT_LENGTH, HeaderValue::from(len));
    // Nothing sets validators today; removing them keeps that true for rewritten bytes.
    headers.remove(ETAG);
    headers.remove(LAST_MODIFIED);
    response
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::task::{Context, Waker};

    use preview_domain::{InstanceId, Revision};

    use super::*;
    use crate::inject::budget::HtmlBudget;

    /// A page whose first read reports `entered` and then blocks until `release` is sent.
    struct GatedPage {
        bytes: io::Cursor<Vec<u8>>,
        entered: Option<Sender<()>>,
        release: Receiver<()>,
    }

    impl Read for GatedPage {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).expect("test is waiting");
                self.release.recv().expect("test releases the read");
            }
            self.bytes.read(buf)
        }
    }

    impl ContentFile for GatedPage {
        fn len(&self) -> u64 {
            self.bytes.get_ref().len() as u64
        }
    }

    #[tokio::test]
    async fn should_keep_reservation_until_blocking_work_ends_when_request_dropped() {
        // given
        let budget = HtmlBudget::new(1024 * 1024);
        let full = budget.available_bytes();
        let injection = Injection::default().with_budget(budget.clone());
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let page = GatedPage {
            bytes: io::Cursor::new(b"<html><body><p>x</p></body></html>".to_vec()),
            entered: Some(entered_tx),
            release: release_rx,
        };
        let headers = HeaderMap::new();
        let request = HtmlRequest {
            method: &Method::GET,
            headers: &headers,
            raw_path: "/",
        };
        let snapshot = SiteState::new(
            InstanceId::parse("startup-abc").expect("valid instance id"),
            Revision::from(1),
        );
        let mut response = Box::pin(html_response(
            &injection,
            &request,
            Box::new(page),
            snapshot,
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(response.as_mut().poll(&mut cx).is_pending());
        entered_rx.recv().expect("the blocking read started");

        // when
        drop(response);
        let while_blocked = budget.available_bytes();
        release_tx.send(()).expect("the blocking read waits");
        let after = budget.reserve(full).await;

        // then
        assert!(
            while_blocked < full,
            "the reservation must outlive a dropped request while the page is in memory"
        );
        assert!(
            after.is_some(),
            "the blocking task must release it when it ends"
        );
    }

    #[test]
    fn should_show_encoded_path_when_warning_names_page() {
        // given
        let raw = [
            "/",
            "/guide/",
            "/my%20dir/caf%C3%A9.html",
            "/a/b.html",
            "/esc%1B%5B2J.html",
        ];

        // when
        let shown = raw.map(shown_path);

        // then
        assert_eq!(
            shown,
            [
                Some("/".to_owned()),
                Some("/guide/".to_owned()),
                Some("/my%20dir/caf%C3%A9.html".to_owned()),
                Some("/a/b.html".to_owned()),
                // A control character never decodes, so it can never reach a terminal.
                None,
            ]
        );
    }
}
