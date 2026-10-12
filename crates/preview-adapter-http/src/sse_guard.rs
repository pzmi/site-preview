//! Admission to the live-reload event stream (plan.md §5.4 bullets 4–5, §7.4; ADR 0037,
//! ADR 0044).
//!
//! Two checks run, in this order, before a connection subscribes to anything:
//! 1. the browser headers, through [`OriginPolicy::check`] against the authority the Host
//!    gate accepted; a refusal is `403` with a fixed body;
//! 2. one permit from the per-server [`StreamLimit`]; none left is `503` with
//!    `Retry-After: 2`.
//!
//! Neither refusal carries a CORS header or anything from the request.

use std::sync::Arc;

use axum::http::header::{ORIGIN, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use preview_domain::{Authority, OriginDecision, OriginPolicy, OriginReject};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::errors::plain;

/// How many event streams one server keeps open at once by default (plan.md §7.4).
pub const DEFAULT_MAX_STREAMS: usize = 64;

const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

/// The open-stream budget of one server. Clones share the same budget.
#[derive(Debug, Clone)]
pub struct StreamLimit {
    permits: Arc<Semaphore>,
}

impl StreamLimit {
    /// A budget of `max` simultaneous streams; composition passes [`DEFAULT_MAX_STREAMS`].
    #[must_use]
    pub fn new(max: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max)),
        }
    }

    /// One stream's permit, returned to the budget when it is dropped; `None` when every
    /// permit is held (`should_return_503_when_limit_reached`).
    pub(crate) fn acquire(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.permits).try_acquire_owned().ok()
    }
}

/// The answer when no permit is left: `503` with `Retry-After: 2`. A browser's `EventSource`
/// does not retry a non-200 answer (it fails the connection and ignores `Retry-After`); the
/// injected client re-creates it with a bounded backoff (ADR 0044, amendment).
pub(crate) fn busy() -> Response {
    let mut response = plain(StatusCode::SERVICE_UNAVAILABLE);
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("2"));
    response
}

/// The answer to a refused Origin check: `403` with a fixed body.
pub(crate) fn forbidden() -> Response {
    plain(StatusCode::FORBIDDEN)
}

/// Whether the browser headers let the request through.
///
/// The raw header values go to the domain, never normalized here. What only the adapter
/// sees is refused before the policy runs: a header sent more than once, or a value that is
/// not visible ASCII (`should_reject_with_403_when_origin_header_is_ambiguous`).
pub(crate) fn admit(headers: &HeaderMap, host: &Authority) -> Result<(), OriginReject> {
    let origin = single(headers, &ORIGIN)?;
    let site = single(headers, &SEC_FETCH_SITE)?;
    match OriginPolicy::check(origin, site, host) {
        OriginDecision::Allow => Ok(()),
        OriginDecision::Reject(reason) => Err(reason),
    }
}

/// The header's one value, `None` when absent.
fn single<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Result<Option<&'a str>, OriginReject> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => Ok(None),
        (Some(value), None) => value
            .to_str()
            .map(Some)
            .map_err(|_| OriginReject::Malformed),
        (Some(_), Some(_)) => Err(OriginReject::Malformed),
    }
}
