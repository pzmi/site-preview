# 0044. SSE admission order, `HEAD` on the event stream, and the absolute-form scheme

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

## Context

plan.md §5.4 asks the event stream to refuse a conflicting `Origin` and an explicit
cross-site `Sec-Fetch-Site`, and §7.4 limits it to 64 simultaneous connections with the
permit reclaimed on disconnect or shutdown. ADR 0037 fixes what the domain's
`OriginPolicy` decides but leaves to the adapter what only it sees: header repetition and
bytes that are not text. Neither document says whether `HEAD` takes a connection slot, nor
in which order the Origin check and the limit run. Separately, the M1 review found that
the Host gate compared an absolute-form request target's authority (RFC 9112 §3.2.2) but
ignored its scheme, so `GET https://127.0.0.1:P/` and `GET ftp://127.0.0.1:P/` were served.

## Decision

- The Host gate stores the `Authority` it accepted in the request's extensions; the event
  stream's Origin check compares against that value and never parses `Host` again.
- An absolute-form request target whose scheme is not `http` (compared without regard to
  ASCII case) is refused by the Host gate with the same `421 Host not allowed` as an
  authority mismatch: RFC 9110 §15.5.20 defines 421 for a scheme and authority combination
  the server will not answer for.
- `GET` and `HEAD /__site_preview/events` run the Origin check first, before anything is
  subscribed or reserved. `Origin` and `Sec-Fetch-Site` go to `OriginPolicy::check` raw;
  either header repeated, or with a value that is not visible ASCII, is refused without
  consulting the policy. Every refusal is `403` with the fixed body `403 Forbidden`.
- `GET` then takes one permit with `try_acquire_owned` from a per-server semaphore
  (`DEFAULT_MAX_STREAMS = 64`, an `AppState` field so tests can lower it). None left is
  `503` with `Retry-After: 2` and the fixed body `503 Service Unavailable`. The permit is
  owned by the stream's feed, so dropping the response body, the stream ending on shutdown
  and the reload state closing all return it.
- `HEAD` answers the stream's headers with an empty body and neither subscribes nor takes a
  permit: no stream follows it, so it costs no slot and is answered even at the limit.
- Every response keeps `no-store` and `nosniff` from the outer layer; no CORS header is
  added to any of them.

## Consequences

- HTTP-11 implements this in `preview-adapter-http` (`sse_guard.rs`, `sse.rs`, `host.rs`);
  composition passes `StreamLimit::new(DEFAULT_MAX_STREAMS)`.
- ADR 0037's amendment still holds: a cross-site `no-cors` request to a LAN address carries
  no `Origin` and no `Sec-Fetch-Site`, passes the check and can hold slots until the limit;
  the limit bounds that cost, it does not prevent it.
- DOC-03 documents these rules with the tests that pin them.
