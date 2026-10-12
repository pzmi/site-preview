//! The Host authority gate (plan.md §5.4 bullets 1–3, ADR 0021, ADR 0037).
//!
//! Every request, internal routes included, passes [`HostPolicy::check`] before any route or
//! the method gate sees it; a refusal is `421 Misdirected Request` with the fixed body
//! `Host not allowed`. Nothing from the header reaches the response, and no CORS header is
//! ever added.
//!
//! The accepted [`Authority`] is stored in the request's extensions, so a handler that needs
//! it (the SSE Origin check) reads exactly what this gate accepted instead of parsing the
//! header a second time.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::HOST;
use axum::http::uri::Scheme;
use axum::middleware::Next;
use axum::response::Response;
use preview_domain::{Authority, HostDecision, HostPolicy, HostReject};

use crate::errors::plain;

/// Lets a request through only when its Host authority is one the listener answers for.
///
/// The adapter decides only what the domain cannot see:
/// - more than one Host header is refused before the policy runs
///   (`should_reject_with_421_when_host_header_is_duplicated`);
/// - a value that is not visible ASCII is refused as malformed, never treated as absent
///   (`should_reject_with_421_when_host_is_not_utf8`);
/// - an absolute-form request target (`GET http://a/ HTTP/1.1`) must name the same accepted
///   authority as the Host header, since RFC 9112 §3.2.2 lets a server route by either
///   (`should_reject_with_421_when_absolute_target_disagrees_with_host`), and its scheme
///   must be `http`, the only one this listener speaks: RFC 9110 §15.5.20 makes 421 the
///   answer for a scheme and authority the server will not produce a response for
///   (`should_reject_with_421_when_absolute_target_scheme_is_not_http`).
pub(crate) async fn host_gate(
    State(policy): State<HostPolicy>,
    mut request: Request,
    next: Next,
) -> Response {
    match authorize(policy, &request) {
        Ok(authority) => {
            request.extensions_mut().insert(authority);
            next.run(request).await
        }
        Err(_) => plain(StatusCode::MISDIRECTED_REQUEST),
    }
}

fn authorize(policy: HostPolicy, request: &Request) -> Result<Authority, HostReject> {
    let mut values = request.headers().get_all(HOST).iter();
    let raw = match (values.next(), values.next()) {
        (None, _) => None,
        (Some(value), None) => Some(value.to_str().map_err(|_| HostReject::Malformed)?),
        (Some(_), Some(_)) => return Err(HostReject::Malformed),
    };
    let accepted = decide(policy, raw)?;
    if request
        .uri()
        .scheme()
        .is_some_and(|scheme| *scheme != Scheme::HTTP)
    {
        return Err(HostReject::AddressNotAllowed);
    }
    if let Some(target) = request.uri().authority()
        && decide(policy, Some(target.as_str()))? != accepted
    {
        return Err(HostReject::AddressNotAllowed);
    }
    Ok(accepted)
}

fn decide(policy: HostPolicy, raw: Option<&str>) -> Result<Authority, HostReject> {
    match policy.check(raw) {
        HostDecision::Allow(authority) => Ok(authority),
        HostDecision::Reject(reason) => Err(reason),
    }
}
