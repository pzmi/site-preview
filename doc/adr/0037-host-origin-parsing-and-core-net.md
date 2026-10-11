# 0037. Host and Origin parsing details; `core::net` value types in the domain

Date: 2026-10-11

## Status

Proposed (planner default — owner may override)

## Context

ADR 0021 says which Host authorities are allowed but not how a header is parsed, and
plan.md §5.4 asks the SSE endpoint to reject a "conflicting" Origin without defining one.
ADR 0012 puts both policies in `preview-domain`, whose purity rule forbids `std::net`
because it holds sockets. Hand-parsing IPv6 in a security boundary invites bugs.

## Decision

- The domain uses the address value types of `core::net` (`IpAddr`, `Ipv4Addr`,
  `Ipv6Addr`), which parse and compare and own no socket; `std::net` stays forbidden. The
  adapter passes its `std::net::IpAddr`, the same type.
- Host parsing is strict: printable ASCII; no userinfo; IPv6 only in brackets and without
  a zone id; IPv4 only in canonical dotted-quad form (`127.1`, octal, hex, padded and
  trailing-dot forms are names, so rejected); a canonical decimal port, 80 when absent.
- `localhost` matches without regard to ASCII case and is allowed whenever the listener
  is loopback, `--allow-remote` or not: only this machine resolves it, so it is no
  rebinding vector. `localhost.` and `*.localhost` are names and rejected.
- A Host of `0.0.0.0` or `::` is always rejected: a wildcard is never a destination.
  A wildcard listener without `--allow-remote` therefore answers for nothing (fail closed).
- Numeric addresses compare exactly; `[::ffff:127.0.0.1]` is not `127.0.0.1`.
- SSE: `Sec-Fetch-Site: cross-site` is refused; an Origin must be `http://` plus the
  authority the Host check accepted, port included (`localhost` and `127.0.0.1` differ);
  `null`, other schemes and anything with a path, query or fragment are refused. Neither
  header present is allowed.

## Consequences

- HTTP-05 maps a Host refusal to 421, HTTP-11 an Origin refusal to 403; both pass raw
  header values and never pre-normalize them.
- A user who types a non-canonical address or a LAN host name gets 421, by design.

## Amendments

- **2026-10-11 (APP-04 checkpoint review), a known limitation, not a change of decision.**
  Browsers send `Sec-Fetch-*` only to potentially trustworthy origins (HTTPS, `localhost`,
  loopback addresses). A page loaded from `http://<LAN address>` in `--allow-remote` mode
  gets no Fetch Metadata, so HTML injection runs on its fallback classification as the normal
  case there, and a cross-site `no-cors` request (an `<img>` or `<script>` on another site
  pointing at `/__site_preview/events`) carries neither `Sec-Fetch-Site` nor `Origin` and
  is accepted after the Host check. Such requests can hold SSE slots up to the per-server
  limit (plan.md §7.4), so a hostile page on the same network can deny live reload, though it
  cannot read the stream. plan.md §15 lists it; `--allow-remote` is already documented as
  unsafe on untrusted networks.
