//! Fuzz target for the Host authority check `HostPolicy::check` (plan.md §5.4, ADR 0021,
//! ADR 0037, SEC-05).
//!
//! Input layout (so the committed seeds stay readable):
//!
//! - byte 0, bits 0 to 2: the bound address, from [`BOUND_ADDRESSES`]; bit 3: `--allow-remote`;
//!   bit 4: the request has no Host header;
//! - bytes 1 and 2: the bound port, big-endian;
//! - the rest: the Host header value, then optionally `0xFF` and an `Origin` value, then
//!   optionally `0xFF` and a `Sec-Fetch-Site` value (each made UTF-8 lossily).
//!
//! The check must never panic, and:
//!
//! 1. any `Allow` has a port equal to the bound port;
//! 2. an allowed host is one the listener answers for: `localhost` only on a loopback
//!    listener; a numeric address never unspecified, and either the bound address or any
//!    address on a wildcard listener with `--allow-remote`;
//! 3. the decision is deterministic;
//! 4. the SSE Origin check never panics on an accepted authority, and allows the origin a
//!    browser would send for that very authority (`http://` plus the accepted Host value).

#![no_main]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use libfuzzer_sys::fuzz_target;
use preview_domain::{
    AuthorityHost, BoundAddr, HostDecision, HostPolicy, OriginDecision, OriginPolicy,
};

/// Loopback, wildcard, LAN, IPv4-mapped and link-local listeners.
const BOUND_ADDRESSES: [IpAddr; 8] = [
    IpAddr::V4(Ipv4Addr::LOCALHOST),
    IpAddr::V6(Ipv6Addr::LOCALHOST),
    IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
    IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 1)),
    IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
    IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)),
];

const SEPARATOR: u8 = 0xFF;

fuzz_target!(|data: &[u8]| {
    let [flags, port_high, port_low, rest @ ..] = data else {
        return;
    };
    let bound = BoundAddr::new(
        BOUND_ADDRESSES[usize::from(flags & 0b111)],
        u16::from_be_bytes([*port_high, *port_low]),
    );
    let allow_remote = flags & 0b1000 != 0;
    let mut parts = rest.split(|byte| *byte == SEPARATOR);
    let host = String::from_utf8_lossy(parts.next().unwrap_or_default());
    let origin = parts.next().map(String::from_utf8_lossy);
    let sec_fetch_site = parts.next().map(String::from_utf8_lossy);
    let header = (flags & 0b1_0000 == 0).then_some(host.as_ref());

    let policy = HostPolicy::new(bound, allow_remote);
    let decision = policy.check(header);
    assert_eq!(
        decision,
        policy.check(header),
        "the decision is not deterministic"
    );
    let HostDecision::Allow(authority) = decision else {
        return;
    };

    assert_eq!(
        authority.port(),
        bound.port(),
        "allowed a port other than the bound one"
    );
    match authority.host() {
        AuthorityHost::Localhost => {
            assert!(
                bound.is_loopback(),
                "allowed localhost on a non-loopback listener"
            );
        }
        AuthorityHost::Ip(ip) => {
            assert!(
                !ip.to_canonical().is_unspecified(),
                "allowed an unspecified address"
            );
            assert!(
                ip == bound.ip() || (allow_remote && bound.is_wildcard()),
                "allowed {ip} on a listener bound to {}",
                bound.ip()
            );
        }
    }

    let _ = OriginPolicy::check(origin.as_deref(), sec_fetch_site.as_deref(), &authority);
    let echoed = format!("http://{host}");
    assert_eq!(
        OriginPolicy::check(Some(&echoed), None, &authority),
        OriginDecision::Allow,
        "refused the origin of the accepted authority {host:?}"
    );
});
